package j2me.cli

import j2me.process.CommandResult
import j2me.process.ProcessRunner
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.zip.ZipFile
import kotlin.io.path.Path
import kotlin.io.path.absolute
import kotlin.io.path.createDirectories
import kotlin.io.path.isDirectory
import kotlin.io.path.isRegularFile
import kotlin.io.path.pathString
import kotlin.io.path.writeText

private const val stubCacheVersion = 2
private const val ecjMainClass = "org.eclipse.jdt.internal.compiler.batch.Main"
private const val legacyJavacMainClass = "j2me.thirdparty.legacyjavac.Main"
private val managedStubCacheLocks = ConcurrentHashMap<Path, Any>()

internal enum class CompileBackend(val id: String) {
    JAVAC("javac"),
    ECJ("ecj"),
    LEGACY("legacy");

    companion object {
        fun parse(value: String): CompileBackend = when (value.lowercase()) {
            "javac" -> JAVAC
            "ecj" -> ECJ
            "legacy" -> LEGACY
            else -> throw IllegalArgumentException("Unsupported compile-stubs compiler: $value (expected legacy, ecj, or javac)")
        }
    }
}

internal object CompileStubDefaults {
    val backend = CompileBackend.LEGACY
    const val javaRelease = 8
    const val maxCompilerErrors = 100000
    const val javacBin = "javac"
    const val javaBin = "java"
}

private sealed interface JavaCompiler {
    val backend: CompileBackend
    val displayName: String
    val cwd: Path?

    fun cacheInputs(): List<String>

    fun buildCommand(
        apiPath: ApiCompilePath,
        classpathEntries: List<Path>,
        outputDir: Path,
        sourceList: Path,
        maxerrs: Int? = null,
    ): List<String>

    fun summaryFields(): Map<String, String>

    fun diagnostics(stderr: String): CompilerDiagnostics
}

private data class JavacCompiler(
    val javaRelease: Int,
    val javacBin: String,
) : JavaCompiler {
    override val backend = CompileBackend.JAVAC
    override val displayName = "javac"
    override val cwd: Path? = null

    override fun cacheInputs(): List<String> = listOf(
        "java_release=$javaRelease",
        "javac_bin=$javacBin",
    )

    override fun buildCommand(
        apiPath: ApiCompilePath,
        classpathEntries: List<Path>,
        outputDir: Path,
        sourceList: Path,
        maxerrs: Int?,
    ): List<String> {
        val cmd = mutableListOf(javacBin, "--release", javaRelease.toString(), "-Xlint:-options")
        maxerrs?.let {
            cmd += listOf("-Xmaxerrs", it.toString())
        }
        if (classpathEntries.isNotEmpty()) {
            cmd += listOf("-cp", classpathEntries.joinToString(java.io.File.pathSeparator) { it.pathString })
        }
        cmd += listOf("-d", outputDir.pathString, "@${sourceList.pathString}")
        return cmd
    }

    override fun summaryFields(): Map<String, String> = linkedMapOf(
        "java_release" to javaRelease.toString(),
        "javac_bin" to javacBin,
    )

    override fun diagnostics(stderr: String): CompilerDiagnostics = parseJavacDiagnostics(stderr)
}

