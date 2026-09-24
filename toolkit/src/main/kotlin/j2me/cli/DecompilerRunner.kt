package j2me.cli

import org.jetbrains.java.decompiler.api.Decompiler
import org.jetbrains.java.decompiler.api.NamingPlan
import org.jetbrains.java.decompiler.api.J2meApi
import org.jetbrains.java.decompiler.api.SemanticMappingData
import org.jetbrains.java.decompiler.main.Init
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver
import org.jetbrains.java.decompiler.main.decompiler.PrintStreamLogger
import j2me.process.ProcessRunner
import j2me.process.writeCommandLogs
import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.charset.StandardCharsets
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.pathString
import kotlin.system.measureTimeMillis

internal data class DecompilerInvocation(
    val source: Path,
    val output: Path,
    val options: Map<String, String>,
    val libraries: List<Path>,
    val logStdoutPath: Path,
    val logStderrPath: Path,
    val semantics: SemanticMappingData? = null,
    val preparedNames: NamingPlan? = null,
    val api: J2meApi.Resolution? = null,
)

internal fun interface DecompilerRunner {
    fun run(invocation: DecompilerInvocation): Long

    fun prepareNames(invocation: DecompilerInvocation): NamingPlan = BundledDecompiler.prepareNames(invocation)
}

internal class SporeflowerRunner(
    private val paths: ToolkitPaths,
    private val runner: ProcessRunner,
) : DecompilerRunner {
    override fun prepareNames(invocation: DecompilerInvocation): NamingPlan {
        if (!isNativeRuntime()) return BundledDecompiler.prepareNames(invocation)
        val mapping = invocation.output.resolve("prepared.tiny")
        runBundledDecompilerJvm(paths, runner, invocation.copy(options = invocation.options + mapOf(
            "prepare-names-only" to "true", "naming-output" to mapping.toString(),
        )))
        return NamingPlan.read(mapping)
    }

    override fun run(invocation: DecompilerInvocation): Long {
        // Native Image cannot load the engine's JVM classes. Launch only the
        // matching Sporeflower JAR shipped with this installation in that case.
        if (isNativeRuntime()) {
            return runBundledDecompilerJvm(paths, runner, invocation)
        }
        return measureTimeMillis { BundledDecompiler.decompile(invocation) }
    }
}

internal fun isNativeRuntime(): Boolean = System.getProperty("org.graalvm.nativeimage.imagecode") == "runtime"

internal fun decompilerJava(): String =
    System.getenv("JAVA_HOME")?.takeIf { it.isNotBlank() }?.let { Path.of(it, "bin", "java").toString() } ?: "java"

internal fun runBundledDecompilerJvm(paths: ToolkitPaths, runner: ProcessRunner, invocation: DecompilerInvocation): Long =
    measureTimeMillis {
        require(paths.bundledDecompiler.exists()) { "Bundled Sporeflower JAR not found: ${paths.bundledDecompiler}" }
        val transportOptions = invocation.options.toMutableMap()
        invocation.preparedNames?.let { names ->
            val path = transportOptions["naming-output"]?.let(Path::of)
                ?: invocation.logStdoutPath.parent.resolve("prepared-names.tiny")
            names.write(path)
            transportOptions["prepared-names-path"] = path.pathString
        }
        invocation.semantics?.let { data ->
            val path = invocation.logStdoutPath.parent.resolve("semantic-map.json")
            data.write(path)
            transportOptions["semantic-mappings-path"] = path.pathString
        }
        val libraries = invocation.libraries + listOfNotNull(invocation.api?.let {
            transportOptions["bundled-j2me-api"] = "false"
            writeApiSnapshot(it, invocation.logStdoutPath.parent.resolve("api"))
        })
        val options = transportOptions.map { (key, value) -> "--$key=$value" }.toMutableList()
        if (libraries.isNotEmpty()) {
            options += "--add-external=${libraries.joinToString(",") { it.pathString }}"
        }
        runner.run(
            cmd = listOf(
                decompilerJava(),
                "-XX:+IgnoreUnrecognizedVMOptions",
                "-XX:+UnlockExperimentalVMOptions",
                "-XX:-UseJVMCICompiler",
                "-XX:-UnlockExperimentalVMOptions",
                "-XX:CompileThresholdScaling=1.5",
                "-jar", paths.bundledDecompiler.pathString,
            ) + options + listOf(invocation.source.pathString, invocation.output.pathString),
            logStdoutPath = invocation.logStdoutPath,
            logStderrPath = invocation.logStderrPath,
        )
    }

private object BundledDecompiler {
    // Kotlin object initialization serializes Init.init(), which itself is not
    // synchronized. Fullrun can then create separate contexts on worker threads.
    init { Init.init() }

    fun prepareNames(invocation: DecompilerInvocation): NamingPlan = execute(invocation, true)!!

    fun decompile(invocation: DecompilerInvocation) { execute(invocation, false) }

    private fun execute(invocation: DecompilerInvocation, prepareOnly: Boolean): NamingPlan? {
        val stdout = ByteArrayOutputStream()
        val stderr = ByteArrayOutputStream()
        PrintStream(stdout, true, StandardCharsets.UTF_8).use { log ->
            PrintStream(stderr, true, StandardCharsets.UTF_8).use { errors ->
                try {
                    invocation.output.createDirectories()
                    val builder = Decompiler.builder()
                        .inputs(invocation.source.toFile())
                        .libraries(*invocation.libraries.map { it.toFile() }.toTypedArray())
                        .output(DirectoryResultSaver(invocation.output.toFile()))
                        .logger(PrintStreamLogger(log))
                        .semanticMappings(invocation.semantics)
                        .preparedNames(invocation.preparedNames)
                    invocation.api?.let { builder.libraries(it) }
                    invocation.options.forEach { (key, value) -> builder.option(key, value) }
                    val decompiler = builder.build()
                    if (prepareOnly) return decompiler.prepareNames()
                    decompiler.decompile()
                } catch (exc: Throwable) {
                    exc.printStackTrace(errors)
                    throw exc
                } finally {
                    writeCommandLogs(invocation.logStdoutPath, invocation.logStderrPath,
                        stdout.toString(StandardCharsets.UTF_8), stderr.toString(StandardCharsets.UTF_8))
                }
            }
        }
        return null
    }
}
