package j2me.cli

import com.github.ajalt.clikt.core.CliktCommand
import com.github.ajalt.clikt.core.Context
import com.github.ajalt.clikt.core.main
import com.github.ajalt.clikt.core.subcommands
import com.github.ajalt.clikt.output.PlaintextHelpFormatter
import com.github.ajalt.clikt.parameters.options.default
import com.github.ajalt.clikt.parameters.options.check
import com.github.ajalt.clikt.parameters.options.convert
import com.github.ajalt.clikt.parameters.options.flag
import com.github.ajalt.clikt.parameters.options.option
import com.github.ajalt.clikt.parameters.options.required
import com.github.ajalt.clikt.parameters.types.int
import j2me.process.ProcessRunner
import j2me.process.RealProcessRunner
import j2me.validation.printUserFacingError
import java.nio.file.Files
import kotlin.io.path.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.exists
import kotlin.io.path.name

class InitCommand(
    private val paths: ToolkitPaths,
    private val runner: ProcessRunner,
) : CliktCommand(name = "init") {
    override fun help(context: Context): String = "Initialize project, extract resources, and run remap"

    private val project by option("--project", help = projectHelp)
    private val jar by option("--jar", help = "Path to source JAR (copied into project)").required()
    private val jarName by option("--as", "-n", help = "Destination JAR file name to use inside project ('.jar' added if missing)")
        .convert { raw ->
            val name = raw.trim()
            require(name.isNotBlank() && '/' !in name && '\\' !in name && name != "." && name != "..") {
                "provide a file name only (no path)"
            }
            if (name.lowercase().endsWith(".jar")) name else "$name.jar"
        }
    private val force by option("--force", help = "Overwrite files that init manages when the project already exists.").flag(default = false)

    override fun run() {
        val root = projectRoot(project)
        root.createDirectories()
        val mappings = root.resolve("mappings")
        mappings.createDirectories()

        val jarSrc = Path(jar).toAbsolutePath().normalize()
        require(jarSrc.exists()) { "Jar not found: $jarSrc" }

        val targetName = jarName ?: jarSrc.name

        val jarDst = root.resolve(targetName).toAbsolutePath().normalize()
        validateInitTargets(root, targetName, force, sourceJar = jarSrc)
        if (!sameFile(jarSrc, jarDst)) {
            Files.copy(jarSrc, jarDst, java.nio.file.StandardCopyOption.REPLACE_EXISTING)
        }

        val cfgPath = root.resolve("j2me.toml")
        val bytecode = bytecodeConfigFromJar(jarDst, source = "config")
        writeProjectConfig(cfgPath, targetName, bytecode)

        writeProjectGuidanceFiles(root, paths.mappingsDocTemplate)

        root.resolve(".cache").createDirectories()

        println("Initialized project: $root")
        println("Jar copied to: $jarDst")
        println("Config: $cfgPath")
        println("Bytecode: ${bytecode.version} (${bytecode.classCount ?: 0} classes), legacy target ${bytecode.javacTarget}")
        println("Running: extract-resources")
        runExtractResources(root)
        println("Running: remap")

        val global = loadToml(paths.globalCfg)
        val actualJar = resolveProjectJar(root)

        val remapArgs = buildRemapPipelineArgs(
            root = root,
            paths = paths,
            global = global,
            jar = actualJar,
            raw = false,
            noComments = false,
        )
        runRemapPipeline(remapArgs, runner)
    }
}

class RemapCommand(
    private val paths: ToolkitPaths,
    private val runner: ProcessRunner,
) : CliktCommand(name = "remap") {
    override fun help(context: Context): String = "Run remap pipeline (mapping outputs + remapped bytecode + decompile)"

    private val project by option("--project", help = projectHelp)
    private val noComments by option("--no-comments", help = "Disable decompiler comments.").flag(default = false)
    private val noSemanticMappings by option(
        "--no-semantic-mappings",
        help = "Disable semantic mappings while keeping class, member, and parameter-name mappings enabled.",
    ).flag(default = false)
    private val raw by option("--raw", help = "Bypass mappings entirely. Decompile raw bytecode and force Vineflower '--rename-members=true'.").flag(default = false)

    override fun run() {
        val root = projectRoot(project)
        val global = loadToml(paths.globalCfg)
        val jar = resolveProjectJar(root)

        val args = buildRemapPipelineArgs(
            root = root,
            paths = paths,
            global = global,
            jar = jar,
            raw = raw,
            noComments = noComments,
            semanticMappingsEnabled = !noSemanticMappings,
        )

        runRemapPipeline(args, runner)
    }
}

class ExtractResourcesCommand : CliktCommand(name = "extract-resources") {
    override fun help(context: Context): String = "Extract non-code JAR resources into project resources/"