private data class EcjCompiler(
    val jar: Path,
    val javaBin: String,
    override val cwd: Path,
    val sourceLevel: String = "1.3",
    val targetLevel: String = "1.1",
) : JavaCompiler {
    override val backend = CompileBackend.ECJ
    override val displayName = "ecj"

    override fun cacheInputs(): List<String> = buildList {
        add("source=$sourceLevel")
        add("target=$targetLevel")
        add("java_bin=$javaBin")
        val stat = fileStat(jar)
        add("ecj=$jar:${stat.size}:${stat.mtimeMs}")
    }

    override fun buildCommand(
        apiPath: ApiCompilePath,
        classpathEntries: List<Path>,
        outputDir: Path,
        sourceList: Path,
        maxerrs: Int?,
    ): List<String> {
        val cmd = mutableListOf(
            javaBin,
            "-cp",
            jar.pathString,
            ecjMainClass,
            "-source",
            sourceLevel,
            "-target",
            targetLevel,
            "-nowarn",
            "-bootclasspath",
            apiPath.bootClasspath.joinToString(java.io.File.pathSeparator) { it.pathString },
        )
        maxerrs?.let {
            cmd += listOf("-maxProblems", it.toString())
        }
        if (classpathEntries.isNotEmpty()) {
            cmd += listOf("-classpath", compilerClasspath(classpathEntries, cwd))
        }
        cmd += listOf("-d", outputDir.pathString, "@${sourceList.pathString}")
        return cmd
    }

    override fun summaryFields(): Map<String, String> = linkedMapOf(
        "java_bin" to javaBin,
        "source_level" to sourceLevel,
        "target_level" to targetLevel,
        "compiler_jar" to jar.toString(),
    )

    override fun diagnostics(stderr: String): CompilerDiagnostics = parseEcjDiagnostics(stderr)
}

private data class LegacyJavacCompiler(
    val jar: Path,
    val javaBin: String,
    override val cwd: Path,
    val sourceLevel: String,
    val targetLevel: String,
    val targetSource: String,
) : JavaCompiler {
    override val backend = CompileBackend.LEGACY
    override val displayName = "legacy"

    override fun cacheInputs(): List<String> = buildList {
        add("source=$sourceLevel")
        add("target=$targetLevel")
        add("target_source=$targetSource")
        add("java_bin=$javaBin")
        val stat = fileStat(jar)
        add("legacy_javac=$jar:${stat.size}:${stat.mtimeMs}")
    }

    override fun buildCommand(
        apiPath: ApiCompilePath,
        classpathEntries: List<Path>,
        outputDir: Path,
        sourceList: Path,
        maxerrs: Int?,
    ): List<String> {
        val cmd = mutableListOf(
            javaBin,
            "-cp",
            jar.pathString,
            legacyJavacMainClass,
            "-source",
            sourceLevel,
            "-target",
            targetLevel,
            "-bootclasspath",
            apiPath.bootClasspath.joinToString(java.io.File.pathSeparator) { it.pathString },
        )
        maxerrs?.let {
            cmd += listOf("-Xmaxerrs", it.toString())
        }
        if (classpathEntries.isNotEmpty()) {
            cmd += listOf("-classpath", compilerClasspath(classpathEntries, cwd))
        }
        cmd += listOf("-d", outputDir.pathString, "@${sourceList.pathString}")
        return cmd
    }

    override fun summaryFields(): Map<String, String> = linkedMapOf(
        "java_bin" to javaBin,
        "source_level" to sourceLevel,
        "target_level" to targetLevel,
        "target_source" to targetSource,
        "compiler_jar" to jar.toString(),
    )

    override fun diagnostics(stderr: String): CompilerDiagnostics = parseLegacyJavacDiagnostics(stderr)
}

private data class ApiCompilePath(
    val bootClasspath: List<Path>,
    val classpath: List<Path>,
)

private fun defaultEcjJar(paths: ToolkitPaths): Path =
    paths.base.resolve("vendor/compilers/ecj/ecj.jar").absolute().normalize()

private fun defaultLegacyJavacJar(paths: ToolkitPaths): Path =
    paths.base.resolve("vendor/compilers/legacy-javac/legacy-javac.jar").absolute().normalize()

private fun compilerClasspath(entries: List<Path>, cwd: Path): String =
    entries.joinToString(java.io.File.pathSeparator) { path ->
        if (path.isDirectory()) {
            relativeOrAbsolute(path, cwd)
        } else {
            path.pathString
        }
    }

private data class ResolvedCompileLevels(
    val sourceLevel: String,
    val targetLevel: String,
    val source: String,
)

