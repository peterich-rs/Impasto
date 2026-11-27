import java.util.Properties

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
    plugins {
        id("com.android.library") version "8.7.3"
        id("org.jetbrains.kotlin.android") version "2.1.21"
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "impasto-share"
val local = Properties()
file("local.properties").takeIf { it.isFile }?.inputStream()?.use { local.load(it) }
val sdk = local.getProperty("impasto.sdk") ?: System.getenv("IMPASTO_ROOT") ?: "../../.."
include(":impasto-host")
project(":impasto-host").projectDir = file(sdk).resolve("crates/embedder/android/host")
