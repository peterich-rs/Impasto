plugins {
    alias(libs.plugins.kotlin.multiplatform)
}

kotlin {
    jvm {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
        }
    }
    sourceSets {
        commonMain.dependencies {
            implementation(project(":library"))
        }
    }
}

tasks.register<JavaExec>("runGallery") {
    group = "application"
    description = "Print the counter scene from the Kotlin DSL."
    val compilation = kotlin.jvm().compilations.getByName("main")
    classpath(compilation.output.classesDirs, compilation.runtimeDependencyFiles)
    mainClass.set("dev.uifw.gallery.MainKt")
    dependsOn(compilation.compileTaskProvider)
}