private fun resolveCompileLevels(
    root: Path,
    projectJar: Path,
    sourceOverride: String?,
    targetOverride: String?,
): ResolvedCompileLevels {
    val bytecode = loadProjectBytecodeConfig(root) ?: bytecodeConfigFromJar(projectJar, source = "inferred")
    return ResolvedCompileLevels(
        sourceLevel = sourceOverride?.trim()?.takeIf { it.isNotBlank() } ?: bytecode.javacSource,
        targetLevel = targetOverride?.trim()?.takeIf { it.isNotBlank() } ?: bytecode.javacTarget,
        source =
            when {
                sourceOverride?.trim()?.isNotBlank() == true || targetOverride?.trim()?.isNotBlank() == true -> "cli"
                else -> bytecode.source
            },
    )
}

private fun resolveCompiler(paths: ToolkitPaths, root: Path, projectJar: Path, args: CompileStubsArgs): JavaCompiler =
    when (args.compiler) {
        CompileBackend.JAVAC -> JavacCompiler(
            javaRelease = args.javaRelease,
            javacBin = args.javacBin,
        )
        CompileBackend.ECJ -> {
            val ecjJar = (args.compilerJarArg?.let(::Path) ?: defaultEcjJar(paths)).absolute().normalize()
            require(ecjJar.isRegularFile()) {
                "ECJ jar not found: $ecjJar\nUse --compiler-jar to point at a compiler jar."
            }
            val levels = resolveCompileLevels(root, projectJar, args.sourceLevel, args.targetLevel)
            EcjCompiler(
                jar = ecjJar,
                javaBin = args.javaBin,
                cwd = paths.base,
                sourceLevel = levels.sourceLevel,
                targetLevel = levels.targetLevel,
            )
        }
        CompileBackend.LEGACY -> {
            val legacyJavacJar = (args.compilerJarArg?.let(::Path) ?: defaultLegacyJavacJar(paths)).absolute().normalize()
            require(legacyJavacJar.isRegularFile()) {
                "Legacy javac jar not found: $legacyJavacJar\nUse --compiler-jar to point at the relocated compiler jar."
            }
            val levels = resolveCompileLevels(root, projectJar, args.sourceLevel, args.targetLevel)
            LegacyJavacCompiler(
                jar = legacyJavacJar,
                javaBin = args.javaBin,
                cwd = paths.base,
                sourceLevel = levels.sourceLevel,
                targetLevel = levels.targetLevel,
                targetSource = levels.source,
            )
        }
    }

// JVM-hosted legacy compilers must see CLDC as the boot class library, otherwise
// they silently compile against the host JDK's java.lang/java.util surface. The
// boot jar is identified by the classes it provides rather than by file names.
private fun selectBootApiJar(apiJars: List<Path>): Path? {
    val candidates = apiJars.mapNotNull { jar ->
        ZipFile(jar.toFile()).use { zip ->
            if (zip.getEntry("java/lang/Object.class") == null) {
                null
            } else {
                val hasPreferredSurface =
                    zip.getEntry("java/lang/Float.class") != null &&
                        zip.getEntry("java/lang/Double.class") != null &&
                        zip.getEntry("java/lang/ref/WeakReference.class") != null
                jar to hasPreferredSurface
            }
        }
    }
    return candidates.firstOrNull { it.second }?.first ?: candidates.firstOrNull()?.first
}

private fun resolveApiCompilePath(apiJars: List<Path>, compiler: JavaCompiler): ApiCompilePath =
    when (compiler.backend) {
        CompileBackend.JAVAC -> ApiCompilePath(
            bootClasspath = emptyList(),
            classpath = apiJars,
        )
        CompileBackend.ECJ,
        CompileBackend.LEGACY -> {
            val bootJar = selectBootApiJar(apiJars)
            require(bootJar != null) {
                "${compiler.displayName} compile-stubs requires a CLDC API jar that provides java/lang/Object.class in api-jars."
            }
            ApiCompilePath(
                bootClasspath = listOf(bootJar),
                classpath = apiJars - bootJar,
            )
        }
    }

