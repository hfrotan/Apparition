//! Apparition Companion for Windows and Linux (and macOS without the menu-bar app): the same
//! relay and info protocol as Companion/Relay.swift, as one small command-line program.
//!
//!     apparition-companion                 relay on port 49160, advertised over mDNS
//!     apparition-companion anisette up     run the self-hosted anisette server with Docker
//!     apparition-companion anisette down   stop it (its data volume stays)
//!
//! The headset resets connections it makes to its own RemotePairing service, so Apparition asks
//! a companion to connect back to it:
//!
//!     Apparition -> companion:49160   "APPARITION-RELAY 1 <port>\n"
//!     companion -> <Apparition's own address>:<port>, then "OK\n" and bytes both ways
//!
//! It only ever connects back to the address a request came from, so it can't be used to
//! reach anything else, and what it carries is end-to-end encrypted between Apparition and the
//! headset. `APPARITION-INFO 1\n` gets `INFO {json}\n` back: what else this companion offers
//! (an anisette server's port, when one answers on this machine).

use std::net::{IpAddr, SocketAddr};
use std::process::ExitCode;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const RELAY_PORT: u16 = 49160;
const ANISETTE_PORT: u16 = 6969;
const SERVICE_TYPE: &str = "_apparition-companion._tcp.local.";
const COMPOSE: &str = include_str!("../../selfhost/anisette/compose.yaml");

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => relay(RELAY_PORT, true).await,
        ["--port", port] | ["--port", port, "--no-mdns"] | ["--no-mdns", "--port", port] => match port.parse() {
            Ok(port) => relay(port, !args.contains(&"--no-mdns")).await,
            Err(_) => usage(),
        },
        ["--no-mdns"] => relay(RELAY_PORT, false).await,
        ["anisette", action @ ("up" | "down")] => anisette(action).await,
        ["anisette", "compose"] => {
            print!("{COMPOSE}");
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: apparition-companion [--port N] [--no-mdns]\n       apparition-companion anisette up|down|compose"
    );
    ExitCode::from(2)
}

