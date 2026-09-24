package j2me.cli

import j2me.bytecode.RemappedJarStats
import j2me.bytecode.defaultRemappedJarPath
import j2me.bytecode.remapJarBytecode
import j2me.map.loadJavaLikeMappings
import j2me.model.ClassSymbols
import j2me.model.ProjectMappings
import j2me.model.CanonicalMap
import j2me.model.FieldSig
import j2me.model.MethodSig
import org.jetbrains.java.decompiler.api.NamingPlan
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
import j2me.semantic.generatedDomainOwners
import org.jetbrains.java.decompiler.api.SemanticMappingData
import org.jetbrains.java.decompiler.api.J2meApi
import j2me.validation.validateMap
import j2me.validation.validateRealizedNames
import org.tomlj.TomlParseResult
import java.nio.file.Path
import java.nio.file.Files
import kotlin.io.path.absolute
import kotlin.io.path.exists
import kotlin.io.path.isDirectory
import kotlin.io.path.pathString

internal data class DecompilerConfig(
    val output: Path,
    val external: List<Path>,
    val api: J2meApi.Resolution? = null,
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
    val preserveClassNameStrings: Boolean = true,
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
        "preserve-class-name-strings" to args.preserveClassNameStrings.toString(),
    )

    options["rename-members"] = "true"
    if (!args.raw) options["default-package"] = "defpackage"
    if (tinyPath != null) {
        options["mappings-path"] = tinyPath.pathString
        options["mappings-source-namespace"] = "official"
        options["mappings-target-namespace"] = "named"
    }

    if (args.noComments) {
        options["sourcefile-comments"] = "false"
        options["decompiler-comments"] = "false"
    }
    val managed = setOf("mappings-path", "mappings-source-namespace", "mappings-target-namespace",
        "prepared-names-path", "naming-output", "source-metadata-output", "prepare-names-only", "reserved-class-names",
        "rename-members", "preserve-class-name-strings", "user-renamer-class", "semantic-mappings-path")
    require(args.decompilerOptions.keys.none { it in managed }) {
        "Decompiler options cannot override pipeline naming controls: ${args.decompilerOptions.keys.intersect(managed)}"
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
        api = decompiler.api,
    )
}


private fun loadSymbolsForPipeline(args: RemapPipelineArgs): JarAnalysis {
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
    val api = J2meApi.resolve(jar, localApiJars(paths), true)
    val configuredWorkers = global.valueOrDefault(
        "remap.analysis_workers",
        global.valueOrDefault("remap.javap_workers", 8L) { getLong(it) },
    ) { getLong(it) }
    require(configuredWorkers in 1..Int.MAX_VALUE.toLong()) {
        "remap.analysis_workers must be between 1 and ${Int.MAX_VALUE}, got $configuredWorkers"
    }
    val analysisWorkers = configuredWorkers.toInt()
    val classpathSymbolsByClass = apiClassSymbols(api, analysisWorkers)

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
                external = emptyList(),
                api = api,
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
    val names = prepareNaming(args, symbols, cmap, decompilerRunner)
    val realized = names.canonical(cmap?.canonical ?: CanonicalMap())
    if (cmap != null && args.semanticMappingsEnabled) {
        validateSemanticMap(cmap.semantic, realized, symbols.symbolsByClass, args.classpathSymbolsByClass)
    }
    validateRealizedNames(args.jar, realized, args.classpathSymbolsByClass)
    ensureOutputDir(args.outDir, args.overwriteOutputDir)

    args.writeIndex?.let {
        writeSymbolIndex(it, symbols.symbolsByClass)
        if (!quiet) println("Wrote symbol index: $it")
    }
    if (args.raw && !quiet) println("Raw mode: skipping mappings and enabling automatic member renaming.")
    val namingPath = args.outDir.resolve("mapping.tiny")
    names.write(namingPath)
    val mappingOutputs = cmap?.let { mappedModeOutputs(args, symbols, it, realized, quiet) }
    val decompileOutputs = runDecompiler(args, mappingOutputs, names, decompilerRunner)
    return RemapPipelineResult(args.outDir, mappingOutputs, decompileOutputs, (System.nanoTime() - pipelineStartNs) / 1_000_000)
        .also { if (!quiet) printSummary(it) }
}

internal fun NamingPlan.canonical(requested: CanonicalMap = CanonicalMap()): CanonicalMap = requested.copy(
    classes = classes(),
    fields = fields().mapKeys { (key, _) -> FieldSig(key.owner(), key.name(), key.descriptor()) },
    methods = methods().mapKeys { (key, _) -> MethodSig(key.owner(), key.name(), key.descriptor()) },
)

private fun prepareNaming(args: RemapPipelineArgs, symbols: JarAnalysis, mappings: ProjectMappings?, runner: DecompilerRunner): NamingPlan {
    val scratch = Files.createTempDirectory("sporeflower-names-")
    try {
        val requested = mappings?.let {
            scratch.resolve("requested.tiny").also { path ->
                writeTinyMapping(path, it.canonical, symbols.symbolsByClass, symbols.classes)
            }
        }
        val preparedArgs = args.copy(outDir = scratch,
            decompiler = (args.decompiler ?: DecompilerConfig(scratch, emptyList())).copy(output = scratch))
        val invocation = buildDecompilerInvocation(preparedArgs, requested, null)
        val reserved = mappings?.semantic?.let(::generatedDomainOwners).orEmpty().joinToString(",")
        return runner.prepareNames(invocation.copy(options = invocation.options + ("reserved-class-names" to reserved)))
    } finally {
        deleteRecursivelyIfExists(scratch)
    }
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
    return mappings
}

private fun mappedModeOutputs(args: RemapPipelineArgs, symbols: JarAnalysis, mappings: ProjectMappings, cmap: CanonicalMap, quiet: Boolean): MappingOutputs {
    val inventory = MemberInventory(symbols.symbolsByClass, mappings.canonical, symbols.usage)
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
    val classNames = j2me.bytecode.resolveClassNameRemapping(args.jar, cmap, symbols.symbolsByClass, mappings.semantic.classNames)
    val semantics = if (mappings.semantic.domains.isEmpty() && classNames.literals.isEmpty() && classNames.warnings.isEmpty() && !args.exportSemanticMap) null else
        buildSemanticMappings(mappings.semantic, cmap, symbols.symbolsByClass, args.classpathSymbolsByClass,
            if (args.preserveClassNameStrings) emptyList() else classNames.literals)
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
    names: NamingPlan,
    runner: DecompilerRunner,
): DecompileOutputs? {
    val decompiler = args.decompiler ?: return null

    ensureOutputDir(decompiler.output, args.overwriteOutputDir)
    val invocation = buildDecompilerInvocation(args, null, mappingOutputs?.semanticMappings)
    val decompilerMs = runner.run(invocation.copy(options = invocation.options + mapOf(
        "naming-output" to decompiler.output.resolve(".sporeflower-names.tiny").toString(),
        "source-metadata-output" to decompiler.output.resolve(".sporeflower.json").toString(),
    ), preparedNames = names))

    return DecompileOutputs(
        output = decompiler.output,
        elapsedMs = decompilerMs,
        fileCount = countJavaFiles(decompiler.output),
    )
}
