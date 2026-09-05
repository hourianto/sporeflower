plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.graalvm.native)
    application
}

repositories {
    mavenCentral()
}

version = rootProject.version

val decompilerArtifact by configurations.creating {
    isCanBeConsumed = false
    isCanBeResolved = true
}

dependencies {
    implementation(project(path = ":", configuration = "decompilerDistribution"))
    decompilerArtifact(project(path = ":", configuration = "decompilerDistribution"))
    implementation(libs.clikt)
    implementation(libs.asm)
    implementation(libs.asm.commons)
    implementation(libs.asm.tree)
    implementation(libs.javaparser.core)
    implementation(libs.mapping.io)
    implementation(libs.kotlinx.coroutines.core)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.tomlj)
    runtimeOnly(libs.slf4j.simple)

    testImplementation(libs.junit.jupiter)
    testImplementation(libs.kotest.runner.junit5)
    testImplementation(libs.kotest.assertions.core)
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

application {
    applicationName = "j2me"
    mainClass = "j2me.MainKt"
    applicationDefaultJvmArgs = listOf(
        "-XX:+IgnoreUnrecognizedVMOptions",
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:-UseJVMCICompiler",
        "-XX:-UnlockExperimentalVMOptions",
        "-XX:CompileThresholdScaling=1.5",
        "--enable-native-access=ALL-UNNAMED",
    )
}

graalvmNative {
    toolchainDetection.set(true)
    binaries {
        named("main") {
            imageName.set("j2me-native")
            mainClass.set("j2me.MainKt")
            buildArgs.add("--no-fallback")
            buildArgs.add("-O2")
            buildArgs.add("-H:IncludeResources=j2me/builtin-mappings/.*\\.map")
            javaLauncher.set(
                javaToolchains.launcherFor {
                    languageVersion.set(JavaLanguageVersion.of(25))
                    vendor.set(org.gradle.jvm.toolchain.JvmVendorSpec.ORACLE)
                },
            )
        }
    }
}

tasks.named<Test>("test") {
    useJUnitPlatform()
    filter.isFailOnNoMatchingTests = false
    dependsOn(decompilerArtifact, "installDist", "distZip")
    inputs.files(decompilerArtifact).withPropertyName("decompilerArtifact")
    doFirst {
        systemProperty("sporeflower.test.jar", decompilerArtifact.singleFile.absolutePath)
        systemProperty("j2me.test.installation", layout.buildDirectory.dir("install/j2me").get().asFile.absolutePath)
        systemProperty("j2me.test.archive", tasks.named<Zip>("distZip").get().archiveFile.get().asFile.absolutePath)
    }
}

// The engine is a normal library dependency. The standalone copy also serves
// native CLI builds and explicit subprocess runs.
distributions {
    main {
        contents {
            from(decompilerArtifact) {
                into("decompiler")
                rename { "sporeflower.jar" }
            }
            from(rootProject.file("docs")) { into("docs") }
            // Project guidance and the published reference share one source.
            from(rootProject.file("docs/MAPPINGS.md")) {
                into("templates")
                rename { "mappings-doc.md" }
            }
            from(rootProject.file("README.md"))
            from(rootProject.file("LICENSE.md"))
            from("config/global.example.toml") { into("config") }
        }
    }
}

tasks.named<JavaExec>("run") {
    dependsOn("installDist")
    systemProperty("j2me.home", layout.buildDirectory.dir("install/j2me").get().asFile.absolutePath)
}

tasks.named<Sync>("installDist") {
    // Local settings are not part of the distribution inputs. Preserve them
    // when Gradle refreshes an existing installation after a source change.
    preserve { include("config/global.toml") }
    // Local-only inputs: never put these unreviewed binaries in release archives.
    from("vendor") { into("vendor") }
}

tasks.withType<Jar>().configureEach {
    isReproducibleFileOrder = true
    isPreserveFileTimestamps = false
}
