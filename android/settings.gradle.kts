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

rootProject.name = "voiceon-android"
// Ядро на Rust (../core через ../ffi): .so и Kotlin-биндинги UniFFI.
include(":core")
include(":app")
