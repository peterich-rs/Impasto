plugins {
    id("com.android.application")
}

android {
    namespace = "dev.uifw.host"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.uifw.host"
        // games-activity 4.4.0 requires 21. libvulkan.so requires 24.
        minSdk = 24
        targetSdk = 35
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
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

dependencies {
    implementation("androidx.games:games-activity:4.4.0")
    // GameActivity extends AppCompatActivity. The published pom does not declare it.
    implementation("androidx.appcompat:appcompat:1.7.0")
}

configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.jetbrains.kotlin" && requested.name.startsWith("kotlin-stdlib")) {
            useVersion("1.8.22")
            because("kotlin-stdlib 1.8 absorbs jdk7 and jdk8")
        }
    }
}
