package j2me.cli

import j2me.common.isJavaClassFile
import org.tomlj.Toml
import org.tomlj.TomlParseResult
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipFile
import kotlin.io.path.Path
import kotlin.io.path.exists
import kotlin.io.path.writeText

internal const val projectHelp = "Project directory (default: current working directory)"

data class ToolkitPaths(
    val base: Path,
    val globalCfg: Path,
    val mappingsDocTemplate: Path,
    val bundledDecompiler: Path = base.resolve("decompiler/sporeflower.jar"),
)

internal fun defaultToolkitPaths(): ToolkitPaths {
    // Resolve relative to the installed CLI jar, never the user's working
    // directory. Gradle run and native launchers can supply an explicit home.
    val installation = System.getProperty("j2me.home")?.let(::Path) ?: run {
        if (System.getProperty("org.graalvm.nativeimage.imagecode") == "runtime") {
            Path(ProcessHandle.current().info().command().orElseThrow()).parent.parent
        } else {
            Path.of(ToolkitPaths::class.java.protectionDomain.codeSource.location.toURI()).parent.parent
        }
    }
    return toolkitPaths(installation, System.getenv("J2ME_BASE"), System.getenv("J2ME_CONFIG"))
}

internal fun toolkitPaths(installation: Path, baseOverride: String?, configOverride: String?): ToolkitPaths {
    val home = installation.toAbsolutePath().normalize()
    val base = baseOverride?.let(::Path)?.toAbsolutePath()?.normalize() ?: home
    return ToolkitPaths(
        base = base,
        globalCfg = configOverride?.let(::Path)?.toAbsolutePath()?.normalize() ?: base.resolve("config/global.toml"),
        mappingsDocTemplate = base.resolve("templates/mappings-doc.md"),
        bundledDecompiler = home.resolve("decompiler/sporeflower.jar"),
    )
}

internal fun configuredDecompiler(paths: ToolkitPaths, global: TomlParseResult?): String {
    val configured: String? = global?.getString("vineflower.bin")
    val override: String = System.getenv("SPOREFLOWER_JAR")?.takeIf { it.isNotBlank() }
        ?: configured?.takeIf { it.isNotBlank() }
        ?: return paths.bundledDecompiler.toString()
    val path = Path(override)
    return if (path.isAbsolute || (!override.endsWith(".jar", ignoreCase = true) && '/' !in override && '\\' !in override)) {
        override
    } else {
        paths.globalCfg.parent.resolve(path).normalize().toString()
    }
}

internal fun loadToml(path: Path): TomlParseResult? {
    if (!path.exists()) {
        return null
    }
    val parsed = Toml.parse(path)
    if (parsed.hasErrors()) {
        val errs = parsed.errors().joinToString("\n") { it.toString() }
        throw IllegalArgumentException("Failed to parse TOML: $path\n$errs")
    }
    return parsed
}

internal fun projectRoot(argProject: String?): Path =
    if (argProject.isNullOrBlank()) Path(".").toAbsolutePath().normalize() else Path(argProject).toAbsolutePath().normalize()

internal fun requireProjectCfg(root: Path): String {
    val cfgPath = root.resolve("j2me.toml")
    val cfg = loadToml(cfgPath) ?: throw IllegalArgumentException("Missing project config: $cfgPath\nRun: j2me init --jar <path>")
    val jar = cfg.getString("jar")?.trim().orEmpty()
    require(jar.isNotBlank()) { "Invalid $cfgPath: missing 'jar'" }
    return jar
}

internal fun resolveProjectJar(root: Path): Path {
    val jar = root.resolve(requireProjectCfg(root))
    require(jar.exists()) { "Jar configured in j2me.toml not found: $jar" }
    return jar
}

internal data class ClassVersion(val major: Int, val minor: Int) : Comparable<ClassVersion> {
    override fun compareTo(other: ClassVersion): Int =
        major.compareTo(other.major).takeIf { it != 0 } ?: minor.compareTo(other.minor)

    override fun toString(): String = "$major.$minor"
}

internal data class JavacLevels(val source: String, val target: String)

internal data class BytecodeProfile(
    val selected: ClassVersion,
    val classCount: Int,
)

internal data class ProjectBytecodeConfig(
    val version: ClassVersion,
    val javacTarget: String,
    val javacSource: String,
    val classCount: Int?,
    val source: String,
)

private fun readClassVersion(header: ByteArray): ClassVersion? {
    if (header.size < 8 || !isJavaClassFile(header)) {
        return null
    }
    val minor = ((header[4].toInt() and 0xff) shl 8) or (header[5].toInt() and 0xff)
    val major = ((header[6].toInt() and 0xff) shl 8) or (header[7].toInt() and 0xff)
    return ClassVersion(major = major, minor = minor)
}

