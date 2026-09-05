package j2me.cli

import j2me.bytecode.RemappedJarStats
import j2me.bytecode.defaultRemappedJarPath
import j2me.bytecode.remapJarBytecode
import j2me.map.loadJavaLikeMappings
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.ProjectMappings
import j2me.output.writeTinyMapping
import j2me.process.ProcessRunner
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

internal data class VineflowerConfig(
    val bin: String,
    val javaBin: String,
    val output: Path,
    val external: List<Path>,
    val bundled: Boolean = true,
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
    val vineflower: VineflowerConfig?,
    val extraVineflowerOptions: Map<String, String> = emptyMap(),
)

private data class PipelineSummary(
    val outDir: Path,
    val coverage: CoverageStats?,
    val coveragePath: Path?,
    val usageMdPath: Path?,
    val usageTsvPath: Path?,
    val semanticStats: SemanticStats?,
    val semanticReportPath: Path?,
    val remappedJar: RemappedJarStats?,
    val decompiledOutput: Path?,
    val decompiledFileCount: Int?,
)

private data class MappingOutputs(
    val tinyPath: Path?,
    val semanticMappings: SemanticMappingData?,
    val coverage: CoverageStats?,
    val coveragePath: Path?,
    val usageMdPath: Path?,
    val usageTsvPath: Path?,
    val semanticStats: SemanticStats?,
    val semanticReportPath: Path?,
    val remappedJar: RemappedJarStats?,
)

internal data class DecompileOutputs(
    val vineflowerWaitMs: Long,
    val decompiledFileCount: Int?,
)

internal data class RemapPipelineResult(
    val raw: Boolean,
    val coverage: CoverageStats?,
    val mappingPath: Path?,
    val remappedJar: Path?,
    val decompiledOutput: Path?,
    val decompiledFileCount: Int?,
    val vineflowerWaitMs: Long,
)

