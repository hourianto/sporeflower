package j2me.cli

import com.github.ajalt.clikt.core.CliktCommand
import com.github.ajalt.clikt.core.Context
import com.github.ajalt.clikt.parameters.options.default
import com.github.ajalt.clikt.parameters.options.check
import com.github.ajalt.clikt.parameters.options.flag
import com.github.ajalt.clikt.parameters.options.multiple
import com.github.ajalt.clikt.parameters.options.option
import com.github.ajalt.clikt.parameters.types.int
import j2me.process.ProcessRunner
import j2me.symbols.AnalysisCachePaths
import java.io.PrintWriter
import java.io.StringWriter
import java.nio.file.Files
import java.nio.file.Path
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.concurrent.CompletionService
import java.util.concurrent.ExecutorCompletionService
import java.util.concurrent.Executors
import kotlin.io.path.Path
import kotlin.io.path.absolute
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.isDirectory
import kotlin.io.path.isRegularFile
import kotlin.io.path.name
import kotlin.io.path.pathString
import kotlin.io.path.relativeTo
import kotlin.io.path.writeText
import kotlin.math.max
import kotlin.math.min
import kotlin.system.measureTimeMillis

private enum class DecompilerMode {
    AUTO,
    IN_PROCESS,
    PROCESS;

    companion object {
        fun parse(value: String): DecompilerMode = when (value.lowercase()) {
            "auto" -> AUTO
            "in-process", "inprocess" -> IN_PROCESS
            "process" -> PROCESS
            else -> throw IllegalArgumentException("Unsupported decompiler mode: $value")
        }
    }
}

internal enum class FullrunHistoryMode {
    OFF,
    SNAPSHOT,
    COMMIT;

    companion object {
        fun parse(value: String): FullrunHistoryMode = when (value.lowercase()) {
            "off", "none" -> OFF
            "snapshot", "stage", "staged" -> SNAPSHOT
            "commit" -> COMMIT
            else -> throw IllegalArgumentException("Unsupported history mode: $value")
        }
    }
}

internal enum class FullrunKeepWork {
    NONE,
    FAILURES,
    ALL;

    companion object {
        fun parse(value: String): FullrunKeepWork = when (value.lowercase()) {
            "none" -> NONE
            "failures", "failure", "failed" -> FAILURES
            "all" -> ALL
            else -> throw IllegalArgumentException("Unsupported keep-work mode: $value")
        }
    }
}

