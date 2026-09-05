package j2me.cli

import j2me.process.ProcessRunner
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.isDirectory
import kotlin.io.path.isRegularFile
import kotlin.io.path.name
import kotlin.io.path.nameWithoutExtension
import kotlin.io.path.pathString
import kotlin.io.path.readText
import kotlin.io.path.relativeTo
import kotlin.io.path.writeText

internal data class FullrunHistoryResult(
    val historyDir: Path,
    val mode: FullrunHistoryMode,
    val changed: Boolean,
    val commitHash: String?,
    val diffStat: String,
    val nameStatus: List<String>,
    val regressions: List<String>,
    val fixes: List<String>,
)

private data class HistoryProjectRow(
    val projectKey: String,
    val projectPath: String,
    val project: String,
    val remapStatus: String,
    val compileStatus: String,
    val sources: String,
    val errors: String,
    val warnings: String,
)

private const val maxChangedFilesToDisplay = 80
private val temporaryFullrunPathRegex = Regex("/tmp/j2me-fullrun-[^\\s:]+")

internal fun updateFullrunHistory(
    root: Path,
    historyDir: Path,
    mode: FullrunHistoryMode,
    fullSelection: Boolean,
    results: List<FullrunProjectResult>,
    runner: ProcessRunner,
): FullrunHistoryResult {
    historyDir.createDirectories()
    ensureHistoryRepo(historyDir, runner)

    val sourcesDir = historyDir.resolve("sources")
    val statusDir = historyDir.resolve("status")
    val diagnosticsDir = statusDir.resolve("diagnostics")
    sourcesDir.createDirectories()
    diagnosticsDir.createDirectories()

    val selectedKeys = results.map { it.projectKey }.toSet()
    val previousRows = readProjectRows(statusDir.resolve("projects.tsv"))
    val rows = if (fullSelection) linkedMapOf() else previousRows.toMutableMap()

    for (result in results) {
        val sourceCount = syncTrackedSources(
            sourceDir = result.decompiledDir,
            targetDir = sourcesDir.resolve(result.projectKey),
            enabled = result.remapStatus == "PASS",
        )
        writeTrackedDiagnostics(
            root = root,
            result = result,
            diagnosticsPath = diagnosticsDir.resolve("${result.projectKey}.txt"),
        )
        rows[result.projectKey] = HistoryProjectRow(
            projectKey = result.projectKey,
            projectPath = result.projectPath,
            project = result.project,
            remapStatus = result.remapStatus,
            compileStatus = result.compileStatus,
            sources = sourceCount.toString(),
            errors = result.errors,
            warnings = result.warnings,
        )
    }

    if (fullSelection) {
        pruneUnselectedSources(sourcesDir, selectedKeys)
        pruneUnselectedDiagnostics(diagnosticsDir, selectedKeys)
    }

    writeProjectRows(statusDir.resolve("projects.tsv"), rows.values.sortedWith(compareBy({ it.projectPath }, { it.projectKey })))
    writeHistoryGitignore(historyDir.resolve(".gitignore"))

    val regressions = statusTransitions(previousRows, rows, selectedKeys, expectRegression = true)
    val fixes = statusTransitions(previousRows, rows, selectedKeys, expectRegression = false)

    runner.run(listOf("git", "add", "-A", ".gitignore", "sources", "status"), cwd = historyDir)
    val changed = runner.run(
        listOf("git", "diff", "--cached", "--quiet", "--", "sources", "status", ".gitignore"),
        okReturnCodes = setOf(0, 1),
        cwd = historyDir,
    ).returnCode == 1

    val diffStat = if (changed) {
        runner.run(
            listOf("git", "diff", "--cached", "--stat", "--stat-count=80", "--", "sources", "status", ".gitignore"),
            cwd = historyDir,
        ).stdout.trimEnd()
    } else {
        ""
    }
    val nameStatus = if (changed) {
        runner.run(
            listOf("git", "diff", "--cached", "--name-status", "--", "sources", "status", ".gitignore"),
            cwd = historyDir,
        ).stdout
            .lineSequence()
            .filter { it.isNotBlank() }
            .take(maxChangedFilesToDisplay + 1)
            .toList()
    } else {
        emptyList()
    }

    val commitHash = if (changed && mode == FullrunHistoryMode.COMMIT) {
        val passes = rows.values.count { it.remapStatus == "PASS" && it.compileStatus == "PASS" }
        val failures = rows.values.count { it.remapStatus != "PASS" || it.compileStatus == "FAIL" }
        runner.run(
            listOf(
                "git",
                "-c",
                "user.name=J2ME Fullrun",
                "-c",
                "user.email=j2me-fullrun@localhost",
                "commit",
                "-m",
                "fullrun: $passes pass, $failures fail",
            ),
            cwd = historyDir,
        )
        runner.run(listOf("git", "rev-parse", "--short", "HEAD"), cwd = historyDir).stdout.trim()
    } else {
        null
    }

    return FullrunHistoryResult(
        historyDir = historyDir,
        mode = mode,
        changed = changed,
        commitHash = commitHash,
        diffStat = diffStat,
        nameStatus = nameStatus,
        regressions = regressions,
        fixes = fixes,
    )
}

