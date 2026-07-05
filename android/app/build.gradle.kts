plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Only apply google-services plugin if config file exists (gitignored, not in repo)
if (file("../google-services.json").exists() || file("google-services.json").exists()) {
    apply(plugin = "com.google.gms.google-services")
}

android {
    namespace = "com.stablechannels.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.stablechannels.app"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.9"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
}

dependencies {
    // LDK Node — toneloc/ldk-node fork (splice + close fixes, LDK 0.3), version 0.7.5.
    // Pulled from the fork's GitHub release via the Ivy repo in settings.gradle.kts,
    // mirroring how iOS pulls the xcframework through SwiftPM.
    implementation("org.lightningdevkit:ldk-node-android:0.7.5@aar")
    // The @aar dependency carries no POM, so its transitive deps are declared explicitly:
    implementation("net.java.dev.jna:jna:5.12.0@aar") // loads libldk_node.so via JNA
    implementation("org.slf4j:slf4j-api:1.7.30")
    implementation("androidx.appcompat:appcompat:1.4.0")

    // Compose BOM
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")

    // Navigation
    implementation("androidx.navigation:navigation-compose:2.8.5")

    // Lifecycle / ViewModel
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")

    // Activity
    implementation("androidx.activity:activity-compose:1.9.3")

    // Core
    implementation("androidx.core:core-ktx:1.15.0")

    // Kotlin Serialization
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")

    // Coroutines
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    // QR code generation
    implementation("com.google.zxing:core:3.5.3")

    // QR code scanning
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")

    // OkHttp for network calls
    implementation("com.squareup.okhttp3:okhttp:4.12.0")

    // Charts
    implementation("io.github.bytebeats:compose-charts:0.2.1")

    // Firebase
    implementation(platform("com.google.firebase:firebase-bom:33.7.0"))
    implementation("com.google.firebase:firebase-messaging-ktx")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-play-services:1.9.0")

    // Biometric authentication
    implementation("androidx.biometric:biometric:1.2.0-alpha05")

    // CameraX
    implementation("androidx.camera:camera-camera2:1.4.1")
    implementation("androidx.camera:camera-lifecycle:1.4.1")
    implementation("androidx.camera:camera-view:1.4.1")

    // ML Kit Barcode Scanning
    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    // Unit tests
    testImplementation("junit:junit:4.13.2")
}
