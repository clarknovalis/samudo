// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// The Android app (ADR 0028). The Rust side is built by rust.sh before every
// build: the library for the phone and the Kotlin bindings from it, so the
// two cannot drift apart. Needs the SDK, the NDK and cargo-ndk, see
// android/README.md.

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "de.samudo.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "de.samudo.app"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
    }

    buildFeatures {
        compose = true
    }

    sourceSets["main"].jniLibs.directories.add(layout.buildDirectory.dir("rust/jniLibs").get().asFile.path)
    sourceSets["main"].kotlin.directories.add(layout.buildDirectory.dir("rust/kotlin").get().asFile.path)
}

val rust by tasks.registering(Exec::class) {
    description = "Builds samu-mobile for the phone and generates its Kotlin bindings."
    workingDir = rootProject.projectDir
    commandLine("./rust.sh", layout.buildDirectory.dir("rust").get().asFile.path)
}

tasks.named("preBuild") {
    dependsOn(rust)
}

// The same libraries and versions as catastr's app.
dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.12.4")
    // The sync on opening, after a change and every quarter of an hour
    // (ADR 0028 §5). Google, Apache 2.0.
    implementation("androidx.work:work-runtime:2.12.0")
    // The widget "Heute" (ADR 0028 §9): Compose-like, lists and taps
    // without a RemoteViewsService. Google, Apache 2.0.
    implementation("androidx.glance:glance-appwidget:1.1.1")
    // Reading the QR code that pairs the phone (ADR 0031 §7): the code
    // scanner of the Play services, as catastr's app. It brings its own
    // camera screen, so no camera permission; ML Kit terms, a binary part,
    // needs the Play services on the phone.
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
    // uniffi's Kotlin calls the library through JNA; the aar carries its
    // native part for Android.
    implementation("net.java.dev.jna:jna:5.18.1@aar")
}
