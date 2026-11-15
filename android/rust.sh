#!/bin/sh
# Builds samu-mobile for the phone and writes its Kotlin bindings, both
# under the directory given (ADR 0028 §1). Called by Gradle before every
# build, so Rust and Kotlin cannot drift apart. As catastr's android/rust.sh.
set -eu

out=$1
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    ANDROID_NDK_HOME=$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)
fi
export ANDROID_HOME ANDROID_NDK_HOME

cd "$root"
target=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
cargo ndk -t arm64-v8a -P 26 -o "$out/jniLibs" build -p samu-mobile --release
cargo run -q -p samu-mobile --features bindgen --bin uniffi-bindgen -- \
    generate --library "$target/aarch64-linux-android/release/libsamu_mobile.so" \
    --language kotlin --no-format \
    --out-dir "$out/kotlin"