    private val project by option("--project", help = projectHelp)
    override fun run() {
        runExtractResources(projectRoot(project))
    }
}

class DoctorCommand(
    private val paths: ToolkitPaths,
) : CliktCommand(name = "doctor") {
    override fun help(context: Context): String = "Show installation and binary path health"

    override fun run() {
        val global = loadToml(paths.globalCfg)
        val vineflowerBin = configuredDecompiler(paths, global)
        val vineflowerJavaBin = global.valueOrDefault("vineflower.java_bin", "java") { getString(it) }

        println("base: ${paths.base}")
        println("global config: ${paths.globalCfg}")
        println("remap engine: kotlin (ok)")

        if (vineflowerBin.isNotBlank()) {
            val vineflowerExists = binaryExists(vineflowerBin)
            val vineflowerJavaExists = binaryExists(vineflowerJavaBin)
            println("vineflower bin: $vineflowerBin (${if (vineflowerExists) "ok" else "missing"})")
            println("vineflower java: $vineflowerJavaBin (${if (vineflowerJavaExists) "ok" else "missing"})")
        }
    }
}

class CompileStubsCommand(
    private val paths: ToolkitPaths,
    private val runner: ProcessRunner,
) : CliktCommand(name = "compile-stubs") {
    override fun help(context: Context): String =
        "Special command mainly for Java decompiler development; NOT needed for normal remapping work, where j2me remap is sufficient. Compile decompiled sources against shared stubs"

    private val project by option("--project", help = projectHelp)
    private val stubsSrc by option("--stubs-src")
    private val stubsClassesDir by option("--stubs-classes-dir")
    private val skipStubCompile by option("--skip-stub-compile").flag(default = false)
    private val noStubCache by option("--no-stub-cache").flag(default = false)
    private val stubCacheDir by option("--stub-cache-dir")
    private val decompiledSrc by option("--decompiled-src")
    private val apiJarsDir by option("--api-jars-dir")
    private val outDir by option("--out-dir")
    private val compiler by option("--compiler", help = "Compiler backend: legacy (default), ecj, or javac.").default(CompileStubDefaults.backend.id)
    private val javaRelease by option("--java-release").int().default(CompileStubDefaults.javaRelease).check("must be positive") { it > 0 }
    private val maxerrs by option("--maxerrs").int().default(CompileStubDefaults.maxCompilerErrors).check("must be positive") { it > 0 }
    private val javacBin by option("--javac-bin").default(CompileStubDefaults.javacBin)
    private val javaBin by option("--java-bin").default(CompileStubDefaults.javaBin)
    private val compilerJar by option("--compiler-jar", help = "Override the selected jar-backed compiler.")
    private val sourceLevel by option("--source-level", help = "Source level for legacy/ecj backends; defaults to j2me.toml [bytecode].javac_source or inferred bytecode source.")
    private val targetLevel by option("--target-level", help = "Target level for legacy/ecj backends; legacy defaults to j2me.toml [bytecode].javac_target.")

    override fun run() {
        compileStubs(
            root = projectRoot(project),
            paths = paths,
            runner = runner,
            args = CompileStubsArgs(
                stubsSrcArg = stubsSrc,
                stubsClassesDirArg = stubsClassesDir,
                skipStubCompile = skipStubCompile,
                noStubCache = noStubCache,
                stubCacheDirArg = stubCacheDir,
                apiJarsDirArg = apiJarsDir,
                decompiledSrcArg = decompiledSrc,
                outDirArg = outDir,
                compiler = CompileBackend.parse(compiler),
                javaRelease = javaRelease,
                maxerrs = maxerrs,
                javacBin = javacBin,
                javaBin = javaBin,
                compilerJarArg = compilerJar,
                sourceLevel = sourceLevel,
                targetLevel = targetLevel,
            ),
        )
    }
}

class J2meCommand(
    private val paths: ToolkitPaths = defaultToolkitPaths(),
    private val runner: ProcessRunner = RealProcessRunner(),
) : CliktCommand(
    name = "j2me",
) {
    override fun help(context: Context): String = "Shared J2ME reversing utility."

    override fun run() = Unit

    init {
        configureContext {
            helpFormatter = { c -> PlaintextHelpFormatter(c) }
        }
        subcommands(
            InitCommand(paths, runner),
            RemapCommand(paths, runner),
            ExtractResourcesCommand(),
            DoctorCommand(paths),
            CompileStubsCommand(paths, runner),
            FullrunCommand(paths, runner),
        )
    }
}

fun runCli(args: Array<String>): Int {
    return try {
        J2meCommand().main(args)
        0
    } catch (exc: Exception) {
        printUserFacingError(exc)
        1
    }
}
