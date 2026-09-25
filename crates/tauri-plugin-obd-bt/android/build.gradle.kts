plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.eclipseos.obdbt"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        consumerProguardFiles("consumer-rules.pro")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    implementation("androidx.appcompat:appcompat:1.6.0")
    implementation("com.google.android.material:material:1.7.0")
    // O provedor FUNDIDO de localização — o mesmo que o Google Maps usa.
    //
    // O `LocationManager` cru só entrega o que o chip de GPS der, e nesta
    // central o chip nunca reportou um satélite sequer (diário de 21/09: o
    // callback de GNSS registrou e NUNCA disparou). O fundido junta satélite,
    // Wi-Fi e rede móvel, e é por isso que um tablet sem antena de GPS ainda
    // sabe onde está.
    //
    // A ROM pode não ter Play Services — e aí a classe nem existe em tempo de
    // execução. Por isso todo o uso está embrulhado em `catch (Throwable)`:
    // `NoClassDefFoundError` não é `Exception`. Ver `Localizacao.kt`.
    implementation("com.google.android.gms:play-services-location:21.3.0")

    // O App Remote do Spotify — ver `libs/PROCEDENCIA.md` para de onde veio, o
    // sha256, e por que um binário solto em vez de uma dependência normal.
    //
    // Em resumo: a Spotify NÃO publica o App Remote em repositório nenhum, e
    // sem ele não há como mandar o som para o app da central. Três versões
    // tentaram pela Web API e falharam, porque ela só comanda um aparelho que
    // já esteja anunciado no Spotify Connect — e o app só se anuncia depois de
    // ser aberto na mão.
    implementation(files("libs/spotify-app-remote-release-0.8.0.aar"))
    // O App Remote serializa por Gson e não o embute.
    implementation("com.google.code.gson:gson:2.11.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
    implementation(project(":tauri-android"))
}