private fun printSummary(summary: PipelineSummary) {
    if (summary.coverage != null) {
        println(
            "Coverage: ${summary.coverage.classDeclared}/${summary.coverage.classTotal} classes, " +
                "${summary.coverage.fieldMapped}/${summary.coverage.fieldTotal} fields, " +
                "${summary.coverage.methodMapped}/${summary.coverage.methodTotal} methods " +
                "(${formatPercentOneDecimal(summary.coverage.memberMapped, summary.coverage.memberTotal)} overall)",
        )
        if (summary.coverage.deadFieldTotal > 0) {
            println("Dead fields: ${summary.coverage.deadFieldTotal} excluded from coverage")
        }
        if (summary.coverage.ignoredClassTotal > 0) {
            println("Ignored classes: ${summary.coverage.ignoredClassTotal} already named in bytecode")
        }
        println()
    }
    if (summary.semanticStats != null) {
        val semantic = summary.semanticStats
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
    summary.coveragePath?.let { println("  ${relativeOrAbsolute(it, outputRoot)}") }
    summary.usageMdPath?.let { println("  ${relativeOrAbsolute(it, outputRoot)}") }
    summary.usageTsvPath?.let { println("  ${relativeOrAbsolute(it, outputRoot)}") }
    summary.semanticReportPath?.let { println("  ${relativeOrAbsolute(it, outputRoot)}") }
    summary.remappedJar?.let {
        println("  ${relativeOrAbsolute(it.path, outputRoot)}  (${it.classCount} classes, ${it.resourceCount} resources)")
    }

    if (summary.decompiledOutput != null) {
        var display = relativeOrAbsolute(summary.decompiledOutput, outputRoot)
        if (!display.endsWith('/')) {
            display += "/"
        }
        val suffix = summary.decompiledFileCount?.let { "  ($it files)" }.orEmpty()
        println("  $display$suffix")
    }
}

private fun buildVineflowerInvocation(
    args: RemapPipelineArgs,
    tinyPath: Path?,
    semantics: SemanticMappingData?,
): VineflowerInvocation {
    val vineflower = requireNotNull(args.vineflower)

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
    options.putAll(args.extraVineflowerOptions)

    return VineflowerInvocation(
        bin = vineflower.bin,
        javaBin = vineflower.javaBin,
        source = args.jar,
        output = vineflower.output,
        options = options,
        libraries = vineflower.external,
        logStdoutPath = args.outDir.resolve("vineflower.stdout.log"),
        logStderrPath = args.outDir.resolve("vineflower.stderr.log"),
        semantics = semantics,
        bundled = vineflower.bundled,
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
    val vineflowerEnabled = global.valueOrDefault("vineflower.enabled", true) { getBoolean(it) }
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
        vineflower = if (vineflowerEnabled) {
            VineflowerConfig(
                bin = configuredDecompiler(paths, global),
                javaBin = global.valueOrDefault("vineflower.java_bin", "java") { getString(it) },
                output = root.resolve("decompiled"),
                external = apiJars,
                bundled = configuredDecompiler(paths, global) == paths.bundledDecompiler.toString(),
            )
        } else {
            null
        },
    )
}

internal fun runRemapPipeline(
    args: RemapPipelineArgs,
    runner: ProcessRunner,
    vineflowerRunner: VineflowerRunner = InProcessVineflowerRunner(fallback = ProcessVineflowerRunner(runner)),
    quiet: Boolean = false,
): RemapPipelineResult {
    val pipelineStartNs = System.nanoTime()

    require(args.jar.exists()) { "JAR not found: ${args.jar}" }
    require(args.analysisWorkers >= 1) { "remap.analysis_workers must be >= 1, got ${args.analysisWorkers}" }
    require(!args.raw || args.vineflower != null) {
        "Raw remap requires Vineflower because raw mode only decompiles bytecode. Enable vineflower.enabled in global.toml."
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

    val mappingOutputs = if (args.raw) {
        rawModeOutputs(args, symbols, quiet)
    } else {
        mappedModeOutputs(args, symbols, requireNotNull(cmap))
    }
    val decompileOutputs = runVineflower(args, mappingOutputs, vineflowerRunner)

    if (!quiet) {
        printSummary(
            PipelineSummary(
                outDir = args.outDir,
                coverage = mappingOutputs.coverage,
                coveragePath = mappingOutputs.coveragePath,
                usageMdPath = mappingOutputs.usageMdPath,
                usageTsvPath = mappingOutputs.usageTsvPath,
                semanticStats = mappingOutputs.semanticStats,
                semanticReportPath = mappingOutputs.semanticReportPath,
                remappedJar = mappingOutputs.remappedJar,
                decompiledOutput = args.vineflower?.output,
                decompiledFileCount = decompileOutputs.decompiledFileCount,
            ),
        )
    }

    val pipelineWallMs = (System.nanoTime() - pipelineStartNs) / 1_000_000
    val kotlinNoVineflowerWaitMs = (pipelineWallMs - decompileOutputs.vineflowerWaitMs).coerceAtLeast(0)
    if (!quiet) {
        println(
            "Timing: kotlin pipeline (excluding Vineflower process wait) = ${kotlinNoVineflowerWaitMs}ms " +
                "[wall=${pipelineWallMs}ms, vineflower_wait=${decompileOutputs.vineflowerWaitMs}ms]",
        )
    }

    return RemapPipelineResult(
        raw = args.raw,
        coverage = mappingOutputs.coverage,
        mappingPath = mappingOutputs.tinyPath,
        remappedJar = mappingOutputs.remappedJar?.path,
        decompiledOutput = args.vineflower?.output,
        decompiledFileCount = decompileOutputs.decompiledFileCount,
        vineflowerWaitMs = decompileOutputs.vineflowerWaitMs,
    )
}

private fun rawModeOutputs(args: RemapPipelineArgs, symbols: JarAnalysis, quiet: Boolean): MappingOutputs {
    if (!quiet) {
        println("Raw mode: skipping map load/generation and forcing Vineflower rename-members.")
    }
    writeOptionalSymbolIndex(args, symbols.symbolsByClass)
    return MappingOutputs(
        tinyPath = null,
        semanticMappings = null,
        coverage = null,
        coveragePath = null,
        usageMdPath = null,
        usageTsvPath = null,
        semanticStats = null,
        semanticReportPath = null,
        remappedJar = null,
    )
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

private fun mappedModeOutputs(args: RemapPipelineArgs, symbols: JarAnalysis, mappings: ProjectMappings): MappingOutputs {
    val cmap = mappings.canonical
    val coveragePath = args.outDir.resolve("coverage.md")
    val coverage = writeCoverageReport(coveragePath, symbols.symbolsByClass, cmap, symbols.usage)

    val usageMdPath = args.outDir.resolve("usage-priority.md")
    val usageTsvPath = args.outDir.resolve("usage-priority.tsv")
    writeUsagePriorityReport(
        usageMdPath,
        usageTsvPath,
        symbols.symbolsByClass,
        cmap,
        symbols.usage,
    )

    writeOptionalSymbolIndex(args, symbols.symbolsByClass)

    val tinyPath = args.outDir.resolve("mapping.tiny")
    writeTinyMapping(tinyPath, cmap, symbols.symbolsByClass, symbols.symbolsByClass.keys)
    val semantics = if (mappings.semantic.domains.isEmpty() && !args.exportSemanticMap) null else
        buildSemanticMappings(mappings.semantic, cmap, symbols.symbolsByClass, args.classpathSymbolsByClass)
    if (args.exportSemanticMap) {
        val path = args.outDir.resolve("semantic-map.json")
        requireNotNull(semantics).write(path)
        println("Wrote semantic map: $path")
    }
    val semanticReportPath = if (mappings.semantic.domains.isEmpty()) null else args.outDir.resolve("semantic-summary.md")
    val semanticStats = semanticReportPath?.let { writeSemanticReport(it, mappings.semantic) }
    val remappedJar = remapJarBytecode(
        inputJar = args.jar,
        outputJar = defaultRemappedJarPath(args.jar, args.outDir),
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

private fun writeOptionalSymbolIndex(args: RemapPipelineArgs, symbolsByClass: Map<String, ClassSymbols>) {
    args.writeIndex?.let {
        writeSymbolIndex(it, symbolsByClass)
        println("Wrote symbol index: $it")
    }
}

private fun runVineflower(
    args: RemapPipelineArgs,
    mappingOutputs: MappingOutputs,
    runner: VineflowerRunner,
): DecompileOutputs {
    val vineflower = args.vineflower ?: return DecompileOutputs(0L, null)

    ensureOutputDir(vineflower.output, args.overwriteOutputDir)
    val waitMs = runner.run(buildVineflowerInvocation(args, mappingOutputs.tinyPath, mappingOutputs.semanticMappings))

    return DecompileOutputs(
        vineflowerWaitMs = waitMs,
        decompiledFileCount = countJavaFiles(vineflower.output),
    )
}
