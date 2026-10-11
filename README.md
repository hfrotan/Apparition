# Apparition

An on-device sideloader for Apple Vision Pro: install apps from AltStore-format
sources, sign them with your own (free) Apple ID, keep them refreshed, and
launch them with debugger-prepared JIT, all from the headset.

Early spike stage. `Probe/` is spike 1: can an app pair with, and open a
RemotePairing tunnel to, the device it runs on, with no computer involved?

`Lab/` is a test app for JIT experiments, launched through the probe's `launchJIT`.
Its first experiment (2026-10-11, visionOS 27.2): bytes the debugger writes into the
app's own signed `__TEXT` keep running after it detaches, including code first run
only after the detach and a site rewritten to branch to a trampoline. Apparition serves
this through an experimental JIT26 command, `x16 = 2` (copy x2 bytes from x1 to x0).

## Build

    scripts/build-core.sh      # builds the Rust core (isideload + idevice) for iOS
    xcodegen                   # generates Apparition.xcodeproj from project.yml
    build-and-sign             # or open Apparition.xcodeproj and Run
    SCHEME_NAME=ApparitionLab TARGET_NAME=ApparitionLab BUILD_BUNDLE_ID=com.illixion.apparition.lab \
        build-and-sign --no-mcp    # the lab app

Licensed MIT.
