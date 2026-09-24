package j2me.bytecode

import j2me.common.isJavaClassFile
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import org.objectweb.asm.ClassReader
import org.objectweb.asm.tree.ClassNode
import java.nio.file.Files
import java.nio.file.Path
import java.util.zip.ZipFile
import kotlin.io.path.isDirectory
import kotlin.io.path.readBytes

internal fun readClasses(path: Path, debug: Boolean = false): Map<String, ClassNode> {
    val classes = linkedMapOf<String, ClassNode>()
    fun add(bytes: ByteArray) {
        if (!isJavaClassFile(bytes)) return
        fun read(flags: Int) = ClassNode().also { ClassReader(bytes).accept(it, flags) }
        val node = try {
            read(if (debug) 0 else ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
        } catch (failure: IndexOutOfBoundsException) {
            if (!debug) throw failure
            try { read(ClassReader.SKIP_DEBUG) }
            catch (invalidCode: RuntimeException) { invalidCode.addSuppressed(failure); throw invalidCode }
        }
        require(classes.putIfAbsent(node.name, node) == null) { "Duplicate class: ${node.name}" }
    }
    if (path.isDirectory()) {
        Files.walk(path).use { files -> files.filter { it.toString().endsWith(".class") }.sorted().forEach { add(it.readBytes()) } }
    } else {
        ZipFile(path.toFile()).use { zip ->
            zip.entries().asSequence().filter { it.name.endsWith(".class") && !it.name.startsWith("META-INF/") }
                .forEach { add(zip.getInputStream(it).use { stream -> stream.readBytes() }) }
        }
    }
    return classes
}

internal fun ClassNode.symbols(): ClassSymbols {
    val fields = fields.associate { FieldSig(name, it.name, it.desc) to it.access }
    val methods = methods.associate { MethodSig(name, it.name, it.desc) to it.access }
    return ClassSymbols(fields.keys.toList(), methods.keys.toList(), methods, fields,
        superName = superName, interfaces = interfaces, access = access)
}
