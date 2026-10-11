# Apparition Companion for Windows and Linux

The same relay and info protocol as the macOS menu-bar companion (`../Companion`), as one
small command-line program with no dependencies beyond the OS.

    apparition-companion                 # relay on TCP 49160, advertised over mDNS
    apparition-companion --port 49161 --no-mdns
    apparition-companion anisette up     # self-hosted anisette with your Docker (pinned image)
    apparition-companion anisette down   # stop it; its data volume stays
    apparition-companion anisette compose   # print the compose file it runs

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

2026-10-11: relayed Apparition's tunnel (pair-verify and the TLS-PSK tunnel) from the headset
over Tailscale with the macOS build. The Windows x64 build answered INFO, refused bad
and privileged-port requests, relayed bytes and reported connect-back failures (on an
ARM64 Windows 11 VM, over loopback). Not yet tested: a Windows machine relaying for the
headset on the LAN, and its mDNS advertisement being seen by Apparition (on macOS, Local
Network privacy blocks an unbundled binary's multicast; use the menu-bar app there).
