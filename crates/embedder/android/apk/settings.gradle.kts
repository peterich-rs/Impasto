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

include(":impasto_file_system")
project(":impasto_file_system").projectDir = file("../../../../packages/file-system/android")

include(":impasto_linking")
project(":impasto_linking").projectDir = file("../../../../packages/linking/android")

include(":impasto_permissions")
project(":impasto_permissions").projectDir = file("../../../../packages/permissions/android")