private fun computeStubCacheKey(
    apiJars: List<Path>,
    localStubs: LocalStubSources,
    compiler: JavaCompiler,
    apiPath: ApiCompilePath,
): String {
    val digest = MessageDigest.getInstance("SHA-256")
    fun update(line: String) = digest.update(line.toByteArray(Charsets.UTF_8))

    update("cache-v$stubCacheVersion\n")
    update("compiler=${compiler.backend.id}\n")
    compiler.cacheInputs().forEach { update("$it\n") }

    localStubs.active.forEach { src ->
        val stat = fileStat(src)
        update("src=$src:${stat.size}:${stat.mtimeMs}\n")
    }
    apiJars.forEach { jar ->
        val stat = fileStat(jar)
        update("api=$jar:${stat.size}:${stat.mtimeMs}\n")
    }
    apiPath.bootClasspath.forEach { jar ->
        val stat = fileStat(jar)
        update("boot=$jar:${stat.size}:${stat.mtimeMs}\n")
    }

    return digest.digest().joinToString("") { "%02x".format(it) }
}

private fun writeSourceList(path: Path, sources: List<Path>) {
    // javac/ECJ parse @files as command-line tokens, so a relocatable toolkit
    // must quote source paths when project directories contain whitespace.
    path.writeText(sources.joinToString("\n") { source ->
        "\"" + source.toString().replace("\\", "\\\\").replace("\"", "\\\"") + "\""
    } + if (sources.isNotEmpty()) "\n" else "")
}

private fun writeRankedLines(path: Path, heading: String, values: List<String>) {
    val counts = values.groupingBy { it }.eachCount().entries.sortedWith(compareByDescending<Map.Entry<String, Int>> { it.value }.thenBy { it.key })
    path.writeText(
        buildString {
            appendLine(heading)
            counts.forEach { (value, count) -> appendLine("%7d %s".format(count, value)) }
        },
    )
}

private fun runCapture(runner: ProcessRunner, cmd: List<String>, cwd: Path? = null): CommandResult =
    runner.run(cmd, okReturnCodes = emptySet(), cwd = cwd)

internal data class CompileStubsArgs(
    val stubsSrcArg: String? = null,
    val stubsClassesDirArg: String? = null,
    val skipStubCompile: Boolean = false,
    val noStubCache: Boolean = false,
    val stubCacheDirArg: String? = null,
    val apiJarsDirArg: String? = null,
    val decompiledSrcArg: String? = null,
    val outDirArg: String? = null,
    val compiler: CompileBackend = CompileStubDefaults.backend,
    val javaRelease: Int = CompileStubDefaults.javaRelease,
    val maxerrs: Int = CompileStubDefaults.maxCompilerErrors,
    val javacBin: String = CompileStubDefaults.javacBin,
    val javaBin: String = CompileStubDefaults.javaBin,
    val compilerJarArg: String? = null,
    val sourceLevel: String? = null,
    val targetLevel: String? = null,
)

private data class ResolvedStubCache(
    val useStubCache: Boolean,
    val cacheManaged: Boolean,
    val cacheRoot: Path,
    val cacheKey: String,
    val classesDir: Path,
    val markerFile: Path?,
)

private data class CompileStubsWorkspace(
    val projectJar: Path,
    val stubsSrc: Path,
    val apiJarsDir: Path,
    val decompiledSrc: Path,
    val outDir: Path,
    val apiJars: List<Path>,
    val apiPath: ApiCompilePath,
    val localStubs: LocalStubSources,
    val projectSources: List<Path>,
    val stubCache: ResolvedStubCache,
    val compiler: JavaCompiler,
)

private data class StubCompilePlan(
    val mode: String,
    val cacheHit: Boolean,
    val sources: List<Path>,
    val shadowedSources: List<Path>,
)

