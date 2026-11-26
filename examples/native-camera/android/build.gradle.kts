plugins {
    id("com.android.application") version "8.7.3" apply false
    id("com.android.library") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.1.21" apply false
}

val output = rootProject.file("../build/android/gradle")
subprojects { layout.buildDirectory.set(output.resolve(name)) }
