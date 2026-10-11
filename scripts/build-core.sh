#!/usr/bin/env bash
# Builds the Rust core (core/: isideload + jkcoxson/idevice's C FFI) as one static
# library for iOS devices and stages it with its headers under build/core-out.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# Must match the idevice rev in core/Cargo.toml: the header comes from this checkout.
commit="${IDEVICE_COMMIT:-3854a5df4a5a6dee71ffce4d8befc2ea356a8065}"
src="$root/build/idevice"
out="$root/build/core-out"
sdk="$(xcrun --sdk iphoneos --show-sdk-path)"

if [[ ! -d "$src/.git" ]]; then
  git clone https://github.com/jkcoxson/idevice.git "$src"
fi
git -C "$src" fetch --quiet origin
git -C "$src" checkout --quiet "$commit"
# Generates idevice.h (cbindgen runs in the FFI crate's build script).
(cd "$src/ffi" && BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$sdk" IPHONEOS_DEPLOYMENT_TARGET=17.0 \
  cargo build --release --target aarch64-apple-ios)

(cd "$root/core" && BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$sdk" IPHONEOS_DEPLOYMENT_TARGET=17.0 \
  cargo build --release --target aarch64-apple-ios)

mkdir -p "$out/lib" "$out/include"
cp "$root/core/target/aarch64-apple-ios/release/libapparition_core.a" "$out/lib/"
cp "$src/ffi/idevice.h" "$src/ffi/plist.h" "$root/core/include/apparition.h" "$out/include/"
echo "Staged apparition-core (idevice $commit) in $out"