private data class CompileRunFiles(
    val stubList: Path,
    val shadowedStubList: Path,
    val projectList: Path,
    val stubStdout: Path,
    val stubStderr: Path,
    val projectStdout: Path,
    val projectStderr: Path,
)

private data class StubCompileRun(
    val plan: StubCompilePlan,
    val files: CompileRunFiles,
    val result: CommandResult,
)

private inline fun <T> withManagedStubCacheLock(stubCache: ResolvedStubCache, action: () -> T): T {
    if (!stubCache.cacheManaged) {
        return action()
    }

    stubCache.cacheRoot.createDirectories()
    val lockPath = stubCache.cacheRoot.resolve("${stubCache.cacheKey}.lock").absolute().normalize()
    val jvmLock = managedStubCacheLocks.computeIfAbsent(lockPath) { Any() }
    synchronized(jvmLock) {
        FileChannel.open(lockPath, StandardOpenOption.CREATE, StandardOpenOption.WRITE).use { channel ->
            channel.lock().use {
                return action()
            }
        }
    }
}

private fun cleanCompileWorkspace(workspace: CompileStubsWorkspace, args: CompileStubsArgs) {
    val preserved = workspace.stubCache.classesDir.takeIf { args.skipStubCompile && it.startsWith(workspace.outDir) }
    if (preserved == null) {
        deleteRecursivelyIfExists(workspace.outDir)
        return
    }

    if (!workspace.outDir.isDirectory()) {
        return
    }
    Files.walk(workspace.outDir).use { walk ->
        walk.sorted(reverseOrder()).forEach { path ->
            if (path == workspace.outDir || path.startsWith(preserved) || preserved.startsWith(path)) {
                return@forEach
            }
            Files.deleteIfExists(path)
        }
    }
}

private fun resolveStubCache(
    stubsClassesDirArg: String?,
    noStubCache: Boolean,
    stubCacheDirArg: String?,
    paths: ToolkitPaths,
    outDir: Path,
    apiJars: List<Path>,
    localStubs: LocalStubSources,
    compiler: JavaCompiler,
    apiPath: ApiCompilePath,
): ResolvedStubCache {
    val useStubCache = !noStubCache
    val cacheManaged = useStubCache && stubsClassesDirArg == null
    val cacheRoot = (stubCacheDirArg?.let(::Path) ?: paths.base.resolve(".cache/compile-stubs")).absolute().normalize()

    return if (cacheManaged) {
        val key = computeStubCacheKey(apiJars, localStubs, compiler, apiPath)
        ResolvedStubCache(
            useStubCache = true,
            cacheManaged = true,
            cacheRoot = cacheRoot,
            cacheKey = key,
            classesDir = cacheRoot.resolve(key).resolve("classes"),
            markerFile = cacheRoot.resolve(key).resolve("ready.txt"),
        )
    } else {
        ResolvedStubCache(
            useStubCache = useStubCache,
            cacheManaged = false,
            cacheRoot = cacheRoot,
            cacheKey = "",
            classesDir = stubsClassesDirArg?.let(::Path)?.absolute()?.normalize() ?: outDir.resolve("stubs-classes"),
            markerFile = null,
        )
    }
}

