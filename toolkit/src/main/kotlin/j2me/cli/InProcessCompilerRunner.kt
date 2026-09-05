package j2me.cli

import j2me.process.CommandResult
import j2me.process.ProcessRunner
import j2me.process.checkCommandResult
import j2me.process.writeCommandLogs
import java.io.ByteArrayOutputStream
import java.io.PrintWriter
import java.lang.reflect.InvocationTargetException
import java.net.URLClassLoader
import java.nio.charset.StandardCharsets
import java.nio.file.Path
import kotlin.io.path.pathString

internal class InProcessCompilerRunner(
    private val fallback: ProcessRunner,
) : ProcessRunner {
    private val workers = ThreadLocal.withInitial { mutableMapOf<CompilerKey, ReflectiveCompiler>() }

    override fun run(
        cmd: List<String>,
        okReturnCodes: Set<Int>,
        emitOutputOnAllowedNonZero: Boolean,
        cwd: Path?,
        logStdoutPath: Path?,
        logStderrPath: Path?,
    ): CommandResult {
        val parsed = parseCompilerCommand(cmd, cwd)
            ?: return fallback.run(cmd, okReturnCodes, emitOutputOnAllowedNonZero, cwd, logStdoutPath, logStderrPath)

        val compiler = try {
            workers.get().getOrPut(parsed.key) { ReflectiveCompiler(parsed.key) }
        } catch (_: Throwable) {
            return fallback.run(cmd, okReturnCodes, emitOutputOnAllowedNonZero, cwd, logStdoutPath, logStderrPath)
        }
        val result = compiler.run(cmd, parsed.args)

        writeCommandLogs(logStdoutPath, logStderrPath, result.stdout, result.stderr)
        checkCommandResult(result, okReturnCodes, emitOutputOnAllowedNonZero, logStdoutPath, logStderrPath)
        return result
    }

    private fun parseCompilerCommand(cmd: List<String>, cwd: Path?): ParsedCompiler? {
        if (cmd.size < 4 || cmd[1] != "-cp") {
            return null
        }
        val mainClass = cmd[3]
        val backend = when (mainClass) {
            "j2me.thirdparty.legacyjavac.Main" -> CompilerBackend.LEGACY
            "org.eclipse.jdt.internal.compiler.batch.Main" -> CompilerBackend.ECJ
            else -> return null
        }
        return ParsedCompiler(
            key = CompilerKey(jar = resolveAgainstCwd(cmd[2], cwd), backend = backend, mainClass = mainClass),
            args = normalizeCompilerArgsForInProcess(cmd.drop(4), cwd).toTypedArray(),
        )
    }
}

internal fun normalizeCompilerArgsForInProcess(args: List<String>, cwd: Path?): List<String> {
    if (cwd == null) {
        return args
    }

    val normalized = args.toMutableList()
    var i = 0
    while (i < normalized.size) {
        when (normalized[i]) {
            "-bootclasspath",
            "-classpath",
            "-cp" -> {
                if (i + 1 < normalized.size) {
                    normalized[i + 1] = normalizePathList(normalized[i + 1], cwd)
                    i += 2
                    continue
                }
            }
            "-d",
            "-sourcepath" -> {
                if (i + 1 < normalized.size) {
                    normalized[i + 1] = resolveAgainstCwd(normalized[i + 1], cwd).pathString
                    i += 2
                    continue
                }
            }
        }

        if (normalized[i].startsWith("@")) {
            normalized[i] = "@" + resolveAgainstCwd(normalized[i].drop(1), cwd).pathString
        }
        i++
    }
    return normalized
}

private fun normalizePathList(value: String, cwd: Path): String =
    value.split(java.io.File.pathSeparatorChar).joinToString(java.io.File.pathSeparator) { entry ->
        if (entry.isEmpty()) entry else resolveAgainstCwd(entry, cwd).pathString
    }

private fun resolveAgainstCwd(value: String, cwd: Path?): Path {
    val path = Path.of(value)
    return if (path.isAbsolute || cwd == null) {
        path.toAbsolutePath().normalize()
    } else {
        cwd.resolve(path).normalize()
    }
}

private enum class CompilerBackend {
    LEGACY,
    ECJ,
}

private data class CompilerKey(
    val jar: Path,
    val backend: CompilerBackend,
    val mainClass: String,
)

private data class ParsedCompiler(
    val key: CompilerKey,
    val args: Array<String>,
)

private class ReflectiveCompiler(
    private val key: CompilerKey,
) {
    private val loader = URLClassLoader(arrayOf(key.jar.toUri().toURL()), ClassLoader.getPlatformClassLoader())
    private val main = Class.forName(key.mainClass, true, loader)

    fun run(cmd: List<String>, args: Array<String>): CommandResult {
        val stdout = ByteArrayOutputStream()
        val stderr = ByteArrayOutputStream()
        val outWriter = PrintWriter(stdout.writer(StandardCharsets.UTF_8), true)
        val errWriter = PrintWriter(stderr.writer(StandardCharsets.UTF_8), true)

        val rc = try {
            when (key.backend) {
                CompilerBackend.LEGACY -> runLegacy(args, errWriter)
                CompilerBackend.ECJ -> runEcj(args, outWriter, errWriter)
            }
        } catch (exc: InvocationTargetException) {
            val cause = exc.targetException ?: exc
            cause.printStackTrace(errWriter)
            1
        } catch (exc: Throwable) {
            exc.printStackTrace(errWriter)
            1
        } finally {
            outWriter.flush()
            errWriter.flush()
        }

        return CommandResult(
            command = cmd,
            returnCode = rc,
            stdout = stdout.toString(StandardCharsets.UTF_8),
            stderr = stderr.toString(StandardCharsets.UTF_8),
        )
    }

    private fun runLegacy(args: Array<String>, errWriter: PrintWriter): Int {
        val compile = main.getMethod("compile", Array<String>::class.java, PrintWriter::class.java)
        return compile.invoke(null, args, errWriter) as Int
    }

    private fun runEcj(args: Array<String>, outWriter: PrintWriter, errWriter: PrintWriter): Int {
        val constructor = main.getConstructor(PrintWriter::class.java, PrintWriter::class.java, Boolean::class.javaPrimitiveType)
        val instance = constructor.newInstance(outWriter, errWriter, false)
        val compile = main.getMethod("compile", Array<String>::class.java)
        val ok = compile.invoke(instance, args) as Boolean
        return if (ok) 0 else 1
    }
}
