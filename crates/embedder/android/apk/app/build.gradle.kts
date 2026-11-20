import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.provider.MapProperty
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
import java.io.ByteArrayOutputStream
import java.io.File
import javax.inject.Inject

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    implementation("androidx.activity:activity:1.10.1")
    implementation("androidx.core:core:1.15.0")
    implementation("androidx.window:window-java:1.2.0")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("com.google.protobuf:protobuf-java:3.25.5")
}

// One Cargo process at a time. debug and release share target/.
abstract class CargoGate : BuildService<BuildServiceParameters.None>

val cargoGate = gradle.sharedServices.registerIfAbsent("impastoCargo", CargoGate::class.java) {
    maxParallelUsages.set(1)
}

val repoRoot: File = rootProject.projectDir.resolve("../../../..").canonicalFile
val profileGallery = providers.gradleProperty("impastoProfile").orNull == "true"
val verificationGallery = providers.gradleProperty("impastoVerification").orNull == "true"
val localSigning = profileGallery || providers.gradleProperty("impastoLocalSigning").orNull == "true"
val signing = listOf(
    "IMPASTO_ANDROID_KEYSTORE", "IMPASTO_ANDROID_STORE_PASSWORD",
    "IMPASTO_ANDROID_KEY_ALIAS", "IMPASTO_ANDROID_KEY_PASSWORD",
).associateWith { providers.environmentVariable(it).orNull?.takeUnless(String::isBlank) }
if (signing.values.any { it != null } && signing.values.any { it == null }) {
    throw GradleException("missing Android signing configuration: ${signing.filterValues { it == null }.keys.joinToString()}")
}
val signingFile = signing["IMPASTO_ANDROID_KEYSTORE"]
val androidNdkVersion = "28.2.13676358"
val rustTargets = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "x86_64" to "x86_64-linux-android",
)
val selectedAbis = providers.gradleProperty("impastoAbis").orNull
    ?.split(',')?.map(String::trim)?.toSet() ?: rustTargets.keys
val unknownAbis = selectedAbis - rustTargets.keys
if (unknownAbis.isNotEmpty()) {
    throw GradleException("unsupported Android ABIs: ${unknownAbis.joinToString()}; use ${rustTargets.keys.joinToString()}")
}