internal fun printFullrunHistoryResult(result: FullrunHistoryResult) {
    println("history: ${result.historyDir}")
    if (!result.changed) {
        println("history: no source/status changes")
        return
    }

    if (result.regressions.isNotEmpty()) {
        println("history regressions:")
        result.regressions.take(20).forEach { println("  $it") }
    }
    if (result.fixes.isNotEmpty()) {
        println("history fixes:")
        result.fixes.take(20).forEach { println("  $it") }
    }
    if (result.diffStat.isNotBlank()) {
        println("history diff stat:")
        println(result.diffStat)
    }
    if (result.nameStatus.isNotEmpty()) {
        println("history changed files:")
        result.nameStatus.take(maxChangedFilesToDisplay).forEach { println("  $it") }
        if (result.nameStatus.size > maxChangedFilesToDisplay) {
            println("  ...")
        }
    }

    when {
        result.commitHash != null -> {
            println("history commit: ${result.commitHash}")
            println("history inspect: git -C ${result.historyDir} show --stat HEAD")
        }
        result.mode == FullrunHistoryMode.SNAPSHOT -> {
            println("history snapshot staged; inspect with: git -C ${result.historyDir} diff --cached")
        }
    }
}

private fun ensureHistoryRepo(historyDir: Path, runner: ProcessRunner) {
    if (!historyDir.resolve(".git").exists()) {
        runner.run(listOf("git", "init"), cwd = historyDir)
    }
}

private fun writeHistoryGitignore(path: Path) {
    path.writeText(
        listOf(
            "artifacts/",
            "logs/",
            "reports/",
            "work/",
            "",
        ).joinToString("\n"),
    )
}

private fun syncTrackedSources(sourceDir: Path, targetDir: Path, enabled: Boolean): Int {
    deleteRecursivelyIfExists(targetDir)
    if (!enabled || !sourceDir.isDirectory()) {
        return 0
    }

    val javaFiles = Files.walk(sourceDir).use { walk ->
        walk
            .filter { it.isRegularFile() && it.fileName.toString().endsWith(".java") }
            .sorted()
            .toList()
    }
    for (source in javaFiles) {
        val relative = source.relativeTo(sourceDir)
        val target = targetDir.resolve(relative)
        target.parent?.createDirectories()
        target.writeText(normalizeTrackedText(source.readText()))
    }
    return javaFiles.size
}

private fun writeTrackedDiagnostics(root: Path, result: FullrunProjectResult, diagnosticsPath: Path) {
    val diagnostic = when {
        result.compileStatus == "FAIL" -> {
            val errorsByMessage = result.compileOutDir.resolve("errors_by_message.txt")
            if (errorsByMessage.isRegularFile()) {
                errorsByMessage.readText()
            } else {
                "COMPILE_FAIL\n${result.notes}\n"
            }
        }
        result.remapStatus != "PASS" -> "REMAP_FAIL\n${result.notes}\n"
        else -> ""
    }

    val normalized = normalizeDiagnosticText(diagnostic, root, result)
    if (normalized.isBlank()) {
        Files.deleteIfExists(diagnosticsPath)
        return
    }
    diagnosticsPath.parent?.createDirectories()
    diagnosticsPath.writeText(normalized)
}

