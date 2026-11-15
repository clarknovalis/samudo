//! uniffi's generator, built from the same version as the library, so the
//! Kotlin it writes matches the scaffolding it reads.

fn main() {
    uniffi::uniffi_bindgen_main();
}
