package j2me.symbols

import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.common.isJavaClassFile
import j2me.common.parallelMap
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassVisitor
import org.objectweb.asm.MethodVisitor
import org.objectweb.asm.Opcodes
import org.objectweb.asm.tree.ClassNode
import java.io.IOException
import java.nio.file.Path
import java.util.zip.ZipFile

private fun parseClassBytes(bytes: ByteArray): ClassSymbols {
    val cn = ClassNode()
    ClassReader(bytes).accept(cn, ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)

    val owner = requireNotNull(cn.name) { "Class file has no internal name" }

    val fields = mutableListOf<FieldSig>()
    val fieldAccess = linkedMapOf<FieldSig, Int>()
    val fieldConstantValues = linkedMapOf<FieldSig, String>()
    for (node in cn.fields.orEmpty()) {
        val sig = FieldSig(owner, node.name, node.desc)
        fields += sig
        fieldAccess[sig] = node.access
        node.value?.let { fieldConstantValues[sig] = it.toString() }
    }

    val methods = mutableListOf<MethodSig>()
    val methodAccess = linkedMapOf<MethodSig, Int>()

    for (node in cn.methods.orEmpty()) {
        if (node.name == "<clinit>") {
            continue
        }
        val sig = MethodSig(owner, node.name, node.desc)
        methods += sig
        methodAccess[sig] = node.access
    }

    return ClassSymbols(
        fields = fields,
        methods = methods,
        methodAccess = methodAccess,
        fieldAccess = fieldAccess,
        fieldConstantValues = fieldConstantValues,
        superName = cn.superName,
        interfaces = cn.interfaces.orEmpty().map { it.toString() },
    )
}

fun parseClassSymbols(jarPath: Path, owner: String): ClassSymbols {
    val bytes = requireNotNull(readClassBytesByOwner(jarPath, listOf(owner))[owner]) {
        "Class not found in JAR: $owner ($jarPath)"
    }
    return parseClassBytes(bytes)
}

fun readClassBytesByOwner(jarPath: Path, classes: Collection<String>? = null): Map<String, ByteArray> {
    val wanted = classes?.toSet()
    if (wanted != null && wanted.isEmpty()) {
        return emptyMap()
    }

    val out = mutableMapOf<String, ByteArray>()
    try {
        ZipFile(jarPath.toFile()).use { zip ->
            for (entry in zip.entries()) {
                val name = entry.name
                if (entry.isDirectory || !name.endsWith(".class") || name.startsWith("META-INF/")) {
                    continue
                }
                val bytes = zip.getInputStream(entry).use { it.readBytes() }
                if (!isJavaClassFile(bytes)) {
                    continue
                }
                val owner = try {
                    ClassReader(bytes).className
                } catch (exc: IllegalArgumentException) {
                    throw IllegalArgumentException("Invalid class entry '$name' in JAR: $jarPath", exc)
                }
                if (wanted != null && owner !in wanted) {
                    continue
                }
                require(out.putIfAbsent(owner, bytes) == null) {
                    "JAR contains duplicate class definition: $owner ($jarPath)"
                }
                if (wanted != null && out.size >= wanted.size) {
                    break
                }
            }
        }
    } catch (exc: IOException) {
        throw IllegalArgumentException("Failed to read JAR as zip: $jarPath\n${exc.message}", exc)
    }
    return out
}

private fun parseClassUsage(bytes: ByteArray, jarClassSet: Set<String>): UsageStats {
    val reader = ClassReader(bytes)
    val callerOwner = reader.className

    val methodRefs = mutableMapOf<MethodSig, Int>()
    val methodCallers = mutableMapOf<MethodSig, MutableSet<String>>()
    val fieldReads = mutableMapOf<FieldSig, Int>()
    val fieldWrites = mutableMapOf<FieldSig, Int>()
    val fieldAccessors = mutableMapOf<FieldSig, MutableSet<String>>()

    reader.accept(
        object : ClassVisitor(Opcodes.ASM9) {
            override fun visitMethod(
                access: Int,
                name: String,
                descriptor: String,
                signature: String?,
                exceptions: Array<out String>?,
            ): MethodVisitor {
                val callerMethod = "$callerOwner.$name$descriptor"
                return object : MethodVisitor(Opcodes.ASM9) {
                    override fun visitMethodInsn(
                        opcode: Int,
                        owner: String,
                        name: String,
                        descriptor: String,
                        isInterface: Boolean,
                    ) {
                        if (owner in jarClassSet && name != "<init>" && name != "<clinit>") {
                            val sig = MethodSig(owner, name, descriptor)
                            methodRefs.increment(sig)
                            methodCallers.addValue(sig, callerMethod)
                        }

                        super.visitMethodInsn(opcode, owner, name, descriptor, isInterface)
                    }

                    override fun visitFieldInsn(opcode: Int, owner: String, name: String, descriptor: String) {
                        if (owner in jarClassSet) {
                            val sig = FieldSig(owner, name, descriptor)
                            val isRead = opcode == Opcodes.GETFIELD || opcode == Opcodes.GETSTATIC
                            (if (isRead) fieldReads else fieldWrites).increment(sig)
                            fieldAccessors.addValue(sig, callerMethod)
                        }

                        super.visitFieldInsn(opcode, owner, name, descriptor)
                    }
                }
            }
        },
        ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES,
    )

    return UsageStats(
        methodRefs = methodRefs,
        methodCallers = methodCallers,
        fieldReads = fieldReads,
        fieldWrites = fieldWrites,
        fieldAccessors = fieldAccessors,
    )
}