private fun resolveCompileStubsWorkspace(
    root: Path,
    paths: ToolkitPaths,
    args: CompileStubsArgs,
): CompileStubsWorkspace {
    val projectJar = resolveProjectJar(root)
    val stubsSrc = (args.stubsSrcArg?.let(::Path) ?: paths.base.resolve("vendor/j2me-stubs/src/main/java")).absolute().normalize()
    val apiJarsDir = (args.apiJarsDirArg?.let(::Path) ?: paths.base.resolve("vendor/j2me-api")).absolute().normalize()
    val decompiledSrc = (args.decompiledSrcArg?.let(::Path) ?: root.resolve("decompiled")).absolute().normalize()
    val outDir = (args.outDirArg?.let(::Path) ?: root.resolve("out/compile_check")).absolute().normalize()

    if (args.stubsSrcArg != null) {
        require(stubsSrc.isDirectory()) { "Missing stubs source directory: $stubsSrc" }
    }
    require(decompiledSrc.isDirectory()) { "Missing decompiled source directory: $decompiledSrc" }

    val apiJars = listApiJars(apiJarsDir)
    val compiler = resolveCompiler(paths, root, projectJar, args)
    val apiPath = resolveApiCompilePath(apiJars, compiler)
    val localStubs = listLocalStubSources(stubsSrc, apiJars)
    val projectSources = listJavaSources(decompiledSrc)
    val stubCache = resolveStubCache(
        stubsClassesDirArg = args.stubsClassesDirArg,
        noStubCache = args.noStubCache,
        stubCacheDirArg = args.stubCacheDirArg,
        paths = paths,
        outDir = outDir,
        apiJars = apiJars,
        localStubs = localStubs,
        compiler = compiler,
        apiPath = apiPath,
    )

    return CompileStubsWorkspace(
        projectJar = projectJar,
        stubsSrc = stubsSrc,
        apiJarsDir = apiJarsDir,
        decompiledSrc = decompiledSrc,
        outDir = outDir,
        apiJars = apiJars,
        apiPath = apiPath,
        localStubs = localStubs,
        projectSources = projectSources,
        stubCache = stubCache,
        compiler = compiler,
    )
}

private fun prepareStubCompilePlan(args: CompileStubsArgs, workspace: CompileStubsWorkspace): StubCompilePlan {
    val stubCache = workspace.stubCache
    val stubSources = workspace.localStubs.active
    val cacheHit = !args.skipStubCompile &&
        stubCache.cacheManaged &&
        stubCache.markerFile?.isRegularFile() == true &&
        stubCache.classesDir.isDirectory()
    val mode = when {
        args.skipStubCompile -> {
            require(stubCache.classesDir.isDirectory()) { "--skip-stub-compile requires existing stubs classes dir: ${stubCache.classesDir}" }
            "skip-flag"
        }
        cacheHit -> "cache-hit"
        stubSources.isEmpty() -> "api-jars-only"
        else -> {
            deleteRecursivelyIfExists(stubCache.classesDir)
            stubCache.classesDir.createDirectories()
            stubCache.markerFile?.parent?.createDirectories()
            "compiled"
        }
    }
    return StubCompilePlan(mode, cacheHit, stubSources, workspace.localStubs.shadowedByApiJar)
}

private fun writeCompileSourceLists(workspace: CompileStubsWorkspace, plan: StubCompilePlan): CompileRunFiles {
    val outDir = workspace.outDir
    outDir.resolve("classes").createDirectories()

    val files = CompileRunFiles(
        stubList = outDir.resolve("stub_sources.txt"),
        shadowedStubList = outDir.resolve("shadowed_stub_sources.txt"),
        projectList = outDir.resolve("project_sources.txt"),
        stubStdout = outDir.resolve("stubs.stdout.log"),
        stubStderr = outDir.resolve("stubs.stderr.log"),
        projectStdout = outDir.resolve("compiler.stdout.log"),
        projectStderr = outDir.resolve("compiler.stderr.log"),
    )
    writeSourceList(files.stubList, plan.sources)
    writeSourceList(files.shadowedStubList, plan.shadowedSources)
    writeSourceList(files.projectList, workspace.projectSources)
    return files
}

