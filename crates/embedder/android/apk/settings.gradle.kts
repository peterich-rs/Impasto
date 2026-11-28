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

rootProject.name = "android"
include(":app")
include(":impasto-host")
project(":impasto-host").projectDir = file("../host")

listOf("clipboard", "connectivity", "file-system", "haptics", "linking", "permissions", "share", "system-info")
    .forEach { plugin ->
        val path = ":impasto_${plugin.replace('-', '_')}"
        include(path)
        project(path).projectDir = file("../../../../packages/$plugin/android")
    }
