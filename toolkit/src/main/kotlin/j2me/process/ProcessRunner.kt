package j2me.process

import java.nio.file.Path
import kotlin.concurrent.thread
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

data class CommandResult(
    val command: List<String>,
    val returnCode: Int,
    val stdout: String,
    val stderr: String,
)

internal fun writeCommandLogs(
    logStdoutPath: Path?,
    logStderrPath: Path?,
    stdout: String,
    stderr: String,
) {
    logStdoutPath?.let {
        it.parent?.createDirectories()
        it.writeText(stdout)
    }
    logStderrPath?.let {
        it.parent?.createDirectories()
        it.writeText(stderr)
    }
}

internal fun checkCommandResult(
    result: CommandResult,
    okReturnCodes: Set<Int>,
    emitOutputOnAllowedNonZero: Boolean,
    logStdoutPath: Path?,
    logStderrPath: Path?,
) {
    if (okReturnCodes.isNotEmpty() && result.returnCode !in okReturnCodes) {
        throw IllegalStateException(
            "Command failed (${result.returnCode}): ${result.command.joinToString(" ")}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}",
        )
    }

    if (emitOutputOnAllowedNonZero && result.returnCode != 0) {
        System.err.println("Command exited with code ${result.returnCode}: ${result.command.joinToString(" ")}")
        if (logStdoutPath != null || logStderrPath != null) {
            System.err.println(
                "Full logs: stdout=${logStdoutPath ?: "<none>"} stderr=${logStderrPath ?: "<none>"}",
            )
        }
        emitImportantOutput(result.stdout)
        emitImportantOutput(result.stderr)
    }
}

private fun emitImportantOutput(text: String) {
    if (text.isBlank()) {
        return
    }
    val important = text.lines().filter { it.contains("ERROR") || it.contains("Exception") || it.contains("Caused by") }
    if (important.isNotEmpty()) {
        System.err.println(important.joinToString("\n"))
    } else {
        System.err.println(text.lines().takeLast(20).joinToString("\n"))
    }
}

interface ProcessRunner {
    fun run(
        cmd: List<String>,
        okReturnCodes: Set<Int> = setOf(0),
        emitOutputOnAllowedNonZero: Boolean = false,
        cwd: Path? = null,
        logStdoutPath: Path? = null,
        logStderrPath: Path? = null,
    ): CommandResult
}

class RealProcessRunner : ProcessRunner {
    override fun run(
        cmd: List<String>,
        okReturnCodes: Set<Int>,
        emitOutputOnAllowedNonZero: Boolean,
        cwd: Path?,
        logStdoutPath: Path?,
        logStderrPath: Path?,
    ): CommandResult {
        require(cmd.isNotEmpty()) { "Command must not be empty" }
        val process = ProcessBuilder(cmd)
            .apply {
                if (cwd != null) {
                    directory(cwd.toFile())
                }
            }
            .start()

        var stdout = ""
        var stderr = ""
        var stdoutError: Throwable? = null
        var stderrError: Throwable? = null

        val stdoutThread = thread(name = "process-stdout-reader", start = true) {
            try {
                stdout = process.inputStream.bufferedReader().use { it.readText() }
            } catch (exc: Throwable) {
                stdoutError = exc
            }
        }
        val stderrThread = thread(name = "process-stderr-reader", start = true) {
            try {
                stderr = process.errorStream.bufferedReader().use { it.readText() }
            } catch (exc: Throwable) {
                stderrError = exc
            }
        }

        val rc = process.waitFor()
        stdoutThread.join()
        stderrThread.join()

        stdoutError?.let { throw IllegalStateException("Failed reading command stdout for: ${cmd.joinToString(" ")}", it) }
        stderrError?.let { throw IllegalStateException("Failed reading command stderr for: ${cmd.joinToString(" ")}", it) }

        writeCommandLogs(logStdoutPath, logStderrPath, stdout, stderr)
        val result = CommandResult(cmd, rc, stdout, stderr)
        checkCommandResult(result, okReturnCodes, emitOutputOnAllowedNonZero, logStdoutPath, logStderrPath)
        return result
    }
}
