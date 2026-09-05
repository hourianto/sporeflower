package j2me.cli

import org.jetbrains.java.decompiler.api.Decompiler
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
import kotlin.io.path.pathString
import kotlin.system.measureTimeMillis

internal data class VineflowerInvocation(
    val bin: String,
    val javaBin: String,
    val source: Path,
    val output: Path,
    val options: Map<String, String>,
    val libraries: List<Path>,
    val logStdoutPath: Path,
    val logStderrPath: Path,
    val semantics: SemanticMappingData? = null,
    val bundled: Boolean = true,
)

internal interface VineflowerRunner {
    fun run(invocation: VineflowerInvocation): Long
}

internal class ProcessVineflowerRunner(
    private val runner: ProcessRunner,
) : VineflowerRunner {
    override fun run(invocation: VineflowerInvocation): Long =
        measureTimeMillis {
            runner.run(
                cmd = buildVineflowerCmd(invocation),
                logStdoutPath = invocation.logStdoutPath,
                logStderrPath = invocation.logStderrPath,
            )
        }

    private fun buildVineflowerCmd(invocation: VineflowerInvocation): List<String> {
        val prefix = if (invocation.bin.lowercase().endsWith(".jar")) {
            listOf(
                invocation.javaBin,
                "-XX:+IgnoreUnrecognizedVMOptions",
                "-XX:+UnlockExperimentalVMOptions",
                "-XX:-UseJVMCICompiler",
                "-XX:-UnlockExperimentalVMOptions",
                "-XX:CompileThresholdScaling=1.5",
                "-jar",
                invocation.bin,
            )
        } else {
            listOf(invocation.bin)
        }

        val transportOptions = invocation.options.toMutableMap()
        invocation.semantics?.let { data ->
            val path = invocation.logStdoutPath.parent.resolve("semantic-map.json")
            data.write(path)
            transportOptions["semantic-mappings-path"] = path.pathString
        }
        val options = transportOptions.map { (key, value) -> "--$key=$value" }.toMutableList()
        if (invocation.libraries.isNotEmpty()) {
            options += "--add-external=${invocation.libraries.joinToString(",") { it.pathString }}"
        }

        return prefix + options + listOf(invocation.source.pathString, invocation.output.pathString)
    }
}

internal class InProcessVineflowerRunner(
    private val fallback: VineflowerRunner? = null,
) : VineflowerRunner {
    override fun run(invocation: VineflowerInvocation): Long {
        // Native Image cannot load arbitrary JVM classes. Only that execution
        // mode, or an explicitly selected external engine, needs file transport.
        if (System.getProperty("org.graalvm.nativeimage.imagecode") == "runtime" || !invocation.bundled) {
            return fallback?.run(invocation)
                ?: error("In-process decompilation requires the bundled JVM engine; use process mode for an external engine")
        }
        return measureTimeMillis { BundledDecompiler.decompile(invocation) }
    }
}

private object BundledDecompiler {
    // Kotlin object initialization serializes Init.init(), which itself is not
    // synchronized. Fullrun can then create separate contexts on worker threads.
    init { Init.init() }

    fun decompile(invocation: VineflowerInvocation) {
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
                    invocation.options.forEach { (key, value) -> builder.option(key, value) }
                    builder.build().decompile()
                } catch (exc: Throwable) {
                    exc.printStackTrace(errors)
                    throw exc
                } finally {
                    writeCommandLogs(invocation.logStdoutPath, invocation.logStderrPath,
                        stdout.toString(StandardCharsets.UTF_8), stderr.toString(StandardCharsets.UTF_8))
                }
            }
        }
    }
}
