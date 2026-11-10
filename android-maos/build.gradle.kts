// Mesmas versões do app Tauri (src-tauri/gen/android), para reaproveitar o
// cache do Gradle em vez de baixar outro Android Gradle Plugin.
plugins {
    id("com.android.application") version "8.11.0" apply false
    id("org.jetbrains.kotlin.android") version "1.9.25" apply false
}
