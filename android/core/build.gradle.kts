// Android-обёртка ядра: libvoiceon_ffi.so из ../ffi (cargo-ndk) и Kotlin-биндинги UniFFI.
import java.util.Properties
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.Exec

plugins {
    alias(libs.plugins.android.library)
}

val repoRoot: File = rootProject.projectDir.resolve("..").canonicalFile
val localProps = Properties().apply {
    rootProject.file("local.properties").takeIf { it.exists() }?.inputStream()?.use { load(it) }
}
val cargoTargetDir: String = providers.environmentVariable("CARGO_TARGET_DIR")
    .orElse(repoRoot.resolve("target").absolutePath).get()
// Каталог с cargo-ndk, если его нет в PATH: свойство cargoBin или cargo.bin в local.properties.
val cargoBin: String? = providers.gradleProperty("cargoBin").orNull ?: localProps.getProperty("cargo.bin")
val pathEnv: String = listOfNotNull(cargoBin, providers.environmentVariable("PATH").orNull).joinToString(File.pathSeparator)
val abis = listOf("arm64-v8a", "armeabi-v7a", "x86_64")
val minSdkLevel = 26
val ndkVersionPinned = "30.0.16248370"
// Биндинги генерируются из отладочной библиотеки хоста: в release-профиле символы
// (а с ними метаданные UniFFI) вырезаются.
val hostLibDir = "$cargoTargetDir/debug"
val hostLib = "$hostLibDir/${System.mapLibraryName("voiceon_ffi")}"
// Убираем из .so локальные пути (домашний каталог, место репозитория), как scripts/release.sh.
val home: String = System.getProperty("user.home")
val remapRustFlags = listOf(
    "--remap-path-prefix=$home/.cargo/registry/src=/cargo",
    "--remap-path-prefix=$home/.rustup=/rustup",
    "--remap-path-prefix=${repoRoot.absolutePath}=/voiceon",
).joinToString(" ")

val rustInputs = fileTree(repoRoot) {
    include("Cargo.toml", "Cargo.lock", "proto/**", "core/**", "ffi/**")
    exclude("**/target/**")
}

abstract class CargoOutputTask : Exec() {
    @get:OutputDirectory
    abstract val outDir: DirectoryProperty
}

android {
    namespace = "io.github.arteminariy.voiceon.core"
    compileSdk = 37
    ndkVersion = ndkVersionPinned

    defaultConfig {
        minSdk = minSdkLevel
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

val ndkDir = androidComponents.sdkComponents.ndkDirectory

val cargoNdk = tasks.register<CargoOutputTask>("cargoNdk") {
    group = "rust"
    description = "Собирает libvoiceon_ffi.so для ${abis.joinToString()} через cargo-ndk"
    inputs.files(rustInputs)
    outDir.set(layout.buildDirectory.dir("rust/jniLibs"))
    workingDir = repoRoot
    environment("PATH", pathEnv)
    environment("RUSTFLAGS", remapRustFlags)
    environment("CFLAGS", "-ffile-prefix-map=$home=/build")
    val args = mutableListOf("cargo", "ndk")
    abis.forEach { args += listOf("-t", it) }
    args += listOf("-P", minSdkLevel.toString(), "-o", outDir.get().asFile.absolutePath, "build", "--release", "-p", "voiceon-ffi")
    commandLine(args)
    val ndk = ndkDir
    doFirst { environment("ANDROID_NDK_HOME", ndk.get().asFile.absolutePath) }
}

val cargoHost = tasks.register<Exec>("cargoHost") {
    group = "rust"
    description = "Собирает libvoiceon_ffi и uniffi-bindgen для хоста (метаданные биндингов)"
    inputs.files(rustInputs)
    outputs.file(hostLib)
    workingDir = repoRoot
    environment("PATH", pathEnv)
    commandLine("cargo", "build", "-p", "voiceon-ffi", "-p", "voiceon-uniffi-bindgen")
}

val uniffiKotlin = tasks.register<CargoOutputTask>("uniffiKotlin") {
    group = "rust"
    description = "Генерирует Kotlin-биндинги UniFFI из библиотеки хоста"
    dependsOn(cargoHost)
    inputs.file(hostLib)
    outDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
    workingDir = repoRoot
    commandLine(
        "$hostLibDir/uniffi-bindgen", "generate",
        "--library", hostLib,
        "--language", "kotlin",
        "--no-format",
        "--out-dir", outDir.get().asFile.absolutePath,
    )
}

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoNdk, CargoOutputTask::outDir)
        variant.sources.kotlin?.addGeneratedSourceDirectory(uniffiKotlin, CargoOutputTask::outDir)
    }
}

dependencies {
    api("${libs.jna.get()}@aar")
}
