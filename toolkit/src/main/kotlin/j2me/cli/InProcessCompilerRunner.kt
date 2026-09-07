package j2me.cli

import j2me.process.CommandResult
import java.io.ByteArrayOutputStream
import java.io.PrintWriter
import java.lang.reflect.InvocationTargetException
import java.net.URLClassLoader
import java.nio.charset.StandardCharsets
import java.nio.file.Path

internal class InProcessCompilerRunner(private val fallback: CompilerRunner) : CompilerRunner {
    private val workers = ThreadLocal.withInitial { mutableMapOf<CompilerKey, ReflectiveCompiler>() }

    override fun run(request: CompilerRequest): CommandResult {
        val config = request.compiler as? JarCompiler ?: return fallback.run(request)
        val key = CompilerKey(config.jar, config.backend)
        val compiler = try {
            workers.get().getOrPut(key) { ReflectiveCompiler(key) }
        } catch (_: Throwable) {
            // Native runtimes and incompatible compiler jars use the process launcher.
            return fallback.run(request)
        }
        return compiler.run(request.command(), request.arguments().toTypedArray())
    }
}

private data class CompilerKey(
    val jar: Path,
    val backend: CompileBackend,
)

private class ReflectiveCompiler(
    private val key: CompilerKey,
) {
    private val loader = URLClassLoader(arrayOf(key.jar.toUri().toURL()), ClassLoader.getPlatformClassLoader())
    private val main = Class.forName(requireNotNull(key.backend.mainClass), true, loader)

    fun run(cmd: List<String>, args: Array<String>): CommandResult {
        val stdout = ByteArrayOutputStream()
        val stderr = ByteArrayOutputStream()
        val outWriter = PrintWriter(stdout.writer(StandardCharsets.UTF_8), true)
        val errWriter = PrintWriter(stderr.writer(StandardCharsets.UTF_8), true)

        val rc = try {
            when (key.backend) {
                CompileBackend.LEGACY -> runLegacy(args, errWriter)
                CompileBackend.ECJ -> runEcj(args, outWriter, errWriter)
                CompileBackend.JAVAC -> error("javac uses the process launcher")
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