class FullrunCommand(
    private val paths: ToolkitPaths,
    private val runner: ProcessRunner,
) : CliktCommand(name = "fullrun") {
    override fun help(context: Context): String = "Run raw remap + compile-stubs over many projects in one j2me process"

    private val rootArg by option("--root", help = "Root containing project directories.").default(System.getenv("J2ME_CORPUS") ?: ".")
    private val reportArg by option("--report", help = "Markdown report path.")
    private val logsArg by option("--logs", help = "Directory for fullrun orchestration logs.")
    private val limit by option("--limit", help = "Only run the first N selected projects.").int().default(0)
        .check("must be non-negative") { it >= 0 }
    private val projects by option("--project", help = "Project name or directory to include. Can be passed more than once.").multiple()
    private val jobs by option("--jobs", help = "Outer project jobs.").int().default(defaultFullrunJobs())
        .check("must be positive") { it > 0 }
    private val decompilerMode by option("--decompiler-mode", help = "auto, in-process, or process.").default("auto")
    private val decompilerThreads by option("--decompiler-threads", help = "Override Sporeflower --thread-count for each project. 0 keeps Sporeflower default.").int().default(defaultFullrunDecompilerThreads())
        .check("must be non-negative") { it >= 0 }
    private val noCompile by option("--no-compile", help = "Run remap only.").flag(default = false)
    private val noComments by option("--no-comments", help = "Disable decompiler comments.").flag(default = false)
    private val inPlace by option("--in-place", help = "Write decompiled/out/compile-check outputs into each project instead of the fullrun scratch workspace.").flag(default = false)
    private val keepWork by option("--keep-work", help = "Scratch workspace retention: failures, none, or all. Ignored with --in-place.").default("failures")
    private val historyMode by option("--history-mode", help = "Git source snapshot tracking: snapshot, commit, or off.").default("snapshot")
    private val historyDirArg by option("--history-dir", help = "Git history repo for normalized decompiled source snapshots.")

    override fun run() {
        val root = Path(rootArg).absolute().normalize()
        val selectedProjects = selectProjects(root, projects, limit)
        require(selectedProjects.isNotEmpty()) { "No projects found under $root" }
        validateUniqueProjectKeys(root, selectedProjects)

        val started = ZonedDateTime.now()
        val fullrunsRoot = root.resolve("fullruns").absolute().normalize()
        fullrunsRoot.createDirectories()
        val runName = uniqueFullrunName(fullrunsRoot, timestampForFullrunName(started), reportArg == null, logsArg == null)
        val logRoot = (logsArg?.let(::Path) ?: fullrunsRoot.resolve(runName)).absolute().normalize()
        logRoot.createDirectories()
        val workRoot = logRoot.resolve("work").absolute().normalize().takeUnless { inPlace }
        workRoot?.createDirectories()
        val report = (reportArg?.let(::Path) ?: fullrunsRoot.resolve("$runName.md")).absolute().normalize()
        val global = loadToml(paths.globalCfg)
        val parsedHistoryMode = FullrunHistoryMode.parse(historyMode)
        val parsedKeepWork = FullrunKeepWork.parse(keepWork)
        val historyDir = (historyDirArg?.let(::Path) ?: fullrunsRoot.resolve("history")).absolute().normalize()
        val fullSelection = projects.isEmpty() && limit <= 0

        val mode = DecompilerMode.parse(decompilerMode)
        val processVineflower = ProcessVineflowerRunner(runner)
        val vineflowerRunner = when (mode) {
            DecompilerMode.PROCESS -> processVineflower
            DecompilerMode.IN_PROCESS -> InProcessVineflowerRunner()
            DecompilerMode.AUTO -> InProcessVineflowerRunner(fallback = processVineflower)
        }
        val compilerRunner = InProcessCompilerRunner(runner)

        val ordered = selectedProjects.sortedByDescending { projectWeight(it) }
        println("fullrun: projects=${ordered.size} jobs=$jobs decompiler=$mode root=$root")
        println("logs: $logRoot")

        val executor = Executors.newFixedThreadPool(max(1, jobs))
        val completion: CompletionService<FullrunProjectResult> = ExecutorCompletionService(executor)
        val startedNs = System.nanoTime()
        try {
            for (project in ordered) {
                completion.submit {
                    runProject(
                        root = root,
                        projectDir = project,
                        paths = paths,
                        global = global,
                        logRoot = logRoot,
                        workRoot = workRoot,
                        vineflowerRunner = vineflowerRunner,
                        compilerRunner = compilerRunner,
                        noCompile = noCompile,
                        noComments = noComments,
                        decompilerThreads = decompilerThreads,
                    )
                }
            }

            val results = mutableListOf<FullrunProjectResult>()
            repeat(ordered.size) {
                val result = completion.take().get()
                results += result
                println("[${result.project}] remap=${result.remapStatus} ${result.remapMs}ms compile=${result.compileStatus} ${result.compileMs}ms")
            }

            val elapsedMs = (System.nanoTime() - startedNs) / 1_000_000
            val sortedResults = results.sortedBy { it.projectPath }
            writeFullrunReport(
                report = report,
                root = root,
                logRoot = logRoot,
                started = started,
                finished = ZonedDateTime.now(),
                elapsedMs = elapsedMs,
                jobs = jobs,
                mode = mode,
                decompilerThreads = decompilerThreads,
                workRoot = workRoot,
                keepWork = parsedKeepWork,
                historyDir = historyDir.takeUnless { parsedHistoryMode == FullrunHistoryMode.OFF },
                results = sortedResults,
            )

            val passes = results.count { it.compileStatus == "PASS" || (noCompile && it.remapStatus == "PASS") }
            val failures = results.size - passes

            if (parsedHistoryMode != FullrunHistoryMode.OFF) {
                val history = updateFullrunHistory(
                    root = root,
                    historyDir = historyDir,
                    mode = parsedHistoryMode,
                    fullSelection = fullSelection,
                    results = sortedResults,
                    runner = runner,
                )
                printFullrunHistoryResult(history)
            }

            cleanupFullrunWork(sortedResults, parsedKeepWork)

            println("report: $report")
            println("passes: $passes")
            println("failures: $failures")
            if (failures > 0) {
                throw IllegalStateException("fullrun completed with $failures failure(s)")
            }
        } finally {
            executor.shutdownNow()
        }
    }
}

