// The same plugin versions as catastr's app (ADR 0028 §1), so both build
// with one SDK and one Gradle.
plugins {
    id("com.android.application") version "9.4.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.21" apply false
}
