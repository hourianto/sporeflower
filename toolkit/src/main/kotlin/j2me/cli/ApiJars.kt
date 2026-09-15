package j2me.cli

import j2me.model.ClassSymbols
import j2me.symbols.collectSymbolsByClass
import org.jetbrains.java.decompiler.api.J2meApi
import java.nio.file.Path
import java.util.zip.ZipFile
import kotlin.io.path.isDirectory
import kotlin.io.path.isRegularFile
import kotlin.io.path.listDirectoryEntries
import kotlin.io.path.name
import kotlin.io.path.pathString

internal fun listApiJars(apiJarsDir: Path): List<Path> =
    if (!apiJarsDir.isDirectory()) emptyList() else apiJarsDir.listDirectoryEntries("*.jar").filter { it.isRegularFile() }.sortedBy { it.name }

internal fun localApiJars(paths: ToolkitPaths): List<Path> =
    listApiJars(paths.base.resolve("vendor/j2me-api"))

internal fun apiClassOwners(apiJars: List<Path>): Set<String> {
    val owners = linkedSetOf<String>()
    apiJars.forEach { jar ->
        ZipFile(jar.toFile()).use { zip ->
            zip.entries().asSequence()
                .filter { !it.isDirectory && it.name.endsWith(".class") && !it.name.startsWith("META-INF/") }
                .forEach { owners += it.name.removeSuffix(".class") }
        }
    }
    return owners
}

internal fun apiClassSymbols(api: J2meApi.Resolution, workers: Int): Map<String, ClassSymbols> =
    collectSymbolsByClass(api.classes(), api.classes().keys.toList(), workers)

internal data class LocalStubSources(
    val active: List<Path>,
    val shadowedByApiJar: List<Path>,
)

internal fun listLocalStubSources(stubsSrc: Path, apiJars: List<Path>): LocalStubSources {
    val sources = listJavaSources(stubsSrc)
    if (sources.isEmpty()) {
        return LocalStubSources(active = emptyList(), shadowedByApiJar = emptyList())
    }

    // Resolved API jars define the compile surface. Source stubs are a
    // fallback for missing APIs, so do not compile a local stub for a class that
    // is already present in the pinned jar set.
    val apiClasses = apiClassOwners(apiJars)
    val (shadowed, active) = sources.partition { source ->
        val relative = stubsSrc.relativize(source).pathString.replace(java.io.File.separatorChar, '/')
        relative.removeSuffix(".java") in apiClasses
    }
    return LocalStubSources(active = active, shadowedByApiJar = shadowed)
}
