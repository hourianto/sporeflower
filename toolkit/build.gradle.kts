import java.io.File

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
    implementation(libs.asm.analysis)
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

// The engine is a normal library dependency. The native CLI launches the
// standalone copy on a JVM.
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

// Local declaration sources must also be visible during decompilation, where
// missing API types can corrupt override-family and overload analysis. Build
// them into a vendor jar; neither the sources nor this jar enter distZip.
val localApiSources = fileTree("vendor/j2me-stubs/src/main/java") { include("**/*.java") }
val localApiClasses = layout.buildDirectory.dir("local-api-stubs/classes")
val localApiSourceList = layout.buildDirectory.file("local-api-stubs/sources.txt")
val compileLocalApiStubs by tasks.registering(JavaExec::class) {
    val compilerJar = file("vendor/compilers/legacy-javac/legacy-javac.jar")
    val apiJars = fileTree("vendor/j2me-api") {
        include("*.jar")
        exclude("local-api-stubs.jar")
    }
    // Gradle removes stale class outputs when the last source is removed.
    inputs.files(localApiSources).skipWhenEmpty()
    inputs.files(apiJars)
    inputs.file(compilerJar).optional()
    outputs.dir(localApiClasses)
    classpath = files(compilerJar)
    mainClass.set("j2me.thirdparty.legacyjavac.Main")
    javaLauncher.set(javaToolchains.launcherFor { languageVersion.set(JavaLanguageVersion.of(21)) })
    doFirst {
        require(compilerJar.isFile) { "Local API sources require vendor/compilers/legacy-javac/legacy-javac.jar" }
        val classes = localApiClasses.get().asFile
        classes.deleteRecursively()
        classes.mkdirs()
        val sources = localApiSourceList.get().asFile
        sources.writeText(localApiSources.files.sorted().joinToString("\n", postfix = "\n") {
            "\"" + it.absolutePath.replace("\\", "\\\\").replace("\"", "\\\"") + "\""
        })
        val libraries = apiJars.files.sorted().joinToString(File.pathSeparator)
        args("-source", "1.3", "-target", "1.1", "-bootclasspath", libraries,
            "-classpath", libraries, "-d", classes.absolutePath, "@${sources.absolutePath}")
    }
}
val localApiStubsJar by tasks.registering(Jar::class) {
    dependsOn(compileLocalApiStubs)
    // Rebuild even for no sources so an old stub jar cannot remain active.
    from(localApiClasses)
    destinationDirectory.set(layout.projectDirectory.dir("vendor/j2me-api"))
    archiveFileName.set("local-api-stubs.jar")
    manifest.attributes("J2ME-Stub-Kind" to "compile-only")
}

tasks.named<Sync>("installDist") {
    dependsOn(localApiStubsJar)
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
