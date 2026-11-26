import java.util.Properties

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "Runner"
include(":app", ":impasto-host")

val generated = file("../.impasto/android")
val properties = Properties()
generated.resolve("plugins.properties").reader().use { properties.load(it) }
val framework = file(properties.getProperty("framework"))
project(":impasto-host").projectDir = framework.resolve("crates/embedder/android/host")
apply(from = generated.resolve("settings-plugins.settings.gradle.kts"))
