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

// One product version (ADR 0118 §4): `version` under [workspace.package] in
// the workspace's Cargo.toml. The release tag is v<version>; versionCode is
// major·10000 + minor·100 + patch, so minor and patch stay below 100.
val productVersion: String = run {
    val cargo = rootProject.projectDir.resolve("../Cargo.toml").readText()
    val section = cargo.substringAfter("[workspace.package]", "").substringBefore("\n[")
    Regex("""(?m)^version\s*=\s*"([^"]+)"""").find(section)?.groupValues?.get(1)
        ?: error("no version under [workspace.package] in Cargo.toml")
}
val productVersionCode: Int = run {
    val parts = productVersion.substringBefore('-').substringBefore('+').split('.').map {
        it.toIntOrNull() ?: error("version $productVersion is not major.minor.patch")
    }
    require(parts.size == 3 && parts[1] < 100 && parts[2] < 100) {
        "version $productVersion: need major.minor.patch with minor and patch below 100"
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

// The release key stays with the maintainer (ADR 0118 §1-2): its path and
// alias as Gradle properties (~/.gradle/gradle.properties), the passwords
// from the environment. Without them assembleRelease builds unsigned, as
// on GitHub, and the APK is signed afterwards with apksigner;
// `-Psamudo.keystore=` builds unsigned on a machine that has the key.
val keystorePath = providers.gradleProperty("samudo.keystore").orNull?.takeIf { it.isNotBlank() }
val keyAliasName = providers.gradleProperty("samudo.keyAlias").orNull

android {
    namespace = "de.samudo.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "de.samudo.app"
        minSdk = 26
        targetSdk = 36
        versionCode = productVersionCode
        versionName = productVersion
    }

    signingConfigs {
        if (keystorePath != null) {
            create("release") {
                storeFile = file(keystorePath)
                keyAlias = keyAliasName ?: error("samudo.keystore is set, samudo.keyAlias is not")
                storePassword = System.getenv("SAMUDO_KEYSTORE_PASSWORD")
                    ?: error("samudo.keystore is set, SAMUDO_KEYSTORE_PASSWORD is not")
                keyPassword = System.getenv("SAMUDO_KEY_PASSWORD") ?: storePassword
            }
        }
    }

    buildTypes {
        // Beside the released app, with its own cache and account (ADR 0118 §3).
        debug {
            applicationIdSuffix = ".dev"
            versionNameSuffix = "-dev"
            resValue("string", "app_name", "samudo dev")
        }
        release {
            resValue("string", "app_name", "samudo")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    buildFeatures {
        compose = true
        resValues = true
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
    // The code scanner brings androidx.fragment 1.0.0, whose FragmentActivity
    // mishandles the activity-result requests (the notification permission);
    // the release lint stops on it. Google, Apache 2.0.
    implementation("androidx.fragment:fragment:1.8.9")
    // uniffi's Kotlin calls the library through JNA; the aar carries its
    // native part for Android.
    implementation("net.java.dev.jna:jna:5.18.1@aar")
}
