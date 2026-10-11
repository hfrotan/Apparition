# Apparition Companion for Windows and Linux

The same relay and info protocol as the macOS menu-bar companion (`../Companion`), as one
small command-line program with no dependencies beyond the OS.

    apparition-companion                 # relay on TCP 49160, advertised over mDNS
    apparition-companion --port 49161 --no-mdns
    apparition-companion anisette up     # self-hosted anisette with your Docker (pinned image)
    apparition-companion anisette down   # stop it; its data volume stays
    apparition-companion anisette compose   # print the compose file it runs
    apparition-companion --verbose | browse  # mDNS diagnostics

Apparition finds it with "Find companion" on the same network, or set the relay to
`<this machine>:49160` by hand (over Tailscale, use its Tailscale address).

**Windows firewall:** allow inbound TCP 49160 for the network you use, e.g. from an
elevated PowerShell:

    New-NetFirewallRule -DisplayName "Apparition Companion" -Direction Inbound -Protocol TCP -LocalPort 49160 -Action Allow -Profile Private

(or accept Windows' first-run prompt when starting it from a desktop session).

## Build

    cargo build --release                                   # this machine
    cargo build --release --target x86_64-pc-windows-gnu    # Windows x64, from macOS with
                                                            # brew install mingw-w64

The Windows build imports only system DLLs (UCRT, Winsock), so the .exe runs as is.

## Tested

2026-10-11:
- The macOS build relayed Apparition's tunnel (pair-verify and the TLS-PSK tunnel) from the
  headset over Tailscale.
- The Windows x64 build, on a bridged Windows 11 VM on the LAN, was found by the
  headset's "Find companion" over mDNS. It relayed the headset's tunnel; identify
  succeeded through it.
- On loopback it also answered INFO, refused bad and privileged-port requests, and
  reported connect-back failures.

On macOS, Local Network privacy blocks an unbundled binary's multicast, so it can't
advertise there: use the menu-bar app.

`apparition-companion --verbose` prints the mDNS daemon's events, and `apparition-companion
browse` lists the companions mDNS sees from that machine.