private fun buildCompileSummary(
    workspace: CompileStubsWorkspace,
    plan: StubCompilePlan,
    files: CompileRunFiles,
    stubResult: CommandResult,
    projectResult: CommandResult,
    projectClasspathEntries: List<Path>,
    args: CompileStubsArgs,
    errorCount: Int,
    warningCount: Int,
): List<String> = buildList {
    add("compiler=${workspace.compiler.displayName}")
    workspace.compiler.summaryFields().forEach { (key, value) -> add("$key=$value") }
    addAll(
        listOf(
            "bootclasspath=${workspace.apiPath.bootClasspath.joinToString(java.io.File.pathSeparator) { it.pathString }}",
            "classpath_entries=${projectClasspathEntries.size}",
            "maxerrs=${args.maxerrs}",
            "project_jar=${workspace.projectJar}",
            "stubs_src=${workspace.stubsSrc}",
            "api_jars_dir=${workspace.apiJarsDir}",
            "api_jars=${workspace.apiJars.size}",
            "decompiled_src=${workspace.decompiledSrc}",
            "stub_cache_enabled=${if (workspace.stubCache.useStubCache) 1 else 0}",
            "stub_cache_managed=${if (workspace.stubCache.cacheManaged) 1 else 0}",
            "stub_cache_dir=${if (workspace.stubCache.cacheManaged) workspace.stubCache.cacheRoot else ""}",
            "stub_cache_key=${workspace.stubCache.cacheKey}",
            "stub_cache_hit=${if (plan.cacheHit) 1 else 0}",
            "stubs_classes_dir=${workspace.stubCache.classesDir}",
            "stub_compile_skipped=${if (args.skipStubCompile) 1 else 0}",
            "stubs_mode=${plan.mode}",
            "stub_sources=${plan.sources.size}",
            "shadowed_stub_sources=${plan.shadowedSources.size}",
            "shadowed_stub_sources_list=${files.shadowedStubList}",
            "project_sources=${workspace.projectSources.size}",
            "stubs_exit=${stubResult.returnCode}",
            "compile_exit=${projectResult.returnCode}",
            "error_lines=$errorCount",
            "warning_lines=$warningCount",
            "stubs_stdout=${files.stubStdout}",
            "stubs_stderr=${files.stubStderr}",
            "compiler_stdout=${files.projectStdout}",
            "compiler_stderr=${files.projectStderr}",
        ),
    )
}

