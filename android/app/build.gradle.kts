import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// Версия приложения — общая версия workspace из ../Cargo.toml: релиз поднимает только её.
val cargoVersion: String = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }
    .substringAfter('"').substringBefore('"')
val (vMajor, vMinor, vPatch) = cargoVersion.split('.').map { it.takeWhile(Char::isDigit).toInt() }

// Ключ подписи релизов лежит вне репозитория. Путь к файлу свойств (storeFile, storePassword,
// keyAlias, keyPassword): -PvoiceonSigning=…, переменная VOICEON_ANDROID_SIGNING или
// ~/.config/voiceon-android.properties. Без него release-сборка остаётся неподписанной.
val signingPropsFile: File? = (
    providers.gradleProperty("voiceonSigning").orNull
        ?: providers.environmentVariable("VOICEON_ANDROID_SIGNING").orNull
    )?.let(::File)
    ?: File(System.getProperty("user.home"), ".config/voiceon-android.properties").takeIf { it.exists() }
val signingProps = Properties().apply {
    signingPropsFile?.takeIf { it.exists() }?.inputStream()?.use { load(it) }
}

android {
    namespace = "io.github.arteminariy.voiceon"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.arteminariy.voiceon"
        minSdk = 26
        targetSdk = 36
        versionCode = vMajor * 10000 + vMinor * 100 + vPatch
        versionName = cargoVersion
        // Ровно те ABI, для которых собирается libvoiceon_ffi.so (JNA несёт больше).
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }

    signingConfigs {
        if (!signingProps.isEmpty) {
            create("release") {
                storeFile = File(signingProps.getProperty("storeFile"))
                storePassword = signingProps.getProperty("storePassword")
                keyAlias = signingProps.getProperty("keyAlias")
                keyPassword = signingProps.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
}

dependencies {
    implementation(project(":core"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
}
