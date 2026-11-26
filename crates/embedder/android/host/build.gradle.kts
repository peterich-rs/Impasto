plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

kotlin { jvmToolchain(17) }

android {
    namespace = "dev.impasto.host"
    compileSdk = 35
    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    api("androidx.activity:activity:1.10.1")
    api("androidx.fragment:fragment:1.8.9")
    implementation("androidx.core:core:1.15.0")
    implementation("androidx.window:window-java:1.2.0")
}
