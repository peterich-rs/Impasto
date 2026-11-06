import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.services.BuildService
import org.gradle.api.services.BuildServiceParameters
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import java.io.File
import javax.inject.Inject

plugins {
    id("com.android.application")
}

// One Cargo process at a time. debug and release share target/.
abstract class CargoGate : BuildService<BuildServiceParameters.None>

val cargoGate = gradle.sharedServices.registerIfAbsent("impastoCargo", CargoGate::class.java) {
    maxParallelUsages.set(1)
}

val repoRoot: File = rootProject.projectDir.resolve("../../..").canonicalFile

android {
    namespace = "dev.impasto.host"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.impasto.host"
        // libvulkan.so requires 24. The Cargo linker uses this same API level.
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
            // Local install only. This is not a store key.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }

    // Staged by the tasks below. src/main/assets and src/main/jniLibs are not inputs.
    sourceSets.getByName("main") {
        assets.setSrcDirs(emptyList<String>())
        jniLibs.setSrcDirs(emptyList<String>())
    }
}

androidComponents {
    onVariants { variant ->
        val cargoRelease = when (variant.buildType) {
            "debug" -> false
            "release" -> true
            else -> throw GradleException("unsupported Android build type ${variant.buildType}")
        }
        val capital = variant.name.replaceFirstChar { it.uppercase() }
        val api = android.defaultConfig.minSdk ?: 24
        val sdk = android.sdkDirectory
        val envNdk = System.getenv("ANDROID_NDK_HOME") ?: System.getenv("ANDROID_NDK_ROOT")
        if (!cargoRelease) {
            variant.packaging.jniLibs.keepDebugSymbols.add("**/libimpasto.so")
        }
        val cargo = tasks.register<CargoImpasto>("cargoImpasto$capital") {
            usesService(cargoGate)
            release.set(cargoRelease)
            minSdk.set(api)
            sdkDir.set(sdk)
            if (!envNdk.isNullOrBlank()) {
                ndkOverride.set(File(envNdk))
            }
            repoPath.set(repoRoot.path)
            // Cargo fingerprints sources. Gradle must not, because a repo-wide file tree overlaps APK outputs.
            outputs.upToDateWhen { false }
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoImpasto::jniLibs)

        val assets = tasks.register<StageApkAssets>("stageApkAssets$capital") {
            quotes.set(repoRoot.resolve("apps/gallery-solid/dist/quotes.js"))
            font.set(repoRoot.resolve("crates/android/assets/fonts/Sans.ttf"))
            logo.set(repoRoot.resolve("crates/macos/assets/logo.png"))
        }
        variant.sources.assets?.addGeneratedSourceDirectory(assets, StageApkAssets::output)
    }
}

abstract class CargoImpasto : DefaultTask() {
    @get:Inject
    abstract val execOps: ExecOperations

    @get:Input
    abstract val release: Property<Boolean>

    @get:Input
    abstract val minSdk: Property<Int>

    @get:Internal
    abstract val sdkDir: DirectoryProperty

    @get:Internal
    abstract val ndkOverride: DirectoryProperty

    @get:Input
    abstract val repoPath: Property<String>

    @get:OutputDirectory
    abstract val jniLibs: DirectoryProperty

    @TaskAction
    fun build() {
        val ndk = resolveNdk()
        val bin = llvmBin(ndk)
        val api = minSdk.get()
        val linker = File(bin, "aarch64-linux-android$api-clang")
        if (!linker.isFile) {
            throw GradleException("missing Android linker ${linker.path}")
        }
        val root = File(repoPath.get())
        val command = mutableListOf(
            "cargo",
            "build",
            "--manifest-path",
            root.resolve("Cargo.toml").path,
            "--target",
            "aarch64-linux-android",
            "-p",
            "android",
        )
        if (release.get()) {
            command.add("--release")
        }
        execOps.exec {
            commandLine(command)
            workingDir(root)
            environment("ANDROID_NDK_HOME", ndk.path)
            environment(
                "PATH",
                bin.path + File.pathSeparator + System.getenv("PATH").orEmpty(),
            )
            environment("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", linker.path)
        }.assertNormalExitValue()

        val profile = if (release.get()) "release" else "debug"
        val built = root.resolve("target/aarch64-linux-android/$profile/libimpasto.so")
        if (!built.isFile) {
            throw GradleException("cargo did not write ${built.path}")
        }
        val impeller = root.resolve("third_party/impeller-sdk/android-arm64/lib/libimpeller.so")
        if (!impeller.isFile) {
            throw GradleException("missing ${impeller.path}; run crates/engine/fetch-impeller-sdk.sh")
        }
        val abi = jniLibs.get().asFile.resolve("arm64-v8a")
        abi.mkdirs()
        built.copyTo(abi.resolve("libimpasto.so"), overwrite = true)
        impeller.copyTo(abi.resolve("libimpeller.so"), overwrite = true)
    }

    private fun resolveNdk(): File {
        val override = ndkOverride.orNull?.asFile
        if (override != null && override.isDirectory) {
            return override
        }
        val latest = sdkDir.get().asFile.resolve("ndk").listFiles()
            ?.filter { it.isDirectory }
            ?.maxByOrNull { it.name }
        if (latest != null) {
            return latest
        }
        throw GradleException("ANDROID_NDK_HOME is not set")
    }

    private fun llvmBin(ndk: File): File {
        val prebuilt = File(ndk, "toolchains/llvm/prebuilt")
        val os = System.getProperty("os.name").lowercase()
        val arch = System.getProperty("os.arch").lowercase()
        val host = when {
            os.contains("mac") && (arch == "aarch64" || arch == "arm64") -> "darwin-arm64"
            os.contains("mac") -> "darwin-x86_64"
            os.contains("linux") && (arch == "aarch64" || arch == "arm64") -> "linux-aarch64"
            else -> "linux-x86_64"
        }
        val preferred = File(prebuilt, "$host/bin")
        if (preferred.isDirectory) {
            return preferred
        }
        val fallback = prebuilt.listFiles()?.firstNotNullOfOrNull { hostDir ->
            File(hostDir, "bin").takeIf { it.isDirectory }
        }
        return fallback ?: throw GradleException("no LLVM prebuilt under ${prebuilt.path}")
    }
}

abstract class StageApkAssets : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val quotes: RegularFileProperty

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val font: RegularFileProperty

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val logo: RegularFileProperty

    @get:OutputDirectory
    abstract val output: DirectoryProperty

    @TaskAction
    fun stage() {
        val script = quotes.get().asFile
        if (!script.isFile) {
            throw GradleException("missing ${script.path}; run just gallery-dist")
        }
        val root = output.get().asFile
        val fonts = root.resolve("fonts")
        val images = root.resolve("images")
        fonts.mkdirs()
        images.mkdirs()
        font.get().asFile.copyTo(fonts.resolve("Sans.ttf"), overwrite = true)
        logo.get().asFile.copyTo(images.resolve("logo.png"), overwrite = true)
        script.copyTo(root.resolve("quotes.js"), overwrite = true)
    }
}
