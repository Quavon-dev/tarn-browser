plugins {
    id("com.android.application") version "8.13.2"
}

android {
    namespace = "dev.tarn.android"
    compileSdk = 36

    defaultConfig {
        applicationId = "de.quavon.tarn"
        minSdk = 28
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    buildFeatures {
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets["main"].jniLibs.srcDirs("../build/tarn/jni")
    sourceSets["main"].assets.srcDirs("../build/tarn/assets")
}

dependencies {
    implementation(files("../build/tarn/nsh-classes.jar"))
    implementation("androidx.webkit:webkit:1.18.0")
}
