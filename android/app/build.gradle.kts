plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
    id("com.google.gms.google-services")
}

android {
    namespace = "com.stablechannels.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.stablechannels.app"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "1.0.0"
    }

    buildTypes {
        debug {
            buildConfigField("String", "NETWORK", "\"signet\"")
            buildConfigField("String", "ESPLORA_URL", "\"https://mutinynet.com/api\"")
            buildConfigField("String", "FALLBACK_ESPLORA_URL", "\"https://mutinynet.com/api\"")
        }
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            buildConfigField("String", "NETWORK", "\"bitcoin\"")
            buildConfigField("String", "ESPLORA_URL", "\"https://blockstream.info/api\"")
            buildConfigField("String", "FALLBACK_ESPLORA_URL", "\"https://mempool.space/api\"")
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
    // LDK Node
    implementation("org.lightningdevkit:ldk-node-android:0.7.0")

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
}