internal fun inspectJarBytecode(jar: Path): BytecodeProfile {
    var selected: ClassVersion? = null
    var classCount = 0
    ZipFile(jar.toFile()).use { archive ->
        for (info in archive.entries()) {
            if (info.isDirectory || !info.name.endsWith(".class")) {
                continue
            }
            val version = archive.getInputStream(info).use { readClassVersion(it.readNBytes(8)) } ?: continue
            selected = selected?.let { maxOf(it, version) } ?: version
            classCount += 1
        }
    }
    val selectedVersion = requireNotNull(selected) { "No class files found in JAR: $jar" }
    return BytecodeProfile(
        selected = selectedVersion,
        classCount = classCount,
    )
}

internal fun javacLevelsForClassVersion(version: ClassVersion): JavacLevels =
    when {
        version.major <= 45 -> JavacLevels(source = "1.3", target = "1.1")
        version.major == 46 -> JavacLevels(source = "1.2", target = "1.2")
        version.major == 47 -> JavacLevels(source = "1.3", target = "1.3")
        else -> JavacLevels(source = "1.4", target = "1.4")
    }

internal fun javacTargetForClassVersion(version: ClassVersion): String = javacLevelsForClassVersion(version).target

internal fun javacSourceForClassVersion(version: ClassVersion): String = javacLevelsForClassVersion(version).source

internal fun tomlString(value: String): String =
    "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

internal fun loadProjectBytecodeConfig(root: Path): ProjectBytecodeConfig? {
    val cfgPath = root.resolve("j2me.toml")
    val cfg = loadToml(cfgPath) ?: return null
    val major = cfg.getLong("bytecode.major") ?: return null
    val minor = cfg.getLong("bytecode.minor") ?: return null
    val version = ClassVersion(major = major.toInt(), minor = minor.toInt())
    val inferredLevels = javacLevelsForClassVersion(version)
    return ProjectBytecodeConfig(
        version = version,
        javacTarget = cfg.getString("bytecode.javac_target") ?: inferredLevels.target,
        javacSource = cfg.getString("bytecode.javac_source") ?: inferredLevels.source,
        classCount = cfg.getLong("bytecode.class_count")?.toInt(),
        source = "config",
    )
}

internal fun bytecodeConfigFromJar(jar: Path, source: String): ProjectBytecodeConfig {
    val profile = inspectJarBytecode(jar)
    val levels = javacLevelsForClassVersion(profile.selected)
    return ProjectBytecodeConfig(
        version = profile.selected,
        javacTarget = levels.target,
        javacSource = levels.source,
        classCount = profile.classCount,
        source = source,
    )
}

internal fun writeProjectConfig(path: Path, jarName: String, bytecode: ProjectBytecodeConfig) {
    path.writeText(
        buildString {
            appendLine("jar = ${tomlString(jarName)}")
            appendLine()
            appendLine("[bytecode]")
            appendLine("major = ${bytecode.version.major}")
            appendLine("minor = ${bytecode.version.minor}")
            appendLine("class_count = ${bytecode.classCount ?: 0}")
            appendLine("javac_source = ${tomlString(bytecode.javacSource)}")
            appendLine("javac_target = ${tomlString(bytecode.javacTarget)}")
        },
    )
}

internal fun validateInitTargets(
    root: Path,
    jarName: String,
    force: Boolean,
    sourceJar: Path? = null,
) {
    if (force) {
        return
    }

    val jarDst = root.resolve(jarName).toAbsolutePath().normalize()
    val managedPaths = listOf(
        jarDst,
        root.resolve("j2me.toml"),
        root.resolve("AGENTS.md"),
        root.resolve("CLAUDE.md"),
    )
    val existing = managedPaths.filter { path ->
        path.exists() && !(path == jarDst && sourceJar != null && sameFile(sourceJar, jarDst))
    }
    require(existing.isEmpty()) {
        "init would overwrite existing file(s): ${existing.joinToString(", ") { root.relativize(it).toString() }}\n" +
            "Re-run with --force to replace files managed by init."
    }
}

internal fun sameFile(left: Path, right: Path): Boolean {
    val normalizedLeft = left.toAbsolutePath().normalize()
    val normalizedRight = right.toAbsolutePath().normalize()
    if (normalizedLeft == normalizedRight) {
        return true
    }
    return normalizedLeft.exists() && normalizedRight.exists() && Files.isSameFile(normalizedLeft, normalizedRight)
}

internal fun writeProjectGuidanceFiles(root: Path, mappingsDocTemplate: Path) {
    require(mappingsDocTemplate.exists()) { "Missing mappings doc template: $mappingsDocTemplate" }
    val agentsPath = root.resolve("AGENTS.md")
    Files.deleteIfExists(agentsPath)
    Files.createSymbolicLink(agentsPath, mappingsDocTemplate.toAbsolutePath().normalize())

    val claudePath = root.resolve("CLAUDE.md")
    Files.deleteIfExists(claudePath)
    Files.createSymbolicLink(claudePath, Path("AGENTS.md"))
}
