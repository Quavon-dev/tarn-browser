plugins {
    id("com.android.application") version "8.13.2"
    id("org.cyclonedx.bom") version "3.5.1" apply false
}

android {
    namespace = "dev.tarn.android"
    compileSdk = 36

    defaultConfig {
        applicationId = "de.quavon.tarn"
        minSdk = 28
        targetSdk = 36
        versionCode = 9
        versionName = "0.1.8"
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
    testImplementation("junit:junit:4.13.2")
}

// The release workflow writes the SBOM with -Psbom after the APK is uploaded, so the plugin never runs in its build.
if (providers.gradleProperty("sbom").isPresent) {
    apply(plugin = "org.cyclonedx.bom")
    tasks.named("cyclonedxDirectBom") { setProperty("includeConfigs", listOf("releaseRuntimeClasspath")) }
}