internal data class FullrunProjectResult(
    val project: String,
    val projectKey: String,
    val projectPath: String,
    val projectDir: Path,
    val remapStatus: String,
    val remapMs: Long,
    val compileStatus: String,
    val compileMs: Long,
    val sources: String,
    val errors: String,
    val warnings: String,
    val logPath: Path,
    val decompiledDir: Path,
    val compileOutDir: Path,
    val workDir: Path?,
    val notes: String,
)

internal data class FullrunWorkspace(
    val projectKey: String,
    val projectPath: String,
    val logPath: Path,
    val workDir: Path?,
    val remapOutDir: Path,
    val decompiledDir: Path,
    val compileOutDir: Path,
    val cacheDir: Path,
)

private fun defaultFullrunJobs(): Int =
    min(3, max(1, Runtime.getRuntime().availableProcessors()))

private fun defaultFullrunDecompilerThreads(): Int =
    min(2, max(1, Runtime.getRuntime().availableProcessors()))

private fun timestampForFullrunName(time: ZonedDateTime): String =
    "fullrun_" + DateTimeFormatter.ofPattern("HH-mm_dd-MMM", Locale.US)
        .format(time)
        .lowercase(Locale.US)

private fun uniqueFullrunName(root: Path, baseName: String, checkReport: Boolean, checkLogRoot: Boolean): String {
    var candidate = baseName
    var suffix = 2
    while ((checkReport && root.resolve("$candidate.md").exists()) || (checkLogRoot && root.resolve(candidate).exists())) {
        candidate = "${baseName}_$suffix"
        suffix += 1
    }
    return candidate
}

private fun selectProjects(root: Path, requested: List<String>, limit: Int): List<Path> {
    val projects = if (requested.isNotEmpty()) {
        requested.map { raw ->
            val path = Path(raw)
            if (path.isAbsolute) path.normalize() else root.resolve(raw).normalize()
        }
    } else {
        Files.list(root).use { stream ->
            stream
                .filter { it.isDirectory() && it.resolve("j2me.toml").isRegularFile() }
                .sorted()
                .toList()
        }
    }
    val existing = projects.filter { it.isDirectory() && it.resolve("j2me.toml").isRegularFile() }
    return if (limit > 0) existing.take(limit) else existing
}

private fun projectWeight(projectDir: Path): Long {
    val jar = runCatching { resolveProjectJar(projectDir) }.getOrNull() ?: return 0
    val jarSize = runCatching { Files.size(jar) }.getOrDefault(0L)
    val classCount = loadProjectBytecodeConfig(projectDir)?.classCount ?: 0
    return jarSize + classCount * 64_000L
}