async fn relay(port: u16, advertise: bool) -> ExitCode {
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("can't listen on {port}: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("Apparition Companion relay listening on port {port}.");
    println!("In Apparition, use \"Find companion\" on this network, or set the relay to this machine's address, port {port}.");

    let mdns = if advertise { Mdns::start(port) } else { None };
    // Keep the advertised anisette port current.
    let mdns_task = mdns.map(|mdns| {
        tokio::spawn(async move {
            let mut published: Option<Option<u16>> = None;
            loop {
                let now = anisette_port().await;
                if published != Some(now) {
                    mdns.publish(now);
                    published = Some(now);
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        })
    });

    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((client, peer)) => { tokio::spawn(serve(client, peer)); }
                Err(error) => eprintln!("accept failed: {error}"),
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    if let Some(task) = mdns_task {
        task.abort();
    }
    ExitCode::SUCCESS
}

async fn serve(mut client: TcpStream, peer: SocketAddr) {
    let _ = client.set_nodelay(true);
    // IPv4 peers on a dual-stack socket arrive as ::ffff:a.b.c.d.
    let peer_ip = match peer.ip() {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        ip => ip,
    };
    let Some(request) = read_line(&mut client).await else {
        return;
    };
    if request == "APPARITION-INFO 1" {
        let mut info = serde_json::json!({ "version": 1 });
        if let Some(port) = anisette_port().await {
            info["anisette"] = port.into();
        }
        let _ = client.write_all(format!("INFO {info}\n").as_bytes()).await;
        return;
    }
    let parts: Vec<&str> = request.split(' ').collect();
    let port = match parts.as_slice() {
        ["APPARITION-RELAY", "1", port] => port.parse::<u16>().ok().filter(|port| *port >= 1024),
        _ => None,
    };
    let Some(port) = port else {
        let _ = client.write_all(b"ERR bad request\n").await;
        return;
    };
    let target = SocketAddr::new(peer_ip, port);
    let device = match tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(target)).await {
        Ok(Ok(device)) => device,
        Ok(Err(error)) => return refuse(&mut client, port, &error.to_string()).await,
        Err(_) => return refuse(&mut client, port, "timed out").await,
    };
    let _ = device.set_nodelay(true);
    if client.write_all(b"OK\n").await.is_err() {
        return;
    }
    println!("relay {peer_ip} :{port} open");
    let mut device = device;
    let outcome = tokio::io::copy_bidirectional(&mut client, &mut device).await;
    match outcome {
        Ok((to_device, from_device)) => println!("relay {peer_ip} :{port} closed ({to_device} B to, {from_device} B from)"),
        Err(error) => println!("relay {peer_ip} :{port} closed: {error}"),
    }
}

async fn refuse(client: &mut TcpStream, port: u16, reason: &str) {
    eprintln!("relay to :{port} failed: {reason}");
    let _ = client.write_all(format!("ERR can't connect to :{port}: {reason}\n").as_bytes()).await;
}

/// Reads up to the first newline, at most 256 bytes, within 10 seconds.
async fn read_line(stream: &mut TcpStream) -> Option<String> {
    let mut line = Vec::new();
    let read = async {
        loop {
            let byte = stream.read_u8().await.ok()?;
            if byte == b'\n' {
                return Some(());
            }
            line.push(byte);
            if line.len() > 256 {
                return None;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), read).await.ok()??;
    String::from_utf8(line).ok()
}

/// The anisette port when a v3 server answers on this machine (GET / returns the headers).
async fn anisette_port() -> Option<u16> {
    let probe = async {
        let mut stream = TcpStream::connect(("127.0.0.1", ANISETTE_PORT)).await.ok()?;
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.ok()?;
        String::from_utf8_lossy(&reply).contains("X-Apple-I-MD").then_some(ANISETTE_PORT)
    };
    tokio::time::timeout(Duration::from_secs(3), probe).await.ok().flatten()
}

struct Mdns {
    daemon: mdns_sd::ServiceDaemon,
    name: String,
    port: u16,
}

impl Mdns {
    fn start(port: u16) -> Option<Self> {
        match mdns_sd::ServiceDaemon::new() {
            Ok(daemon) => Some(Self { daemon, name: machine_name(), port }),
            Err(error) => {
                eprintln!("mDNS unavailable, Apparition won't find this companion on its own: {error}");
                None
            }
        }
    }

    /// Registers (or re-registers, which updates the TXT record) the service.
    fn publish(&self, anisette: Option<u16>) {
        let mut txt = vec![("v".to_string(), "1".to_string())];
        if let Some(port) = anisette {
            txt.push(("anisette".to_string(), port.to_string()));
        }
        let host = format!("{}.local.", self.name.replace(' ', "-"));
        let info = mdns_sd::ServiceInfo::new(SERVICE_TYPE, &self.name, &host, "", self.port, &txt[..])
            .map(|info| info.enable_addr_auto());
        match info.and_then(|info| self.daemon.register(info)) {
            Ok(()) => println!(
                "advertising {} over mDNS{}",
                self.name,
                anisette.map(|port| format!(", with anisette on port {port}")).unwrap_or_default()
            ),
            Err(error) => eprintln!("mDNS register failed: {error}"),
        }
    }
}

fn machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .or_else(|| {
            let output = std::process::Command::new("hostname").arg("-s").output().ok()?;
            String::from_utf8(output.stdout).ok()
        })
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "apparition-companion".to_string())
}

/// Runs the bundled compose file (selfhost/anisette/compose.yaml) with the user's Docker.
async fn anisette(action: &str) -> ExitCode {
    let file = std::env::temp_dir().join("apparition-anisette-compose.yaml");
    if let Err(error) = std::fs::write(&file, COMPOSE) {
        eprintln!("can't write {}: {error}", file.display());
        return ExitCode::FAILURE;
    }
    let mut command = tokio::process::Command::new("docker");
    command.arg("compose").arg("-f").arg(&file);
    if action == "up" {
        command.args(["up", "-d"]);
    } else {
        command.arg("down");
    }
    match command.status().await {
        Ok(status) if status.success() => {
            if action == "up" {
                println!("Anisette starting on port {ANISETTE_PORT}. In Apparition, \"Find companion\" offers it once it answers.");
            }
            ExitCode::SUCCESS
        }
        Ok(status) => {
            eprintln!("docker compose exited with {status}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("can't run docker (Docker Desktop, or Docker Engine with the compose plugin): {error}");
            ExitCode::FAILURE
        }
    }
}
