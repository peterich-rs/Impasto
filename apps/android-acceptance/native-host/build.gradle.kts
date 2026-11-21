plugins {
    id("com.android.application") version "8.7.3"
    id("org.jetbrains.kotlin.android") version "2.1.21"
}

android {
    namespace = "dev.impasto.fieldnotes.embedded"
    compileSdk = 35
    defaultConfig {
        applicationId = "dev.impasto.fieldnotes.embedded"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin { jvmToolchain(17) }

dependencies {
    implementation(files("../build/android/host/build/outputs/aar/host-release.aar"))
    implementation("androidx.activity:activity:1.10.1")
    implementation("androidx.core:core:1.15.0")
    implementation("androidx.window:window-java:1.2.0")
    implementation("androidx.fragment:fragment:1.8.9")
}
