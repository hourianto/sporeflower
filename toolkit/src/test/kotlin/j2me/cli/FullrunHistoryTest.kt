package j2me.cli

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.booleans.shouldBeTrue
import io.kotest.matchers.collections.shouldContain
import io.kotest.matchers.nulls.shouldNotBeNull
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import io.kotest.matchers.string.shouldNotContain
import j2me.process.RealProcessRunner
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.readText
import kotlin.io.path.writeText

class FullrunHistoryTest : FunSpec({
    test("history commits normalized source diffs for projects that still pass") {
        val root = Files.createTempDirectory("fullrun-history")
        val historyDir = root.resolve("fullruns/history")
        val project = root.resolve("demo").createDirectories()
        val decompiled = project.resolve("decompiled/com/game").createDirectories()
        val compileOut = project.resolve("compile-check").createDirectories()
        val runner = RealProcessRunner()

        decompiled.resolve("Main.java").writeText("class Main {  \r\n  int value() { return 1; }   \r\n}\r\n")
        val first = updateFullrunHistory(
            root = root,
            historyDir = historyDir,
            mode = FullrunHistoryMode.COMMIT,
            fullSelection = true,
            results = listOf(historyResult(project, project.resolve("decompiled"), compileOut)),
            runner = runner,
        )

        first.changed.shouldBeTrue()
        first.commitHash.shouldNotBeNull()
        historyDir.resolve("sources/demo-key/com/game/Main.java").readText() shouldBe
            "class Main {\n  int value() { return 1; }\n}\n"

        decompiled.resolve("Main.java").writeText("class Main {\n  int value() { return 2; }\n}\n")
        val second = updateFullrunHistory(
            root = root,
            historyDir = historyDir,
            mode = FullrunHistoryMode.COMMIT,
            fullSelection = true,
            results = listOf(historyResult(project, project.resolve("decompiled"), compileOut)),
            runner = runner,
        )

        second.changed.shouldBeTrue()
        second.commitHash.shouldNotBeNull()
        second.nameStatus shouldContain "M\tsources/demo-key/com/game/Main.java"

        val patch = runner.run(
            listOf("git", "show", "--format=", "--", "sources/demo-key/com/game/Main.java"),
            cwd = historyDir,
        ).stdout
        patch shouldContain "-  int value() { return 1; }"
        patch shouldContain "+  int value() { return 2; }"

        val failedRemap = historyResult(project, project.resolve("decompiled"), compileOut).copy(
            remap = StageResult.run<Unit> { error("decompilation failed") },
            compile = StageResult.skipped(),
        )
        val regression = updateFullrunHistory(root, historyDir, FullrunHistoryMode.SNAPSHOT, true, listOf(failedRemap), runner)
        regression.regressions shouldBe listOf("demo: PASS -> FAIL/SKIPPED")

        val uncompiled = updateFullrunHistory(
            root, historyDir, FullrunHistoryMode.SNAPSHOT, true,
            listOf(historyResult(project, project.resolve("decompiled"), compileOut, compileStatus = "SKIPPED")), runner,
        )
        uncompiled.regressions shouldBe emptyList()
        uncompiled.fixes shouldBe emptyList()
    }

    test("history stores normalized diagnostics without temp or absolute paths") {
        val root = Files.createTempDirectory("fullrun-history-diagnostics")
        val historyDir = root.resolve("fullruns/history")
        val project = root.resolve("demo").createDirectories()
        val decompiled = project.resolve("decompiled").createDirectories()
        val compileOut = project.resolve("compile-check").createDirectories()
        compileOut.resolve("errors_by_message.txt").writeText("stale diagnostics from an older run\n")

        updateFullrunHistory(
            root = root,
            historyDir = historyDir,
            mode = FullrunHistoryMode.COMMIT,
            fullSelection = true,
            results = listOf(
                historyResult(
                    project = project,
                    decompiledDir = decompiled,
                    compileOut = compileOut,
                    compileStatus = "FAIL",
                    errors = listOf(CompilerDiagnosticError("error",
                        message = "${project.resolve("decompiled/Main.java")}: cannot find symbol in /tmp/j2me-fullrun-noise/work")),
                ),
            ),
            runner = RealProcessRunner(),
        )

        val diagnostics = historyDir.resolve("status/diagnostics/demo-key.txt").readText()
        diagnostics shouldContain "<decompiled>/Main.java"
        diagnostics shouldContain "<tmp-fullrun>"
        diagnostics shouldNotContain root.toString()
        diagnostics shouldNotContain "/tmp/j2me-fullrun-noise"
        diagnostics shouldNotContain "stale diagnostics"
    }
})

private fun historyResult(
    project: Path,
    decompiledDir: Path,
    compileOut: Path,
    compileStatus: String = "PASS",
    errors: List<CompilerDiagnosticError> = emptyList(),
): FullrunProjectResult =
    FullrunProjectResult(
        project = "demo",
        projectKey = "demo-key",
        projectPath = "demo",
        projectDir = project,
        remap = StageResult.run { Unit },
        compile = if (compileStatus == "SKIPPED") StageResult.skipped() else StageResult.run(failureMessage = CompileResult::failureMessage) {
            CompileResult(1, CompilerDiagnostics(errors, 0),
                if (compileStatus == "FAIL") "failure" else null)
        },
        logPath = project.resolve("demo.log"),
        decompiledDir = decompiledDir,
        compileOutDir = compileOut,
        workDir = null,
    )