internal fun compileStubs(
    root: Path,
    paths: ToolkitPaths,
    runner: ProcessRunner,
    args: CompileStubsArgs,
    quiet: Boolean = false,
) {
    val workspace = resolveCompileStubsWorkspace(root, paths, args)
    cleanCompileWorkspace(workspace, args)
    val stubRun = withManagedStubCacheLock(workspace.stubCache) {
        val plan = prepareStubCompilePlan(args, workspace)
        val files = writeCompileSourceLists(workspace, plan)

        val stubResult = when {
            args.skipStubCompile -> CommandResult(listOf(workspace.compiler.displayName), 0, "(skipped by flag)\n", "")
            plan.cacheHit -> CommandResult(listOf(workspace.compiler.displayName), 0, "(reused stub cache)\n", "")
            plan.sources.isEmpty() -> CommandResult(listOf(workspace.compiler.displayName), 0, "(no local stubs; API jars provide the compile surface)\n", "")
            else -> {
                runCapture(
                    runner,
                    workspace.compiler.buildCommand(
                        apiPath = workspace.apiPath,
                        classpathEntries = workspace.apiPath.classpath,
                        outputDir = workspace.stubCache.classesDir,
                        sourceList = files.stubList,
                    ),
                    cwd = workspace.compiler.cwd,
                )
            }
        }

        files.stubStdout.writeText(stubResult.stdout)
        files.stubStderr.writeText(stubResult.stderr)

        if (!plan.cacheHit && plan.sources.isNotEmpty() && stubResult.returnCode == 0 && workspace.stubCache.markerFile != null) {
            workspace.stubCache.markerFile.writeText(
                listOf(
                    "cache_version=$stubCacheVersion",
                    "cache_key=${workspace.stubCache.cacheKey}",
                    "stubs_classes_dir=${workspace.stubCache.classesDir}",
                ).joinToString("\n") + "\n",
            )
        }

        StubCompileRun(plan, files, stubResult)
    }

    val plan = stubRun.plan
    val files = stubRun.files
    val stubResult = stubRun.result

    val stubCompileFailed = stubResult.returnCode != 0
    val stubClasspathEntries = if (args.skipStubCompile || plan.sources.isNotEmpty()) listOf(workspace.stubCache.classesDir) else emptyList()
    val projectClasspathEntries = workspace.apiPath.classpath + stubClasspathEntries
    val projectCmd = workspace.compiler.buildCommand(
        apiPath = workspace.apiPath,
        classpathEntries = projectClasspathEntries,
        outputDir = workspace.outDir.resolve("classes"),
        sourceList = files.projectList,
        maxerrs = args.maxerrs,
    )

    val projectResult = if (stubCompileFailed) {
        CommandResult(projectCmd, -1, "", "(skipped because stub compile failed)\n")
    } else {
        runCapture(runner, projectCmd, cwd = workspace.compiler.cwd)
    }
    files.projectStdout.writeText(projectResult.stdout)
    files.projectStderr.writeText(projectResult.stderr)

    workspace.outDir.resolve("stubs.exitcode").writeText("${stubResult.returnCode}\n")
    workspace.outDir.resolve("compiler.exitcode").writeText("${projectResult.returnCode}\n")

    val diagnosticStderr = if (stubCompileFailed) stubResult.stderr else projectResult.stderr
    val diagnostics = workspace.compiler.diagnostics(diagnosticStderr)
    val errors = diagnostics.errors
    val warningCount = diagnostics.warningCount

    val summary = buildCompileSummary(
        workspace = workspace,
        plan = plan,
        files = files,
        stubResult = stubResult,
        projectResult = projectResult,
        projectClasspathEntries = projectClasspathEntries,
        args = args,
        errorCount = errors.size,
        warningCount = warningCount,
    )
    workspace.outDir.resolve("summary.txt").writeText(summary.joinToString("\n") + "\n")
    workspace.outDir.resolve("compile-check.toml").writeText(
        buildString {
            appendLine("decompiled_src = ${tomlString(workspace.decompiledSrc.toString())}")
        },
    )

    val messageValues = errors.mapNotNull { it.message }
    val fileValues = errors.mapNotNull { it.file }

    val errorsByMessagePath = workspace.outDir.resolve("errors_by_message.txt")
    val errorsByFilePath = workspace.outDir.resolve("errors_by_file.txt")
    if (errors.isNotEmpty()) {
        writeRankedLines(errorsByMessagePath, "Errors by message:", messageValues)
        writeRankedLines(errorsByFilePath, "Errors by file:", fileValues)
    } else {
        Files.deleteIfExists(errorsByMessagePath)
        Files.deleteIfExists(errorsByFilePath)
    }

    val status = if (stubResult.returnCode == 0 && projectResult.returnCode == 0) "PASS" else "FAIL"
    if (!quiet) {
        println("Compile check $status: sources=${workspace.projectSources.size} errors=${errors.size} warnings=$warningCount stubs=${plan.mode}")
        println("Summary: ${workspace.outDir.resolve("summary.txt")}")
        if (errors.isNotEmpty()) {
            println("Errors by message: $errorsByMessagePath")
            println("Errors by file: $errorsByFilePath")
        }
        if (stubCompileFailed) {
            println("Full ${workspace.compiler.displayName} stub stderr: ${files.stubStderr}")
        } else if (warningCount > 0 || projectResult.returnCode != 0) {
            println("Full ${workspace.compiler.displayName} stderr: ${files.projectStderr}")
        }
    }

    if (stubCompileFailed) {
        throw IllegalStateException("${workspace.compiler.displayName} stub compile failed with exit code ${stubResult.returnCode}")
    }

    if (projectResult.returnCode != 0) {
        throw IllegalStateException("${workspace.compiler.displayName} failed with exit code ${projectResult.returnCode}")
    }
}