private fun normalizeDiagnosticText(text: String, root: Path, result: FullrunProjectResult): String {
    var out = normalizeTrackedText(text)
    val replacements = listOfNotNull(
        root.pathString to "<root>",
        result.projectDir.pathString to "<project>",
        result.decompiledDir.pathString to "<decompiled>",
        result.compileOutDir.pathString to "<compile_out>",
        result.workDir?.pathString?.let { it to "<work>" },
    )
    for ((from, to) in replacements.sortedByDescending { it.first.length }) {
        out = out.replace(from, to)
    }
    return out.replace(temporaryFullrunPathRegex, "<tmp-fullrun>")
}

private fun normalizeTrackedText(text: String): String {
    val lines = text
        .replace("\r\n", "\n")
        .replace('\r', '\n')
        .split('\n')
        .map { it.trimEnd() }
        .dropLastWhile { it.isEmpty() }
    return if (lines.isEmpty()) "" else lines.joinToString("\n") + "\n"
}

private fun pruneUnselectedSources(sourcesDir: Path, selectedKeys: Set<String>) {
    if (!sourcesDir.isDirectory()) {
        return
    }
    Files.list(sourcesDir).use { stream ->
        stream
            .filter { it.name !in selectedKeys }
            .forEach { deleteRecursivelyIfExists(it) }
    }
}

private fun pruneUnselectedDiagnostics(diagnosticsDir: Path, selectedKeys: Set<String>) {
    if (!diagnosticsDir.isDirectory()) {
        return
    }
    Files.list(diagnosticsDir).use { stream ->
        stream
            .filter { it.nameWithoutExtension !in selectedKeys }
            .forEach { Files.deleteIfExists(it) }
    }
}

private fun readProjectRows(path: Path): LinkedHashMap<String, HistoryProjectRow> {
    if (!path.isRegularFile()) {
        return linkedMapOf()
    }
    return path.readText()
        .lineSequence()
        .drop(1)
        .filter { it.isNotBlank() }
        .mapNotNull { line ->
            val parts = line.split('\t')
            if (parts.size < 8) {
                null
            } else {
                HistoryProjectRow(
                    projectKey = parts[0],
                    projectPath = parts[1],
                    project = parts[2],
                    remapStatus = parts[3],
                    compileStatus = parts[4],
                    sources = parts[5],
                    errors = parts[6],
                    warnings = parts[7],
                )
            }
        }
        .associateByTo(linkedMapOf()) { it.projectKey }
}

private fun writeProjectRows(path: Path, rows: List<HistoryProjectRow>) {
    path.parent?.createDirectories()
    path.writeText(
        buildString {
            appendLine("project_key\tproject_path\tproject\tremap\tcompile\tsources\terrors\twarnings")
            rows.forEach { row ->
                appendLine(
                    listOf(
                        row.projectKey,
                        row.projectPath,
                        row.project,
                        row.remapStatus,
                        row.compileStatus,
                        row.sources,
                        row.errors,
                        row.warnings,
                    ).joinToString("\t") { tsv(it) },
                )
            }
        },
    )
}

private fun statusTransitions(
    previousRows: Map<String, HistoryProjectRow>,
    currentRows: Map<String, HistoryProjectRow>,
    selectedKeys: Set<String>,
    expectRegression: Boolean,
): List<String> {
    return selectedKeys.sorted().mapNotNull { key ->
        val old = previousRows[key] ?: return@mapNotNull null
        val current = currentRows[key] ?: return@mapNotNull null
        val oldPass = old.remapStatus == "PASS" && old.compileStatus == "PASS"
        val newPass = current.remapStatus == "PASS" && current.compileStatus == "PASS"
        when {
            expectRegression && oldPass && !newPass -> "${current.projectPath}: PASS -> ${current.remapStatus}/${current.compileStatus}"
            !expectRegression && !oldPass && newPass -> "${current.projectPath}: ${old.remapStatus}/${old.compileStatus} -> PASS"
            else -> null
        }
    }
}

private fun tsv(value: String): String =
    value.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ')
