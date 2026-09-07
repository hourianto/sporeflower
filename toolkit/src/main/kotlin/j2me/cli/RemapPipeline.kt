package j2me.cli

import j2me.bytecode.RemappedJarStats
import j2me.bytecode.defaultRemappedJarPath
import j2me.bytecode.remapJarBytecode
import j2me.map.loadJavaLikeMappings
import j2me.model.ClassSymbols
import j2me.model.ProjectMappings
import j2me.output.writeTinyMapping
import j2me.reports.MemberInventory
import j2me.reports.CoverageStats
import j2me.reports.SemanticStats
import j2me.reports.writeCoverageReport
import j2me.reports.writeSemanticReport
import j2me.reports.writeSymbolIndex
import j2me.reports.writeUsagePriorityReport
import j2me.symbols.AnalysisCachePaths
import j2me.symbols.JarAnalysis
import j2me.symbols.analyzeJar
import j2me.semantic.validateSemanticMap
import j2me.semantic.buildSemanticMappings
import org.jetbrains.java.decompiler.api.SemanticMappingData
import j2me.validation.validateMap
import org.tomlj.TomlParseResult
import java.nio.file.Path
import kotlin.io.path.absolute
import kotlin.io.path.exists
import kotlin.io.path.isDirectory
import kotlin.io.path.pathString

internal data class DecompilerConfig(
    val output: Path,
    val external: List<Path>,
)

internal data class RemapPipelineArgs(
    val jar: Path,
    val mapsDir: Path,
    val outDir: Path,
    val classpathSymbolsByClass: Map<String, ClassSymbols>,
    val overwriteOutputDir: Boolean,
    val writeIndex: Path?,
    val raw: Boolean,
    val noComments: Boolean,
    val semanticMappingsEnabled: Boolean = true,
    val exportSemanticMap: Boolean = false,
    val analysisWorkers: Int,
    val cache: AnalysisCachePaths,
    val decompiler: DecompilerConfig?,
    val decompilerOptions: Map<String, String> = emptyMap(),
)

internal data class MappingOutputs(
    val tinyPath: Path,
    val semanticMappings: SemanticMappingData?,
    val coverage: CoverageStats,
    val coveragePath: Path,
    val usageMdPath: Path,
    val usageTsvPath: Path,
    val semanticStats: SemanticStats?,
    val semanticReportPath: Path?,
    val remappedJar: RemappedJarStats,
)

internal data class DecompileOutputs(val output: Path, val fileCount: Int, val elapsedMs: Long)

internal data class RemapPipelineResult(
    val outDir: Path,
    val mappings: MappingOutputs?,
    val decompiled: DecompileOutputs?,
    val elapsedMs: Long,
)

private fun printSummary(summary: RemapPipelineResult) {
    summary.mappings?.coverage?.let { coverage ->
        println(
            "Coverage: ${coverage.classDeclared}/${coverage.classTotal} classes, " +
                "${coverage.fieldMapped}/${coverage.fieldTotal} fields, " +
                "${coverage.methodMapped}/${coverage.methodTotal} methods " +
                "(${formatPercentOneDecimal(coverage.memberMapped, coverage.memberTotal)} overall)",
        )
        if (coverage.deadFieldTotal > 0) {
            println("Dead fields: ${coverage.deadFieldTotal} excluded from coverage")
        }
        if (coverage.ignoredClassTotal > 0) {
            println("Ignored classes: ${coverage.ignoredClassTotal} already named in bytecode")
        }
        println()
    }
    summary.mappings?.semanticStats?.let { semantic ->
        println(
            "Semantics: ${semantic.domainTotal} domains, ${semantic.valueTotal} named values; " +
                "${semantic.fieldBindings} fields, ${semantic.returnBindings} returns, " +
                "${semantic.parameterBindings} parameters; return-domain sources: ${semantic.returnDomainSources}",
        )
        println(
            "Semantic arrays: ${semantic.arrayBindingTotal} arrays; ${semantic.indexBindings} index dimensions, " +
                "${semantic.slotBindings} slot dimensions, ${semantic.elementBindings} leaf-value bindings",
        )
        println()
    }

    val outputRoot = summary.outDir.parent.absolute()
    println("Output → ${outputRoot.pathString}${java.io.File.separator}")
    summary.mappings?.let { mapping ->
        for (path in listOfNotNull(mapping.coveragePath, mapping.usageMdPath, mapping.usageTsvPath, mapping.semanticReportPath)) {
            println("  ${relativeOrAbsolute(path, outputRoot)}")
        }
        val jar = mapping.remappedJar
        println("  ${relativeOrAbsolute(jar.path, outputRoot)}  (${jar.classCount} classes, ${jar.resourceCount} resources)")
    }
    summary.decompiled?.let {
        println("  ${relativeOrAbsolute(it.output, outputRoot).trimEnd('/')}/  (${it.fileCount} files)")
    }
    println("Timing: total=${summary.elapsedMs}ms, decompiler=${summary.decompiled?.elapsedMs ?: 0}ms")
}

