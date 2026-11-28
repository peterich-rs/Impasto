plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

kotlin { jvmToolchain(17) }

android {
    namespace = "dev.impasto.plugins.notifications"
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
    compileOnly(project(":impasto-host"))
    implementation("androidx.core:core:1.15.0")
    implementation("androidx.activity:activity:1.10.1")

}
