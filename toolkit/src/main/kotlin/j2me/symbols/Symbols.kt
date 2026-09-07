package j2me.symbols

import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.common.isJavaClassFile
import j2me.common.parallelMap
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassVisitor
import org.objectweb.asm.FieldVisitor
import org.objectweb.asm.MethodVisitor
import org.objectweb.asm.Opcodes
import java.io.IOException
import java.nio.file.Path
import java.util.zip.ZipFile

private data class ClassFacts(val symbols: ClassSymbols?, val usage: UsageStats?)

private fun readClassFacts(bytes: ByteArray, collectSymbols: Boolean, usageOwners: Set<String>?): ClassFacts {
    var instructionOffset = -1
    val reader = object : ClassReader(bytes) {
        override fun readBytecodeInstructionOffset(bytecodeOffset: Int) { instructionOffset = bytecodeOffset }
    }
    val classOwner = reader.className
    val fields = mutableListOf<FieldSig>()
    val methods = mutableListOf<MethodSig>()
    val fieldAccess = linkedMapOf<FieldSig, Int>()
    val methodAccess = linkedMapOf<MethodSig, Int>()
    val constants = linkedMapOf<FieldSig, String>()
    val calls = linkedMapOf<MethodSig, MutableMap<Int, MethodSig>>()
    val usage = usageOwners?.let { UsageAccumulator() }
    reader.accept(object : ClassVisitor(Opcodes.ASM9) {
        override fun visitField(access: Int, name: String, descriptor: String, signature: String?, value: Any?): FieldVisitor? {
            if (collectSymbols) {
                val field = FieldSig(classOwner, name, descriptor)
                fields += field
                fieldAccess[field] = access
                value?.let { constants[field] = it.toString() }
            }
            return null
        }

        override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor {
            val method = MethodSig(classOwner, name, descriptor)
            val recordCalls = collectSymbols && name != "<clinit>"
            if (recordCalls) {
                methods += method
                methodAccess[method] = access
            }
            val caller = "$classOwner.$name$descriptor"
            return object : MethodVisitor(Opcodes.ASM9) {
                override fun visitMethodInsn(opcode: Int, owner: String, name: String, descriptor: String, isInterface: Boolean) {
                    val callee = MethodSig(owner, name, descriptor)
                    if (recordCalls) calls.getOrPut(method) { linkedMapOf() }[instructionOffset] = callee
                    if (usage != null && owner in usageOwners && name != "<init>" && name != "<clinit>") {
                        usage.methodRefs.increment(callee)
                        usage.methodCallers.addValue(callee, caller)
                    }
                }

                override fun visitFieldInsn(opcode: Int, owner: String, name: String, descriptor: String) {
                    if (usage != null && owner in usageOwners) {
                        val field = FieldSig(owner, name, descriptor)
                        val counts = if (opcode == Opcodes.GETFIELD || opcode == Opcodes.GETSTATIC) usage.fieldReads else usage.fieldWrites
                        counts.increment(field)
                        usage.fieldAccessors.addValue(field, caller)
                    }
                }
            }
        }
    }, ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
    val symbols = if (collectSymbols) ClassSymbols(fields, methods, methodAccess, fieldAccess, constants,
        reader.superName, reader.interfaces.toList(), calls) else null
    return ClassFacts(symbols, usage?.snapshot())
}

fun parseClassSymbols(jarPath: Path, owner: String): ClassSymbols {
    val bytes = requireNotNull(readClassBytesByOwner(jarPath, listOf(owner))[owner]) {
        "Class not found in JAR: $owner ($jarPath)"
    }
    return requireNotNull(readClassFacts(bytes, collectSymbols = true, usageOwners = null).symbols)
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
): Map<String, ClassSymbols> = collectJarFacts(classBytesByOwner, classes, workers, includeUsage = false).symbolsByClass

fun collectSymbolUsage(
    jarPath: Path,
    classes: List<String>,
    workers: Int,
    symbolsByClass: Map<String, ClassSymbols>? = null,
): UsageStats {
    val classBytesByOwner = readClassBytesByOwner(jarPath, classes)
    return collectSymbolUsage(classBytesByOwner, classes, workers, symbolsByClass)
}

fun collectSymbolUsage(
    classBytesByOwner: Map<String, ByteArray>,
    classes: List<String>,
    workers: Int,
    symbolsByClass: Map<String, ClassSymbols>? = null,
): UsageStats = collectJarFacts(classBytesByOwner, classes, workers, symbolsByClass, includeUsage = true).usage

/** Cold caches collect declarations and usage in the same streaming bytecode visit. */
internal fun collectJarFacts(
    bytes: Map<String, ByteArray>,
    classes: List<String>,
    workers: Int,
    cachedSymbols: Map<String, ClassSymbols>? = null,
    includeUsage: Boolean,
): JarAnalysis {
    val usageOwners = classes.toSet().takeIf { includeUsage }
    val parsed = parallelMap(workers, classes) { owner ->
        bytes[owner]?.let { readClassFacts(it, cachedSymbols == null, usageOwners) }
    }
    val symbols = cachedSymbols ?: classes.zip(parsed).associateTo(linkedMapOf()) { (owner, facts) ->
        owner to (facts?.symbols ?: ClassSymbols(emptyList(), emptyList()))
    }
    val usage = UsageAccumulator()
    if (includeUsage) {
        val resolver = MemberResolver(symbols)
        parsed.forEach { facts -> facts?.usage?.let { usage.merge(it, resolver) } }
    }
    return JarAnalysis(symbols, usage.snapshot())
}

private class UsageAccumulator {
    val methodRefs = mutableMapOf<MethodSig, Int>()
    val methodCallers = mutableMapOf<MethodSig, MutableSet<String>>()
    val fieldReads = mutableMapOf<FieldSig, Int>()
    val fieldWrites = mutableMapOf<FieldSig, Int>()
    val fieldAccessors = mutableMapOf<FieldSig, MutableSet<String>>()

    fun snapshot() = UsageStats(methodRefs, methodCallers, fieldReads, fieldWrites, fieldAccessors)

    fun merge(usage: UsageStats, resolver: MemberResolver) {
        methodRefs.mergeCounts(usage.methodRefs, resolver::method)
        methodCallers.mergeSets(usage.methodCallers, resolver::method)
        fieldReads.mergeCounts(usage.fieldReads, resolver::field)
        fieldWrites.mergeCounts(usage.fieldWrites, resolver::field)
        fieldAccessors.mergeSets(usage.fieldAccessors, resolver::field)
    }
}

private fun <K> MutableMap<K, Int>.mergeCounts(other: Map<K, Int>, resolve: (K) -> K) {
    other.forEach { (key, count) ->
        val resolved = resolve(key)
        this[resolved] = (this[resolved] ?: 0) + count
    }
}

private fun <K, V> MutableMap<K, MutableSet<V>>.mergeSets(other: Map<K, Set<V>>, resolve: (K) -> K) {
    other.forEach { (key, values) -> getOrPut(resolve(key)) { mutableSetOf() }.addAll(values) }
}