android {
    namespace = "dev.impasto.host"
    compileSdk = 35
    ndkVersion = androidNdkVersion

    if (signingFile != null) {
        signingConfigs.create("impastoRelease") {
            storeFile = file(signingFile)
            storePassword = signing["IMPASTO_ANDROID_STORE_PASSWORD"]
            keyAlias = signing["IMPASTO_ANDROID_KEY_ALIAS"]
            keyPassword = signing["IMPASTO_ANDROID_KEY_PASSWORD"]
        }
    }

    defaultConfig {
        applicationId = "dev.impasto.host"
        // libvulkan.so requires 24. The Cargo linker uses this same API level.
        minSdk = 24
        targetSdk = 35
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += selectedAbis
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildTypes {
        debug {
            if (verificationGallery) applicationIdSuffix = ".verification"
        }
        release {
            isMinifyEnabled = false
            signingConfig = when {
                signingFile != null -> signingConfigs.getByName("impastoRelease")
                localSigning -> signingConfigs.getByName("debug")
                else -> null
            }
            ndk.debugSymbolLevel = "FULL"
            if (profileGallery) {
                applicationIdSuffix = ".profile"
                isProfileable = true
            }
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
        variant.packaging.jniLibs.useLegacyPackaging.set(!cargoRelease)
        variant.packaging.jniLibs.useLegacyPackagingFromBundle.set(!cargoRelease)
        if (!cargoRelease) {
            variant.packaging.jniLibs.keepDebugSymbols.add("**/libimpasto.so")
        }
        val cargo = tasks.register<CargoImpasto>("cargoImpasto$capital") {
            usesService(cargoGate)
            release.set(cargoRelease)
            minSdk.set(api)
            ndkVersion.set(androidNdkVersion)
            targets.set(rustTargets.filterKeys { it in selectedAbis })
            sdkDir.set(sdk)
            if (!envNdk.isNullOrBlank()) {
                ndkOverride.set(File(envNdk))
            }
            repoPath.set(repoRoot.path)
            // Cargo fingerprints sources. Gradle must not, because a repo-wide file tree overlaps APK outputs.
            outputs.upToDateWhen { false }
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoImpasto::jniLibs)
        variant.sources.assets?.addGeneratedSourceDirectory(cargo, CargoImpasto::sdkNotices)

        val assets = tasks.register<StageApkAssets>("stageApkAssets$capital") {
            quotes.set(repoRoot.resolve("apps/gallery-solid/dist/quotes.js"))
            font.set(repoRoot.resolve("crates/embedder/android/assets/fonts/Sans.ttf"))
            logo.set(repoRoot.resolve("crates/embedder/macos/assets/logo.png"))
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

    @get:Input
    abstract val ndkVersion: Property<String>

    @get:Input
    abstract val targets: MapProperty<String, String>

    @get:Internal
    abstract val sdkDir: DirectoryProperty

    @get:Internal
    abstract val ndkOverride: DirectoryProperty

    @get:Input
    abstract val repoPath: Property<String>

    @get:OutputDirectory
    abstract val jniLibs: DirectoryProperty

    @get:OutputDirectory
    abstract val sdkNotices: DirectoryProperty

    @TaskAction
    fun build() {
        val ndk = resolveNdk()
        val bin = llvmBin(ndk)
        val root = File(repoPath.get())
        val libraries = jniLibs.get().asFile
        if (!libraries.deleteRecursively()) {
            throw GradleException("could not clear ${libraries.path}")
        }
        val notices = sdkNotices.get().asFile
        notices.mkdirs()
        for ((abi, target) in targets.get()) {
            val sdk = buildAbi(root, ndk, bin, target, libraries.resolve(abi))
            sdk.resolve("LICENSE").copyTo(notices.resolve("Impeller-LICENSE"), overwrite = true)
        }
    }

    private fun buildAbi(root: File, ndk: File, bin: File, target: String, output: File): File {
        val api = minSdk.get()
        val linker = File(bin, "$target$api-clang")
        val sysroot = bin.parentFile.resolve("sysroot").invariantSeparatorsPath
        if (!linker.isFile) {
            throw GradleException("missing Android linker ${linker.path}")
        }
        val sdkPath = ByteArrayOutputStream()
        val python = if (System.getProperty("os.name").startsWith("Windows")) "python" else "python3"
        execOps.exec {
            commandLine(python, root.resolve("tools/impeller_sdk.py"), "prepare", target)
            workingDir(root)
            environment("PYTHONIOENCODING", "UTF-8")
            standardOutput = sdkPath
        }.assertNormalExitValue()
        val impellerSdk = File(sdkPath.toString(Charsets.UTF_8.name()).trim())
        val command = mutableListOf(
            "cargo",
            "build",
            "--locked",
            "--manifest-path",
            root.resolve("Cargo.toml").path,
            "--target",
            target,
            "-p",
            "embedder-android",
            "--features",
            "impeller",
        )
        if (release.get()) {
            command.add("--release")
        }
        execOps.exec {
            commandLine(command)
            workingDir(root)
            environment("ANDROID_NDK_HOME", ndk.path)
            environment("IMPASTO_IMPELLER_SDK_DIR", impellerSdk.path)
            // Bindgen uses libclang directly. Keep host headers out of Android bindings.
            environment("BINDGEN_EXTRA_CLANG_ARGS_${target.replace('-', '_')}", "--sysroot=\"$sysroot\"")
            if (release.get()) {
                environment("CARGO_PROFILE_RELEASE_DEBUG", "line-tables-only")
            }
            environment(
                "PATH",
                bin.path + File.pathSeparator + System.getenv("PATH").orEmpty(),
            )
            environment("CARGO_TARGET_${target.uppercase().replace('-', '_')}_LINKER", linker.path)
        }.assertNormalExitValue()

        val profile = if (release.get()) "release" else "debug"
        val built = root.resolve("target/$target/$profile/libimpasto.so")
        if (!built.isFile) {
            throw GradleException("cargo did not write ${built.path}")
        }
        output.mkdirs()
        built.copyTo(output.resolve("libimpasto.so"), overwrite = true)
        impellerSdk.resolve("lib/libimpeller.so").copyTo(output.resolve("libimpeller.so"), overwrite = true)
        return impellerSdk
    }

    private fun resolveNdk(): File {
        val override = ndkOverride.orNull?.asFile
        if (override != null && override.isDirectory) {
            return override
        }
        val installed = sdkDir.get().asFile.resolve("ndk/${ndkVersion.get()}")
        if (installed.isDirectory) {
            return installed
        }
        throw GradleException("Android NDK ${ndkVersion.get()} is missing: ${installed.path}")
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
