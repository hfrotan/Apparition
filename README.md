# Apparition

An on-device sideloader for Apple Vision Pro: install apps from AltStore-format
sources, sign them with your own (free) Apple ID, keep them refreshed, and
launch them with debugger-prepared JIT, all from the headset.

Early spike stage. `Probe/` is spike 1: can an app pair with, and open a
RemotePairing tunnel to, the device it runs on, with no computer involved?

## Build

    scripts/build-idevice.sh   # builds jkcoxson/idevice's C FFI for iOS
    xcodegen                   # generates Apparition.xcodeproj from project.yml
    build-and-sign             # or open Apparition.xcodeproj and Run

Licensed MIT.