private fun <K> MutableMap<K, Int>.increment(key: K) {
    this[key] = (this[key] ?: 0) + 1
}

private fun <K, V> MutableMap<K, MutableSet<V>>.addValue(key: K, value: V) {
    getOrPut(key) { mutableSetOf() }.add(value)
}

data class UsageStats(
    val methodRefs: Map<MethodSig, Int> = emptyMap(),
    val methodCallers: Map<MethodSig, Set<String>> = emptyMap(),
    val fieldReads: Map<FieldSig, Int> = emptyMap(),
    val fieldWrites: Map<FieldSig, Int> = emptyMap(),
    val fieldAccessors: Map<FieldSig, Set<String>> = emptyMap(),
)

fun collectSymbolsByClass(
    jarPath: Path,
    classes: List<String>,
    workers: Int,
): Map<String, ClassSymbols> {
    val classBytesByOwner = readClassBytesByOwner(jarPath, classes)
    return collectSymbolsByClass(classBytesByOwner, classes, workers)
}

fun collectSymbolsByClass(
    classBytesByOwner: Map<String, ByteArray>,
    classes: List<String>,
    workers: Int,
): Map<String, ClassSymbols> {
    val parsed = parallelMap(workers, classes) { owner ->
        val bytes = classBytesByOwner[owner]
        val symbols = if (bytes == null) {
            ClassSymbols(emptyList(), emptyList())
        } else {
            parseClassBytes(bytes)
        }
        owner to symbols
    }
    return linkedMapOf<String, ClassSymbols>().apply { parsed.forEach { (owner, symbols) -> this[owner] = symbols } }
}

fun collectSymbolUsage(
    jarPath: Path,
    classes: List<String>,
    workers: Int,
    symbolsByClass: Map<String, ClassSymbols>? = null,
): UsageStats {
    val classBytesByOwner = readClassBytesByOwner(jarPath, classes)
    return collectSymbolUsage(classBytesByOwner, classes, workers, symbolsByClass)
}

private fun <K> MutableMap<K, Int>.mergeCounts(other: Map<K, Int>) {
    other.forEach { (key, count) -> this[key] = (this[key] ?: 0) + count }
}

private fun <K, V> MutableMap<K, MutableSet<V>>.mergeSets(other: Map<K, Set<V>>) {
    other.forEach { (key, values) -> getOrPut(key) { mutableSetOf() }.addAll(values) }
}

fun collectSymbolUsage(
    classBytesByOwner: Map<String, ByteArray>,
    classes: List<String>,
    workers: Int,
    symbolsByClass: Map<String, ClassSymbols>? = null,
): UsageStats {
    val jarClassSet = classes.toSet()
    val all = parallelMap(workers, classes) { owner ->
        val bytes = classBytesByOwner[owner]
        bytes?.let { parseClassUsage(it, jarClassSet) }
    }.filterNotNull()

    val methodRefs = mutableMapOf<MethodSig, Int>()
    val methodCallers = mutableMapOf<MethodSig, MutableSet<String>>()
    val fieldReads = mutableMapOf<FieldSig, Int>()
    val fieldWrites = mutableMapOf<FieldSig, Int>()
    val fieldAccessors = mutableMapOf<FieldSig, MutableSet<String>>()

    for (usage in all) {
        methodRefs.mergeCounts(usage.methodRefs)
        methodCallers.mergeSets(usage.methodCallers)
        fieldReads.mergeCounts(usage.fieldReads)
        fieldWrites.mergeCounts(usage.fieldWrites)
        fieldAccessors.mergeSets(usage.fieldAccessors)
    }

    val rawUsage = UsageStats(
        methodRefs = methodRefs,
        methodCallers = methodCallers,
        fieldReads = fieldReads,
        fieldWrites = fieldWrites,
        fieldAccessors = fieldAccessors,
    )
    val declarations = symbolsByClass ?: collectSymbolsByClass(classBytesByOwner, classes, workers)
    return normalizeUsageOwners(rawUsage, declarations)
}

private fun normalizeUsageOwners(
    usage: UsageStats,
    symbolsByClass: Map<String, ClassSymbols>,
): UsageStats {
    val resolver = MemberResolver(symbolsByClass)

    val methodRefs = mutableMapOf<MethodSig, Int>()
    val methodCallers = mutableMapOf<MethodSig, MutableSet<String>>()
    for ((sig, count) in usage.methodRefs) {
        val resolved = resolver.method(sig)
        methodRefs[resolved] = (methodRefs[resolved] ?: 0) + count
    }
    for ((sig, callers) in usage.methodCallers) {
        methodCallers.getOrPut(resolver.method(sig)) { mutableSetOf() }.addAll(callers)
    }

    val fieldReads = mutableMapOf<FieldSig, Int>()
    val fieldWrites = mutableMapOf<FieldSig, Int>()
    val fieldAccessors = mutableMapOf<FieldSig, MutableSet<String>>()
    for ((sig, count) in usage.fieldReads) {
        val resolved = resolver.field(sig)
        fieldReads[resolved] = (fieldReads[resolved] ?: 0) + count
    }
    for ((sig, count) in usage.fieldWrites) {
        val resolved = resolver.field(sig)
        fieldWrites[resolved] = (fieldWrites[resolved] ?: 0) + count
    }
    for ((sig, accessors) in usage.fieldAccessors) {
        fieldAccessors.getOrPut(resolver.field(sig)) { mutableSetOf() }.addAll(accessors)
    }

    return UsageStats(methodRefs, methodCallers, fieldReads, fieldWrites, fieldAccessors)
}
