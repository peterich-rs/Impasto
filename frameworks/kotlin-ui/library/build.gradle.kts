plugins {
    alias(libs.plugins.kotlin.multiplatform)
}

group = "dev.uifw"
version = "0.1.0"

kotlin {
    jvm {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
        }
    }
    sourceSets {
        commonMain.dependencies {}
        val jvmMain by getting {
            dependencies {
                api(libs.reactive)
            }
        }
        val jvmTest by getting {
            dependencies {
                implementation(kotlin("test"))
            }
        }
    }
}

tasks.named<Test>("jvmTest") {
    val counter = rootProject.projectDir.resolve("../../protocol/tests/fixtures/counter.bin")
    environment("UIFW_COUNTER_BYTES", counter.absolutePath)
}