private fun runProject(
    root: Path,
    projectDir: Path,
    paths: ToolkitPaths,
    global: org.tomlj.TomlParseResult?,
    logRoot: Path,
    workRoot: Path?,
    vineflowerRunner: VineflowerRunner,
    compilerRunner: ProcessRunner,
    noCompile: Boolean,
    noComments: Boolean,
    decompilerThreads: Int,
): FullrunProjectResult {
    val project = projectDir.name
    val workspace = fullrunWorkspace(root, projectDir, logRoot, workRoot)
    val logPath = workspace.logPath
    val log = StringBuilder()

    fun append(line: String) {
        log.appendLine(line)
    }

    var remapStatus = "SKIPPED"
    var remapMs = 0L
    var compileStatus = "SKIPPED"
    var compileMs = 0L
    var compileAttempted = false
    var notes = ""

    try {
        val jar = resolveProjectJar(projectDir)
        append("project=$project")
        append("root=$projectDir")
        append("jar=$jar")
        append("output_mode=${if (workspace.workDir == null) "in-place" else "scratch"}")
        workspace.workDir?.let { append("workspace=$it") }
        append("remap_out=${workspace.remapOutDir}")
        append("decompiled=${workspace.decompiledDir}")
        append("compile_out=${workspace.compileOutDir}")

        remapMs = measureTimeMillis {
            val args = buildRemapPipelineArgs(
                root = projectDir,
                paths = paths,
                global = global,
                jar = jar,
                raw = true,
                noComments = noComments,
            ).let { built ->
                val extraOptions = built.extraVineflowerOptions.toMutableMap()
                extraOptions["log-level"] = "error"
                if (decompilerThreads > 0) {
                    extraOptions["thread-count"] = decompilerThreads.toString()
                }
                built.forFullrunWorkspace(workspace).copy(extraVineflowerOptions = extraOptions)
            }
            val result = runRemapPipeline(args, compilerRunner, vineflowerRunner, quiet = true)
            append("remap_vineflower_ms=${result.vineflowerWaitMs}")
            append("decompiled_files=${result.decompiledFileCount ?: ""}")
        }
        remapStatus = "PASS"
    } catch (exc: Throwable) {
        remapStatus = "FAIL"
        notes = exc.message.orEmpty()
        append("remap_exception:")
        append(stackTrace(exc))
    }

    if (remapStatus == "PASS" && !noCompile) {
        compileAttempted = true
        try {
            compileMs = measureTimeMillis {
                compileStubs(
                    root = projectDir,
                    paths = paths,
                    runner = compilerRunner,
                    args = CompileStubsArgs(
                        decompiledSrcArg = workspace.decompiledDir.pathString,
                        outDirArg = workspace.compileOutDir.pathString,
                    ),
                    quiet = true,
                )
            }
            compileStatus = "PASS"
        } catch (exc: Throwable) {
            compileStatus = "FAIL"
            if (notes.isBlank()) {
                notes = exc.message.orEmpty()
            }
            append("compile_exception:")
            append(stackTrace(exc))
        }
    }

    val compileSummary = if (compileAttempted) readCompileSummary(workspace.compileOutDir) else CompileSummary()
    append("remap_status=$remapStatus")
    append("remap_ms=$remapMs")
    append("compile_status=$compileStatus")
    append("compile_ms=$compileMs")
    append("sources=${compileSummary.sources}")
    append("errors=${compileSummary.errors}")
    append("warnings=${compileSummary.warnings}")
    logPath.writeText(log.toString())

    return FullrunProjectResult(
        project = project,
        projectKey = workspace.projectKey,
        projectPath = workspace.projectPath,
        projectDir = projectDir,
        remapStatus = remapStatus,
        remapMs = remapMs,
        compileStatus = compileStatus,
        compileMs = compileMs,
        sources = compileSummary.sources,
        errors = compileSummary.errors,
        warnings = compileSummary.warnings,
        logPath = logPath,
        decompiledDir = workspace.decompiledDir,
        compileOutDir = workspace.compileOutDir,
        workDir = workspace.workDir,
        notes = notes.lineSequence().firstOrNull()?.take(160).orEmpty(),
    )
}

private fun fullrunWorkspace(root: Path, projectDir: Path, logRoot: Path, workRoot: Path?): FullrunWorkspace {
    val projectPath = fullrunProjectPath(root, projectDir)
    val key = fullrunProjectKey(projectPath)
    val workDir = workRoot?.resolve(key)
    return FullrunWorkspace(
        projectKey = key,
        projectPath = projectPath,
        logPath = logRoot.resolve("$key.log"),
        workDir = workDir,
        remapOutDir = workDir?.resolve("out") ?: projectDir.resolve("out"),
        decompiledDir = workDir?.resolve("decompiled") ?: projectDir.resolve("decompiled"),
        compileOutDir = workDir?.resolve("out/compile_check") ?: projectDir.resolve("out/compile_check"),
        cacheDir = workDir?.resolve(".cache") ?: projectDir.resolve(".cache"),
    )
}

private fun fullrunProjectPath(root: Path, projectDir: Path): String {
    val absoluteRoot = root.absolute().normalize()
    val absoluteProject = projectDir.absolute().normalize()
    return runCatching { absoluteProject.relativeTo(absoluteRoot).pathString }
        .getOrDefault(projectDir.name)
        .replace('\\', '/')
}

private fun validateUniqueProjectKeys(root: Path, projects: List<Path>) {
    val seen = linkedMapOf<String, Path>()
    for (project in projects) {
        val key = fullrunProjectKey(fullrunProjectPath(root, project))
        val previous = seen.putIfAbsent(key, project)
        require(previous == null) {
            "Fullrun project key collision for '$key': $previous and $project"
        }
    }
}

private fun fullrunProjectKey(projectPath: String): String {
    val readable = safeName(projectPath.replace('/', '_'))
    return readable.ifBlank { "project" }
}

private fun RemapPipelineArgs.forFullrunWorkspace(workspace: FullrunWorkspace): RemapPipelineArgs {
    if (workspace.workDir == null) {
        return this
    }

    return copy(
        outDir = workspace.remapOutDir,
        overwriteOutputDir = true,
        writeIndex = writeIndex?.let { workspace.remapOutDir.resolve(it.fileName) },
        cache = AnalysisCachePaths(
            symbols = workspace.cacheDir.resolve("remap-symbols.json"),
            usage = workspace.cacheDir.resolve("remap-usage.json"),
        ),
        vineflower = vineflower?.copy(output = workspace.decompiledDir),
    )
}

