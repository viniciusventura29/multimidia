import com.android.build.gradle.LibraryExtension

// Segue o gradle das outras dependências Tauri do projeto (kotlinVersion,
// AndroidManifest do app). Não foi rodado — sem Android Studio/SDK neste Mac
// para confirmar. Conferir a versão do AGP contra o restante do projeto quando
// `npx tauri android init` gerar o `settings.gradle.kts` de verdade.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.eclipseos.mediasession"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
}
