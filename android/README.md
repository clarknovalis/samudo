# Android app

Kotlin and Compose over the crate `samu-mobile`
([ADR 0028](../docs/adr/0028-android-app.md)).

**Using it:** [`docs/android.md`](../docs/android.md). This page only says
how it is built.

## Setting up

Once per computer:

```sh
~/Android/Sdk/cmdline-tools/latest/bin/sdkmanager \
    platform-tools "platforms;android-37.0" "build-tools;36.1.0" "ndk;29.0.14206865"
rustup target add aarch64-linux-android
cargo install cargo-ndk --locked
echo "sdk.dir=$HOME/Android/Sdk" > android/local.properties
```

## Building and installing

```sh
make android
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

The debug build is called `de.samudo.app.dev` ("samudo dev") and runs
beside the released app, with a cache and an account of its own.

The version comes from `version` under `[workspace.package]` in
`Cargo.toml`; `versionCode` is major·10000 + minor·100 + patch.

## Release

```sh
./gradlew assembleRelease     # in android/
```

Without a key this gives `app-release-unsigned.apk`, to be signed with
`apksigner`. With a key Gradle signs by itself: path and alias as
properties in `~/.gradle/gradle.properties`, the passwords from the
environment.

```properties
samudo.keystore=/path/to/release.jks
samudo.keyAlias=samudo
```

```sh
SAMUDO_KEYSTORE_PASSWORD=… ./gradlew assembleRelease   # SAMUDO_KEY_PASSWORD if different
```

Before every build Gradle runs `android/rust.sh`: it builds `samu-mobile`
for `arm64-v8a` and generates the Kotlin bindings from it under
`app/build/rust/`. Neither is checked in, so that Rust and Kotlin cannot
drift apart.

`android/` is outside `make check`; `samu-mobile` itself is built and
tested by `make check` on the host, against a real `samu-server`.

## Against a test server on the computer

```sh
target/debug/samu-server --data /tmp/samu-test serve --listen 127.0.0.1:8492
adb reverse tcp:8492 tcp:8492
```

In the app, the server is then `http://127.0.0.1:8492`.
