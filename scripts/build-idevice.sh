#!/usr/bin/env bash
# Builds jkcoxson/idevice's C FFI as a static library for iOS devices and stages
# the library and headers under build/idevice-out for the Xcode project.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
commit="${IDEVICE_COMMIT:-3854a5df4a5a6dee71ffce4d8befc2ea356a8065}"
src="$root/build/idevice"
out="$root/build/idevice-out"

if [[ ! -d "$src/.git" ]]; then
  git clone https://github.com/jkcoxson/idevice.git "$src"
fi
git -C "$src" fetch --quiet origin
git -C "$src" checkout --quiet "$commit"

cd "$src/ffi"
BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$(xcrun --sdk iphoneos --show-sdk-path)" \
IPHONEOS_DEPLOYMENT_TARGET=17.0 \
  cargo build --release --target aarch64-apple-ios

mkdir -p "$out/lib" "$out/include"
cp "$src/target/aarch64-apple-ios/release/libidevice_ffi.a" "$out/lib/"
cp "$src/ffi/idevice.h" "$src/ffi/plist.h" "$out/include/"
echo "Staged idevice $commit in $out"