private data class CompileSummary(
    val sources: String = "?",
    val errors: String = "?",
    val warnings: String = "?",
)

private fun readCompileSummary(compileOutDir: Path): CompileSummary {
    val summary = compileOutDir.resolve("summary.txt")
    if (!summary.exists()) {
        return CompileSummary()
    }
    val values = summary.toFile().readLines()
        .mapNotNull { line ->
            val idx = line.indexOf('=')
            if (idx <= 0) null else line.substring(0, idx) to line.substring(idx + 1)
        }
        .toMap()
    return CompileSummary(
        sources = values["project_sources"] ?: "?",
        errors = values["error_lines"] ?: "?",
        warnings = values["warning_lines"] ?: "?",
    )
}

private fun writeFullrunReport(
    report: Path,
    root: Path,
    logRoot: Path,
    started: ZonedDateTime,
    finished: ZonedDateTime,
    elapsedMs: Long,
    jobs: Int,
    mode: DecompilerMode,
    decompilerThreads: Int,
    workRoot: Path?,
    keepWork: FullrunKeepWork,
    historyDir: Path?,
    results: List<FullrunProjectResult>,
) {
    report.parent?.createDirectories()
    val compilePasses = results.count { it.compileStatus == "PASS" }
    val failures = results.count { it.remapStatus != "PASS" || it.compileStatus == "FAIL" }
    report.writeText(
        buildString {
            appendLine("# J2ME Full Run")
            appendLine()
            appendLine("- Root: `$root`")
            appendLine("- Started: `$started`")
            appendLine("- Finished: `$finished`")
            appendLine("- Elapsed: `${elapsedMs}ms`")
            appendLine("- Jobs: `$jobs`")
            appendLine("- Decompiler mode: `$mode`")
            appendLine("- Decompiler threads: `${if (decompilerThreads > 0) decompilerThreads else "default"}`")
            appendLine("- Output mode: `${if (workRoot == null) "in-place" else "scratch"}`")
            workRoot?.let { appendLine("- Workspaces: `$it`") }
            appendLine("- Keep work: `$keepWork`")
            historyDir?.let { appendLine("- History: `$it`") }
            appendLine("- Projects: ${results.size}")
            appendLine("- Compile passes: $compilePasses")
            appendLine("- Failures: $failures")
            appendLine("- Logs: `$logRoot`")
            appendLine()
            appendLine("| Project | Remap | Remap ms | Compile | Compile ms | Sources | Errors | Warnings | Log | Notes |")
            appendLine("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |")
            for (result in results) {
                appendLine(
                    "| `${result.project}` | ${result.remapStatus} | ${result.remapMs} | ${result.compileStatus} | ${result.compileMs} | " +
                        "${result.sources} | ${result.errors} | ${result.warnings} | `${result.logPath}` | ${escapeTable(result.notes)} |",
                )
            }
        },
    )
}

private val unsafeFileNameChars = Regex("[^A-Za-z0-9_.-]")

private fun safeName(value: String): String = value.replace(unsafeFileNameChars, "_")

private fun cleanupFullrunWork(results: List<FullrunProjectResult>, keepWork: FullrunKeepWork) {
    if (keepWork == FullrunKeepWork.ALL) {
        return
    }
    val workRoots = linkedSetOf<Path>()
    for (result in results) {
        val workDir = result.workDir ?: continue
        workDir.parent?.let { workRoots.add(it) }
        val failed = result.remapStatus != "PASS" || result.compileStatus == "FAIL"
        if (keepWork == FullrunKeepWork.NONE || !failed) {
            deleteRecursivelyIfExists(workDir)
        }
    }
    for (workRoot in workRoots) {
        if (workRoot.isDirectory()) {
            val empty = Files.list(workRoot).use { stream -> !stream.findAny().isPresent }
            if (empty) {
                Files.deleteIfExists(workRoot)
            }
        }
    }
}

private fun stackTrace(exc: Throwable): String {
    val out = StringWriter()
    exc.printStackTrace(PrintWriter(out))
    return out.toString()
}

private fun escapeTable(value: String): String =
    value.replace("|", "\\|").replace("\n", " ")
