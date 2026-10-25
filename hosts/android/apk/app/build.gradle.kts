plugins {
    id("com.android.application")
}

android {
    namespace = "dev.uifw.host"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.uifw.host"
        // libvulkan.so requires 24.
        minSdk = 24
        targetSdk = 35
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }
}


