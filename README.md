# Apparition

An on-device sideloader for Apple Vision Pro: install apps from AltStore-format
sources, sign them with your own (free) Apple ID, keep them refreshed, and
launch them with debugger-prepared JIT, all from the headset.

Early spike stage. `Probe/` is spike 1: can an app pair with, and open a
RemotePairing tunnel to, the device it runs on, with no computer involved?

Spike 5 (sources and refresh) works on the headset. Sources are AltStore/SideStore JSON,
or `github:owner/repo` for apps published only as release assets (UTM's visionOS build).
Installed apps keep their IPA in Documents/apps, so a refresh re-signs and reinstalls
without the network, and their data survives. With "Remember password" on, the password
stays in this device's Keychain, and a refresh can sign in with nobody present (Apple
skips 2FA for a device it already trusts). Background tasks ask to run this about every
12 hours: a short app-refresh task, and a processing task for refreshes that outlast its
~30 s (uploading UTM takes 40 to 150 s). They need the companion or LocalDevVPN, and their
firing is not verified yet (`scheduleBackgroundRefresh` is the test hook).

`Companion/` is Apparition Companion, an optional macOS menu-bar app. It replaces
LocalDevVPN: the headset resets connections it makes to its own RemotePairing service,
so Apparition asks the companion (TCP 49160) to connect back to it. The headset then sees
the connection coming from the Mac. The companion only ever connects back to the address
that asked, and it carries only ciphertext (pair-verify, then TLS-PSK) with no keys. With
the headset on Tailscale, which LocalDevVPN no longer occupies, a companion at home keeps
signing and JIT working from anywhere. Tested 2026-10-11 with LocalDevVPN off: tunnel,
refresh (109 s over the LAN, 150 s over Tailscale) and JIT launch. On the LAN, Apparition
finds the companion over Bonjour ("Find companion"); over Tailscale, type its address.

`Lab/` is a test app for JIT experiments, launched through the probe's `launchJIT`.
Its first experiment (2026-10-11, visionOS 27.2): bytes the debugger writes into the
app's own signed `__TEXT` keep running after it detaches, including code first run
only after the detach and a site rewritten to branch to a trampoline. Apparition serves
this through an experimental JIT26 command, `x16 = 2` (copy x2 bytes from x1 to x0).

## Anisette

Signing in to Apple needs an anisette-v3 server. Apparition ships none: enter the URL of
any v3 server, or self-host one with `docker compose -f selfhost/anisette/compose.yaml
up -d` (Dadoum's anisette-v3-server, pulled from its own registry and pinned by
digest), and enter `http://<host>:6969`. The server sees this device's anisette identity
and IP address, never the Apple ID or password, which go straight to Apple.

The companion can run the same compose file with your Docker (Docker Desktop, OrbStack
or Colima): Start in its window or menu. It then advertises the server, and Apparition
offers "Use its anisette server" next to the companion; nothing switches on its own.

## Build

    scripts/build-core.sh      # builds the Rust core (isideload + idevice) for visionOS and iOS
    scripts/build-core.sh visionos-sim   # adds the visionOS simulator (rustup target add aarch64-apple-visionos-sim)
    xcodegen                   # generates Apparition.xcodeproj from project.yml
    build-and-sign             # or open Apparition.xcodeproj and Run
    SCHEME_NAME=ApparitionLab TARGET_NAME=ApparitionLab BUILD_BUNDLE_ID=com.illixion.apparition.lab \
        build-and-sign --no-mcp    # the lab app
    xcodebuild -scheme ApparitionCompanion -allowProvisioningUpdates build   # the Mac companion

Run the companion from /Applications (or anywhere other users can read). mDNSResponder
reads its Info.plist to allow the Bonjour service, and can't inside a private home or
temp directory: the relay then still works, but Apparition won't find it on its own.

Licensed MIT.
