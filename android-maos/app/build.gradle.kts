import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// O endereço do relay e a chave entram no APK pelo build, a partir de
// ~/.eclipse-carro.properties (fora do git: o repositório é público; e fora do
// workspace, para valer em todos). Um android-maos/maos.properties, se
// existir, sobrepõe — é assim que o emulador aponta para o `wrangler dev`.
// Sem nenhum dos dois o APK compila, mas a tela do app avisa que não sabe para
// onde ligar.
val segredos = Properties().apply {
    for (f in listOf(File(System.getProperty("user.home"), ".eclipse-carro.properties"), rootProject.file("maos.properties"))) {
        if (f.exists()) f.inputStream().use { load(it) }
    }
}

android {
    namespace = "com.eclipseos.maos"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.eclipseos.maos"
        minSdk = 26
        // 29 é o Android da central (ela diz 16 na tela "Sobre", mas é 10).
        // Mirar nele evita as regras de visibilidade de pacotes do 11+, que
        // esconderiam a lista de apps instalados.
        targetSdk = 29
        versionCode = 2
        versionName = "2"
        buildConfigField("String", "RELAY", "\"${segredos.getProperty("relay", "")}\"")
        buildConfigField("String", "CHAVE", "\"${segredos.getProperty("chave", "")}\"")
    }

    buildFeatures { buildConfig = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    lint {
        // O targetSdk 29 é de propósito (ver acima); o lint de release o
        // trata como erro fatal.
        checkReleaseBuilds = false
        abortOnError = false
    }
}

dependencies {
    // O Android não tem cliente de WebSocket próprio.
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}
