package j2me.cli

import j2me.process.CommandResult
import j2me.process.ProcessRunner
import java.io.File
import java.nio.file.Path
import java.util.concurrent.ConcurrentHashMap
import kotlin.io.path.isDirectory
import kotlin.io.path.pathString

internal enum class CompileBackend(val id: String, val mainClass: String? = null) {
    JAVAC("javac"),
    ECJ("ecj", "org.eclipse.jdt.internal.compiler.batch.Main"),
    LEGACY("legacy", "j2me.thirdparty.legacyjavac.Main");

    companion object {
        fun parse(value: String): CompileBackend = entries.find { it.id == value.lowercase() }
            ?: throw IllegalArgumentException("Unsupported compile-stubs compiler: $value (expected legacy, ecj, or javac)")
    }
}

internal object CompileStubDefaults {
    val backend = CompileBackend.LEGACY
    const val javaRelease = 8
    const val maxCompilerErrors = 100000
    const val javacBin = "javac"
    const val javaBin = "java"
}

internal sealed interface JavaCompiler {
    val backend: CompileBackend
    val cwd: Path? get() = null
    val command: List<String>
    val settings: Map<String, String>
    fun options(): List<String>

    fun diagnostics(stderr: String): CompilerDiagnostics = when (backend) {
        CompileBackend.JAVAC -> parseJavacDiagnostics(stderr)
        CompileBackend.ECJ -> parseEcjDiagnostics(stderr)
        CompileBackend.LEGACY -> parseLegacyJavacDiagnostics(stderr)
    }
}

internal data class JavacCompiler(val javaRelease: Int, val javacBin: String, val compilerVersion: Int? = null) : JavaCompiler {
    override val backend = CompileBackend.JAVAC
    override val command get() = listOf(javacBin)
    override val settings get() = linkedMapOf("java_release" to javaRelease.toString(), "javac_bin" to javacBin)
    override fun options(): List<String> {
        if (compilerVersion != null && compilerVersion <= 8) {
            // Java 8 has no --release. Restrict this fallback to its own API:
            // targeting an older release would also need that JDK's bootclasspath.
            require(javaRelease == 8) { "Java 8 javac requires --java-release 8; use a newer javac for other releases" }
            return listOf("-source", "8", "-target", "8", "-Xlint:-options")
        }
        return listOf("--release", javaRelease.toString(), "-Xlint:-options")
    }
}

internal data class JarCompiler(
    override val backend: CompileBackend,
    val jar: Path,
    val javaBin: String,
    override val cwd: Path,
    val sourceLevel: String,
    val targetLevel: String,
    val targetSource: String,
) : JavaCompiler {
    init { require(backend != CompileBackend.JAVAC) }
    override val command get() = listOf(javaBin, "-cp", jar.pathString, requireNotNull(backend.mainClass))
    override val settings get() = linkedMapOf(
        "java_bin" to javaBin,
        "source_level" to sourceLevel,
        "target_level" to targetLevel,
        "compiler_jar" to jar.toString(),
    ).apply { if (backend == CompileBackend.LEGACY) put("target_source", targetSource) }
    override fun options() = listOf("-source", sourceLevel, "-target", targetLevel) +
        if (backend == CompileBackend.ECJ) listOf("-nowarn") else emptyList()
}

/** Paths are resolved by the workspace before either execution backend receives them. */
internal data class CompilerRequest(
    val compiler: JavaCompiler,
    val bootClasspath: List<Path>,
    val classpath: List<Path>,
    val outputDir: Path,
    val sourceList: Path,
    val maxErrors: Int? = null,
) {
    fun arguments(cwd: Path? = null): List<String> = buildList {
        addAll(compiler.options())
        add("-g:lines,source")
        if (bootClasspath.isNotEmpty()) addAll(listOf("-bootclasspath", bootClasspath.joinToString(File.pathSeparator)))
        maxErrors?.let { addAll(listOf(if (compiler.backend == CompileBackend.ECJ) "-maxProblems" else "-Xmaxerrs", it.toString())) }
        if (classpath.isNotEmpty()) {
            // Preserve the legacy process launcher's relative directory paths;
            // embedded compilers receive absolute paths and need no cwd emulation.
            val entries = classpath.joinToString(File.pathSeparator) {
                if (cwd != null && it.isDirectory()) relativeOrAbsolute(it, cwd) else it.pathString
            }
            addAll(listOf("-classpath", entries))
        }
        addAll(listOf("-d", outputDir.pathString, "@${sourceList.pathString}"))
    }

    fun command(): List<String> = compiler.command + arguments(compiler.cwd)
}

internal fun interface CompilerRunner {
    fun run(request: CompilerRequest): CommandResult
}

internal class ProcessCompilerRunner(private val process: ProcessRunner) : CompilerRunner {
    private val javacVersions = ConcurrentHashMap<String, Int>()

    override fun run(request: CompilerRequest): CommandResult {
        val compiler = request.compiler
        val effective = if (compiler is JavacCompiler) {
            // Inspect the selected executable, which may differ from our JVM.
            val version = javacVersions.computeIfAbsent(compiler.javacBin) {
                val result = process.run(listOf(it, "-version"))
                parseJavacVersion(result.stdout + "\n" + result.stderr)
            }
            request.copy(compiler = compiler.copy(compilerVersion = version))
        } else request
        return process.run(effective.command(), okReturnCodes = emptySet(), cwd = compiler.cwd)
    }
}

internal fun parseJavacVersion(output: String): Int =
    Regex("\\bjavac\\s+(?:1\\.)?(\\d+)").find(output)?.groupValues?.get(1)?.toInt()
        ?: throw IllegalArgumentException("Cannot determine javac version: ${output.trim()}")
