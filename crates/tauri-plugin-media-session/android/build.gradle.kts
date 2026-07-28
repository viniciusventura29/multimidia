import com.android.build.gradle.LibraryExtension

// Segue o gradle das outras dependências Tauri do projeto (kotlinVersion,
// AndroidManifest do app). Compilado de verdade em 28/07/2026 com
// `tauri android dev` (NDK 27.3, AGP do projeto gerado, JDK 17).
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

    // Sem isto o Kotlin adota o JVM target do JDK que estiver rodando (17 aqui) e
    // o Gradle aborta com "Inconsistent JVM-target compatibility" contra o Java
    // declarado acima. 1.8 é o que o `app/build.gradle.kts` gerado pelo Tauri usa.
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    // Traz `app.tauri.plugin.{Plugin,Invoke,JSObject}` e `app.tauri.annotation.*`.
    // O `tauri.settings.gradle` gerado já registra este projeto; sem declarar a
    // dependência aqui, o Kotlin não acha nada de `app.tauri` e falha com
    // "Unresolved reference: app". É o que o tauri-plugin-opener oficial faz.
    implementation(project(":tauri-android"))
}
