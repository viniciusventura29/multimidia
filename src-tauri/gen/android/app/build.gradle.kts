import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

/*
 * A chave que assina o APK.
 *
 * Mora em `keystore.properties`, que o `.gitignore` daqui já esconde — chave de
 * assinatura não entra em repositório, e este é público. Na CI o arquivo é
 * escrito em tempo de build a partir de um secret (ver `.github/workflows/apk.yml`).
 *
 * SEM O ARQUIVO O BUILD CONTINUA FUNCIONANDO, e sai sem assinatura, como saía
 * antes. É de propósito: quem clonar o repositório sem a chave ainda consegue
 * compilar, e o build de PR não passa a depender de um secret.
 */
val keystoreProperties = Properties().apply {
    val propFile = file("keystore.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}
val temChave = keystoreProperties.getProperty("storeFile") != null

android {
    compileSdk = 36
    namespace = "com.eclipseos.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.eclipseos.app"
        minSdk = 24
        targetSdk = 36
        /*
         * O Android só instala por cima de um APK já instalado se o
         * `versionCode` for MAIOR. O `tauri.properties` dá sempre 1, então dois
         * builds seguidos colidiriam e a atualização no carro seria recusada.
         *
         * Na CI quem manda é o número do run, que só cresce. Localmente cai no
         * 1 de sempre — o que significa que um build local NÃO instala por cima
         * de um da CI (é o mesmo aparelho dizendo "essa versão é mais velha").
         * Para isso, passe `ECLIPSE_VERSION_CODE` na mão.
         */
        versionCode = (System.getenv("ECLIPSE_VERSION_CODE")
            ?: tauriProperties.getProperty("tauri.android.versionCode", "1")).toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        create("release") {
            if (temChave) {
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // Condicional pelo mesmo motivo do comentário da chave: sem
            // `keystore.properties` isto ficaria apontando para um
            // signingConfig vazio, e o gradle falharia em vez de cair no
            // APK sem assinatura.
            if (temChave) {
                signingConfig = signingConfigs.getByName("release")
            }
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")