package j2me.bytecode

import j2me.common.isJavaClassFile
import j2me.common.mappedClassName
import j2me.common.validateClassName
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.symbols.MemberResolver
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.commons.ClassRemapper
import org.objectweb.asm.commons.Remapper
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import java.nio.file.Path
import java.util.jar.JarEntry
import java.util.jar.JarOutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipFile
import kotlin.io.path.createDirectories

data class RemappedJarStats(
    val path: Path,
    val classCount: Int,
    val resourceCount: Int,
)

fun defaultRemappedJarPath(inputJar: Path, outDir: Path): Path {
    val fileName = inputJar.fileName.toString()
    val remappedName = if (fileName.endsWith(".jar", ignoreCase = true)) {
        "${fileName.dropLast(4)}_remapped.jar"
    } else {
        "${fileName}_remapped.jar"
    }
    return outDir.resolve(remappedName)
}

fun remapJarBytecode(
    inputJar: Path,
    outputJar: Path,
    mappings: CanonicalMap,
    symbolsByClass: Map<String, ClassSymbols>,
): RemappedJarStats {
    outputJar.parent?.createDirectories()
    val remapper = CanonicalAsmRemapper(mappings, symbolsByClass)
    val seenEntries = linkedSetOf<String>()
    var classCount = 0
    var resourceCount = 0

    ZipFile(inputJar.toFile()).use { zip ->
        JarOutputStream(Files.newOutputStream(outputJar)).use { out ->
            for (entry in zip.entries().asSequence()) {
                if (entry.isDirectory) {
                    continue
                }
                val inputBytes = zip.getInputStream(entry).use { it.readBytes() }
                val isClassEntry = entry.name.endsWith(".class") &&
                    !entry.name.startsWith("META-INF/") &&
                    isJavaClassFile(inputBytes)
                val (entryName, outputBytes) = if (isClassEntry) {
                    val reader = ClassReader(inputBytes)
                    val writer = ClassWriter(0)
                    reader.accept(ClassRemapper(writer, remapper), 0)
                    classCount += 1
                    "${remapper.map(reader.className)}.class" to writer.toByteArray()
                } else {
                    resourceCount += 1
                    entry.name to inputBytes
                }

                require(seenEntries.add(entryName)) {
                    "Remapped JAR would contain duplicate entry: $entryName"
                }

                out.putNextEntry(copyJarEntry(entry, entryName))
                out.write(outputBytes)
                out.closeEntry()
            }
        }
    }

    return RemappedJarStats(outputJar, classCount, resourceCount)
}

private class CanonicalAsmRemapper(
    private val mappings: CanonicalMap,
    symbolsByClass: Map<String, ClassSymbols>,
) : Remapper(Opcodes.ASM9) {
    private val projectOwners = symbolsByClass.keys
    private val members = MemberResolver(symbolsByClass)

    override fun map(internalName: String?): String? {
        if (internalName == null) return null
        val emittedName = mappedClassName(internalName, mappings)
        return if (internalName in mappings.classes || (internalName in projectOwners && validateClassName(emittedName))) {
            emittedName
        } else {
            internalName
        }
    }

    override fun mapFieldName(owner: String?, name: String?, descriptor: String?): String {
        if (owner == null || name == null || descriptor == null) return name.orEmpty()
        return mappings.fields[members.field(FieldSig(owner, name, descriptor))] ?: name
    }

    override fun mapMethodName(owner: String?, name: String?, descriptor: String?): String {
        if (owner == null || name == null || descriptor == null) return name.orEmpty()
        return mappings.methods[members.method(MethodSig(owner, name, descriptor))] ?: name
    }
}

private fun copyJarEntry(source: ZipEntry, name: String): JarEntry =
    JarEntry(name).also { target ->
        target.comment = source.comment
        target.extra = source.extra
        target.time = source.time
    }