private fun buildDecompilerInvocation(
    args: RemapPipelineArgs,
    tinyPath: Path?,
    semantics: SemanticMappingData?,
): DecompilerInvocation {
    val decompiler = requireNotNull(args.decompiler)

    val options = linkedMapOf(
        "skip-extra-files" to "true",
        "j2me-strict-slot-merge" to "true",
        "legacy-ternary-reference-casts" to "true",
        "decompile-autoboxing" to "false",
    )

    if (args.raw) {
        options["rename-members"] = "true"
    } else if (tinyPath != null) {
        options["mappings-path"] = tinyPath.pathString
        options["mappings-source-namespace"] = "official"
        options["mappings-target-namespace"] = "named"
    }

    if (args.noComments) {
        options["sourcefile-comments"] = "false"
        options["decompiler-comments"] = "false"
    }
    options.putAll(args.decompilerOptions)

    return DecompilerInvocation(
        source = args.jar,
        output = decompiler.output,
        options = options,
        libraries = decompiler.external,
        logStdoutPath = args.outDir.resolve("decompiler.stdout.log"),
        logStderrPath = args.outDir.resolve("decompiler.stderr.log"),
        semantics = semantics,
    )
}


private fun loadSymbolsForPipeline(args: RemapPipelineArgs): JarAnalysis {
    val needSymbols = !args.raw || args.writeIndex != null
    if (!needSymbols) {
        return JarAnalysis(emptyMap())
    }
    return analyzeJar(args.jar, args.analysisWorkers, args.cache, includeUsage = !args.raw)
}

internal fun buildRemapPipelineArgs(
    root: Path,
    paths: ToolkitPaths,
    global: TomlParseResult?,
    jar: Path,
    raw: Boolean,
    noComments: Boolean,
    semanticMappingsEnabled: Boolean = true,
    exportSemanticMap: Boolean = false,
): RemapPipelineArgs {
    require(!exportSemanticMap || !raw && semanticMappingsEnabled) { "--export-semantic-map requires semantic mappings; omit --raw and --no-semantic-mappings" }
    val decompilerEnabled = global.valueOrDefault("decompiler.enabled", true) { getBoolean(it) }
    val apiJars = listApiJars(paths.base.resolve("vendor/j2me-api"))
    val configuredWorkers = global.valueOrDefault(
        "remap.analysis_workers",
        global.valueOrDefault("remap.javap_workers", 8L) { getLong(it) },
    ) { getLong(it) }
    require(configuredWorkers in 1..Int.MAX_VALUE.toLong()) {
        "remap.analysis_workers must be between 1 and ${Int.MAX_VALUE}, got $configuredWorkers"
    }
    val analysisWorkers = configuredWorkers.toInt()
    val classpathSymbolsByClass = if (raw) emptyMap() else apiClassSymbols(apiJars, analysisWorkers)

    return RemapPipelineArgs(
        jar = jar,
        mapsDir = root.resolve("mappings"),
        outDir = root.resolve("out"),
        classpathSymbolsByClass = classpathSymbolsByClass,
        overwriteOutputDir = global.valueOrDefault("remap.overwrite", true) { getBoolean(it) },
        writeIndex = if (global.valueOrDefault("remap.write_index", false) { getBoolean(it) }) root.resolve("out/symbol-index.tsv") else null,
        raw = raw,
        noComments = noComments,
        semanticMappingsEnabled = semanticMappingsEnabled,
        exportSemanticMap = exportSemanticMap,
        analysisWorkers = analysisWorkers,
        cache = AnalysisCachePaths(
            symbols = root.resolve(".cache/remap-symbols.json"),
            usage = root.resolve(".cache/remap-usage.json"),
        ),
        decompiler = if (decompilerEnabled) {
            DecompilerConfig(
                output = root.resolve("decompiled"),
                external = apiJars,
            )
        } else {
            null
        },
    )
}

