package j2me.cli

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.collections.shouldHaveSize
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.process.CommandResult
import j2me.process.ProcessRunner
import java.io.File
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.pathString
import kotlin.io.path.readText
import kotlin.io.path.writeBytes
import kotlin.io.path.writeText

class CompileStubsTest : FunSpec({
    test("compileStubs fails on stub compile failure without compiling project sources") {
        val root = newCompileProject("compile-stub-fails")
        val stubsSrc = root.resolve("stubs").createDirectories()
        stubsSrc.resolve("Stub.java").writeText("class Stub {}\n")

        val runner = QueueProcessRunner(
            CommandResult(emptyList(), 1, "", "${stubsSrc.resolve("Stub.java")}:1: error: broken stub\n"),
            CommandResult(emptyList(), 0, "", ""),
        )

        val exc = shouldThrow<IllegalStateException> {
            compileStubs(
                root = root,
                paths = ToolkitPaths(root, root.resolve("global.toml"), root.resolve("mappings-doc.md")),
                runner = runner,
                args = CompileStubsArgs(
                    stubsSrcArg = stubsSrc.toString(),
                    noStubCache = true,
                    compiler = CompileBackend.JAVAC,
                ),
            )
        }

        exc.message shouldContain "stub compile failed"
        runner.commands shouldHaveSize 1
        val summary = root.resolve("out/compile_check/summary.txt").readText()
        summary shouldContain "stubs_exit=1"
        summary shouldContain "compile_exit=-1"
        summary shouldContain "error_lines=1"
    }

    test("skip stub compile preserves default no-cache classes directory inside output") {
        val root = newCompileProject("compile-skip-preserve")
        val outDir = root.resolve("out/compile_check").createDirectories()
        val stubsClasses = outDir.resolve("stubs-classes").createDirectories()
        val sentinel = stubsClasses.resolve("Existing.class")
        sentinel.writeBytes(byteArrayOf(0))
        outDir.resolve("old.log").writeText("stale\n")

        val runner = QueueProcessRunner(CommandResult(emptyList(), 0, "", ""))

        compileStubs(
            root = root,
            paths = ToolkitPaths(root, root.resolve("global.toml"), root.resolve("mappings-doc.md")),
            runner = runner,
            args = CompileStubsArgs(
                skipStubCompile = true,
                noStubCache = true,
                compiler = CompileBackend.JAVAC,
            ),
        )

        sentinel.exists() shouldBe true
        outDir.resolve("old.log").exists() shouldBe false
        root.resolve("out/compile_check/summary.txt").readText() shouldContain "stubs_mode=skip-flag"
        runner.commands shouldHaveSize 1
    }

    test("api jars only mode does not write a managed stub cache marker") {
        val root = newCompileProject("compile-api-jars-only")
        val runner = QueueProcessRunner(CommandResult(emptyList(), 0, "", ""))

        compileStubs(
            root = root,
            paths = ToolkitPaths(root, root.resolve("global.toml"), root.resolve("mappings-doc.md")),
            runner = runner,
            args = CompileStubsArgs(compiler = CompileBackend.JAVAC),
        )

        val summary = root.resolve("out/compile_check/summary.txt").readText()
        summary shouldContain "stubs_mode=api-jars-only"
        summary shouldContain "stub_sources=0"
        runner.commands shouldHaveSize 1
    }

    test("managed stub cache serializes concurrent cache misses for the same key") {
        val base = Files.createTempDirectory("compile-shared-cache")
        val stubsSrc = base.resolve("stubs").createDirectories()
        stubsSrc.resolve("SharedStub.java").writeText("class SharedStub {}\n")
        val first = newCompileProject("compile-shared-cache-a")
        val second = newCompileProject("compile-shared-cache-b")
        val runner = CountingProcessRunner()
        val paths = ToolkitPaths(base, base.resolve("global.toml"), base.resolve("mappings-doc.md"))
        val args = CompileStubsArgs(
            stubsSrcArg = stubsSrc.toString(),
            compiler = CompileBackend.JAVAC,
        )

        val executor = Executors.newFixedThreadPool(2)
        try {
            val futures = listOf(first, second).map { root ->
                executor.submit {
                    compileStubs(
                        root = root,
                        paths = paths,
                        runner = runner,
                        args = args,
                        quiet = true,
                    )
                }
            }
            for (future in futures) {
                future.get(5, TimeUnit.SECONDS)
            }
        } finally {
            executor.shutdownNow()
        }

        runner.stubCommands.get() shouldBe 1
        runner.projectCommands.get() shouldBe 2
    }

    test("in-process compiler arguments resolve relative paths against compiler cwd") {
        val cwd = Files.createTempDirectory("compile-cwd")
        val normalized = normalizeCompilerArgsForInProcess(
            listOf(
                "-classpath",
                listOf("stubs/classes", "api.jar").joinToString(File.pathSeparator),
                "-d",
                "out/classes",
                "@sources.txt",
            ),
            cwd,
        )

        normalized shouldBe listOf(
            "-classpath",
            listOf(cwd.resolve("stubs/classes"), cwd.resolve("api.jar")).joinToString(File.pathSeparator) { it.normalize().pathString },
            "-d",
            cwd.resolve("out/classes").normalize().pathString,
            "@${cwd.resolve("sources.txt").normalize().pathString}",
        )
    }
})

private fun newCompileProject(name: String): Path {
    val root = Files.createTempDirectory(name)
    root.resolve("game.jar").writeBytes(byteArrayOf(0))
    root.resolve("j2me.toml").writeText("jar = \"game.jar\"\n")
    root.resolve("decompiled").createDirectories().resolve("Game.java").writeText("class Game {}\n")
    return root
}

private class QueueProcessRunner(vararg results: CommandResult) : ProcessRunner {
    private val results = ArrayDeque(results.toList())
    val commands = mutableListOf<List<String>>()

    override fun run(
        cmd: List<String>,
        okReturnCodes: Set<Int>,
        emitOutputOnAllowedNonZero: Boolean,
        cwd: Path?,
        logStdoutPath: Path?,
        logStderrPath: Path?,
    ): CommandResult {
        commands += cmd
        val result = results.removeFirst()
        return result.copy(command = cmd)
    }
}

private class CountingProcessRunner : ProcessRunner {
    val stubCommands = AtomicInteger()
    val projectCommands = AtomicInteger()
    private val firstStubEntered = CountDownLatch(1)

    override fun run(
        cmd: List<String>,
        okReturnCodes: Set<Int>,
        emitOutputOnAllowedNonZero: Boolean,
        cwd: Path?,
        logStdoutPath: Path?,
        logStderrPath: Path?,
    ): CommandResult {
        val text = cmd.joinToString(" ")
        if ("stub_sources.txt" in text) {
            stubCommands.incrementAndGet()
            firstStubEntered.countDown()
            Thread.sleep(100)
        } else if ("project_sources.txt" in text) {
            firstStubEntered.await(1, TimeUnit.SECONDS)
            projectCommands.incrementAndGet()
        }
        return CommandResult(cmd, 0, "", "")
    }
}
