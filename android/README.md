# Android-App

Kotlin/Compose über die Crate `samu-mobile` (ADR 0028), gebaut wie die App
von catastr (catastr ADR 0080).

**Benutzung:** [`docs/android.md`](../docs/android.md) (englisch). Hier steht
nur, wie sie gebaut wird.

## Einrichten

Wie für catastr, einmal je Rechner:

```sh
~/Android/Sdk/cmdline-tools/latest/bin/sdkmanager \
    platform-tools "platforms;android-37.0" "build-tools;36.1.0" "ndk;29.0.14206865"
rustup target add aarch64-linux-android
cargo install cargo-ndk --locked
echo "sdk.dir=$HOME/Android/Sdk" > android/local.properties
```

## Bauen und installieren

```sh
make android
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

Vor jedem Bau ruft Gradle `android/rust.sh`: es baut `samu-mobile` für
`arm64-v8a` und erzeugt daraus die Kotlin-Bindings unter `app/build/rust/`.
Beides wird nicht eingecheckt, damit Rust und Kotlin nicht auseinanderlaufen.

`android/` liegt außerhalb von `make check`; `samu-mobile` selbst baut und
testet `make check` auf dem Host mit, gegen ein echtes `samu-server`.

## Gegen einen Testserver auf dem Rechner

```sh
target/debug/samu-server --data /tmp/samu-test serve --listen 127.0.0.1:8492
adb reverse tcp:8492 tcp:8492
```

In der App dann als Server `http://127.0.0.1:8492`.