internal fun runRemapPipeline(
    args: RemapPipelineArgs,
    decompilerRunner: DecompilerRunner,
    quiet: Boolean = false,
): RemapPipelineResult {
    val pipelineStartNs = System.nanoTime()

    require(args.jar.exists()) { "JAR not found: ${args.jar}" }
    require(args.analysisWorkers >= 1) { "remap.analysis_workers must be >= 1, got ${args.analysisWorkers}" }
    require(!args.raw || args.decompiler != null) {
        "Raw remap requires decompilation. Set decompiler.enabled = true in global.toml."
    }

    if (!args.raw) {
        require(args.mapsDir.exists() && args.mapsDir.isDirectory()) { "Maps directory not found: ${args.mapsDir}" }
    }

    val symbols = loadSymbolsForPipeline(args)
    val cmap = if (args.raw) {
        null
    } else {
        loadAndValidateMap(args, symbols)
    }
    ensureOutputDir(args.outDir, args.overwriteOutputDir)

    args.writeIndex?.let {
        writeSymbolIndex(it, symbols.symbolsByClass)
        if (!quiet) println("Wrote symbol index: $it")
    }
    if (args.raw && !quiet) println("Raw mode: skipping mappings and enabling automatic member renaming.")
    val mappingOutputs = cmap?.let { mappedModeOutputs(args, symbols, it, quiet) }
    val decompileOutputs = runDecompiler(args, mappingOutputs, decompilerRunner)
    return RemapPipelineResult(args.outDir, mappingOutputs, decompileOutputs, (System.nanoTime() - pipelineStartNs) / 1_000_000)
        .also { if (!quiet) printSummary(it) }
}

private fun loadAndValidateMap(args: RemapPipelineArgs, symbols: JarAnalysis): ProjectMappings {
    val mappings = loadJavaLikeMappings(
        args.mapsDir,
        symbols.classes.toSet(),
        args.classpathSymbolsByClass.keys,
        args.classpathSymbolsByClass,
        includeSemanticMappings = args.semanticMappingsEnabled,
    )
    val cmap = mappings.canonical
    validateMap(
        symbolsByClass = symbols.symbolsByClass,
        cmap = cmap,
        mapsDir = args.mapsDir,
        classpathSymbolsByClass = args.classpathSymbolsByClass,
    )
    if (args.semanticMappingsEnabled) {
        validateSemanticMap(mappings.semantic, cmap, symbols.symbolsByClass, args.classpathSymbolsByClass)
    }
    return mappings
}

private fun mappedModeOutputs(args: RemapPipelineArgs, symbols: JarAnalysis, mappings: ProjectMappings, quiet: Boolean): MappingOutputs {
    val cmap = mappings.canonical
    val inventory = MemberInventory(symbols.symbolsByClass, cmap, symbols.usage)
    val coveragePath = args.outDir.resolve("coverage.md")
    val coverage = writeCoverageReport(coveragePath, inventory)

    val usageMdPath = args.outDir.resolve("usage-priority.md")
    val usageTsvPath = args.outDir.resolve("usage-priority.tsv")
    writeUsagePriorityReport(
        usageMdPath,
        usageTsvPath,
        inventory,
    )

    val tinyPath = args.outDir.resolve("mapping.tiny")
    writeTinyMapping(tinyPath, cmap, symbols.symbolsByClass, symbols.symbolsByClass.keys)
    val classNames = j2me.bytecode.resolveClassNameRemapping(args.jar, cmap, symbols.symbolsByClass, mappings.semantic.classNames)
    val semantics = if (mappings.semantic.domains.isEmpty() && classNames.literals.isEmpty() && classNames.warnings.isEmpty() && !args.exportSemanticMap) null else
        buildSemanticMappings(mappings.semantic, cmap, symbols.symbolsByClass, args.classpathSymbolsByClass, classNames.literals)
    if (args.exportSemanticMap) {
        val path = args.outDir.resolve("semantic-map.json")
        requireNotNull(semantics).write(path)
        if (!quiet) println("Wrote semantic map: $path")
    }
    val semanticReportPath = if (mappings.semantic.domains.isEmpty() && mappings.semantic.classNames.isEmpty() && classNames.literals.isEmpty() && classNames.warnings.isEmpty()) null else args.outDir.resolve("semantic-summary.md")
    val semanticStats = semanticReportPath?.let { writeSemanticReport(it, mappings.semantic, symbols.symbolsByClass, cmap, classNames) }
    val remappedJar = remapJarBytecode(
        inputJar = args.jar,
        outputJar = defaultRemappedJarPath(args.jar, args.outDir),
        classNameLiterals = classNames.literals,
        mappings = cmap,
        symbolsByClass = symbols.symbolsByClass,
    )

    return MappingOutputs(
        tinyPath,
        semantics,
        coverage,
        coveragePath,
        usageMdPath,
        usageTsvPath,
        semanticStats,
        semanticReportPath,
        remappedJar,
    )
}

private fun runDecompiler(
    args: RemapPipelineArgs,
    mappingOutputs: MappingOutputs?,
    runner: DecompilerRunner,
): DecompileOutputs? {
    val decompiler = args.decompiler ?: return null

    ensureOutputDir(decompiler.output, args.overwriteOutputDir)
    val decompilerMs = runner.run(buildDecompilerInvocation(args, mappingOutputs?.tinyPath, mappingOutputs?.semanticMappings))

    return DecompileOutputs(
        output = decompiler.output,
        elapsedMs = decompilerMs,
        fileCount = countJavaFiles(decompiler.output),
    )
}
