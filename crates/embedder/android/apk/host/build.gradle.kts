import org.gradle.api.DefaultTask
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.FileSystemOperations
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import javax.inject.Inject

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

kotlin { jvmToolchain(17) }

val app = project(":app")

android {
    namespace = "dev.impasto.host"
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

kotlin.sourceSets.getByName("main").kotlin.srcDir(app.layout.projectDirectory.dir("src/main/kotlin"))

dependencies {
    api("androidx.activity:activity:1.10.1")
    implementation("androidx.core:core:1.15.0")
    implementation("androidx.window:window-java:1.2.0")
}

androidComponents {
    onVariants { variant ->
        val mode = variant.name.replaceFirstChar { it.uppercase() }
        val cargo = "cargoImpasto$mode"
        val assets = "stageApkAssets$mode"
        val jni = tasks.register<StageHostFiles>("stageHostJni$mode") {
            dependsOn(":app:$cargo")
            input.from(app.layout.buildDirectory.dir("generated/jniLibs/$cargo"))
        }
        val resources = tasks.register<StageHostFiles>("stageHostAssets$mode") {
            dependsOn(":app:$cargo", ":app:$assets")
            input.from(
                app.layout.buildDirectory.dir("generated/assets/$cargo"),
                app.layout.buildDirectory.dir("generated/assets/$assets"),
                app.layout.projectDirectory.dir("src/main/app-assets"),
            )
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(jni, StageHostFiles::output)
        variant.sources.assets?.addGeneratedSourceDirectory(resources, StageHostFiles::output)
    }
}

abstract class StageHostFiles : DefaultTask() {
    @get:Inject
    abstract val files: FileSystemOperations

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val input: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val output: DirectoryProperty

    @TaskAction
    fun stage() {
        files.sync {
            from(input)
            into(output)
        }
    }
}
