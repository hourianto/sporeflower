package j2me.bytecode

import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.validation.validateRealizedNames
import org.jetbrains.java.decompiler.api.EmittedClass
import org.jetbrains.java.decompiler.api.SourceMetadata
import org.jetbrains.java.decompiler.api.EmittedClass.Kind
import org.jetbrains.java.decompiler.api.NamingPlan
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.Type
import org.objectweb.asm.commons.ClassRemapper
import org.objectweb.asm.commons.Remapper
import org.objectweb.asm.tree.*
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.*

data class RestoredClasses(val directory: Path, val classCount: Int)

/** Restore a separate class directory. Application strings are never inferred from their spelling. */
fun restoreCompiledClasses(originalJar: Path, compiled: Path, sources: Path, output: Path,
                           libraries: Map<String, ClassSymbols> = emptyMap()): RestoredClasses {
    val metadata = SourceMetadata.read(sources.resolve(".sporeflower.json"))
    require(metadata.classNameStrings() == "original") {
        "Restoration requires sources generated with original class-name strings; regenerate the sources first"
    }
    val names = NamingPlan.read(sources.resolve(metadata.names()))
    val correspondence = metadata.classes().associateBy { it.name() }
    val original = readClasses(originalJar, debug = true)
    val rebuilt = readClasses(compiled, debug = true)
    val correspondenceMatcher = CompiledClassMatcher(names, original, rebuilt, correspondence)
    val compiledByEmitted = correspondenceMatcher.matchAll()
    val markers = correspondenceMatcher.markers
    val originalByEmitted = names.classes().entries.associate { it.value to it.key }
    val inverseClasses = compiledByEmitted.entries.associateTo(linkedMapOf()) { (emitted, compiledName) ->
        compiledName to originalByEmitted.getValue(emitted)
    }
    HelperPlacement(rebuilt, markers, inverseClasses).restoreAll()
    val inverse = inverseMemberNames(names, original, rebuilt, correspondence, compiledByEmitted, inverseClasses, libraries)
    val symbols = libraries + rebuilt.mapValues { it.value.symbols() }
    val remapper = CanonicalAsmRemapper(inverse, symbols)
    val results = linkedMapOf<String, ByteArray>()
    for (node in rebuilt.values) {
        restoreLegacyClassLiterals(node, inverseClasses)
        val writer = ClassWriter(0)
        node.accept(ClassRemapper(writer, remapper))
        val target = remapper.mapType(node.name)
        require(results.putIfAbsent(target, writer.toByteArray()) == null) { "Duplicate restored class: $target" }
    }
    // Constructor markers have no fields or methods. Preserve their original
    // class identity even when source reconstruction no longer needs the marker.
    for ((name, marker) in markers) {
        val writer = ClassWriter(0)
        marker.accept(writer)
        results[name] = writer.toByteArray()
    }
    val scratch = Files.createTempDirectory(output.toAbsolutePath().parent.also { it.createDirectories() }, ".restored-")
    try {
        for ((name, bytes) in results) {
            val file = scratch.resolve("$name.class").normalize()
            require(file.startsWith(scratch)) { "Unsafe restored class name: $name" }
            file.parent.createDirectories()
            file.writeBytes(bytes)
        }
        validateRestoredMembers(scratch)
        // Reuse the access checks for compiler-generated helpers, whose package may also have moved.
        validateRealizedNames(compiled, inverse, libraries)
        require(!output.exists()) { "Restoration output already exists: $output" }
        Files.move(scratch, output)
    } finally {
        if (scratch.exists()) Files.walk(scratch).use { it.sorted(Comparator.reverseOrder()).forEach(Files::delete) }
    }
    return RestoredClasses(output, results.size)
}

private class CompiledClassMatcher(
    private val names: NamingPlan,
    original: Map<String, ClassNode>,
    private val rebuilt: Map<String, ClassNode>,
    private val correspondence: Map<String, EmittedClass>,
) {
    val markers = original.filter { (name, node) ->
        node.fields.isEmpty() && node.methods.isEmpty() && node.interfaces.isEmpty() && node.superName == "java/lang/Object" &&
            names.classes()[name] !in correspondence && names.classes()[name] !in rebuilt
    }
    private val compiledByEmitted = linkedMapOf<String, String>()
    private val assigned = mutableSetOf<String>()
    private val allocationOwners = mutableMapOf<String, MutableSet<String>>()
    private val allocationLines = mutableMapOf<Pair<String, String>, MutableSet<Int>>()

    init {
        for (node in rebuilt.values) for (method in node.methods) {
            var line = -1
            for (instruction in method.instructions) {
                if (instruction is LineNumberNode) line = instruction.line
                if (instruction is TypeInsnNode && instruction.opcode == NEW) {
                    allocationOwners.getOrPut(instruction.desc) { linkedSetOf() }.add(node.name)
                    allocationLines.getOrPut(node.name to instruction.desc) { linkedSetOf() }.add(line)
                }
            }
        }
    }

    fun matchAll(): Map<String, String> {
        for ((name, emitted) in names.classes()) if (name !in markers) match(emitted, mutableSetOf())
        return compiledByEmitted
    }

    private fun match(emitted: String, visiting: MutableSet<String>): String {
        compiledByEmitted[emitted]?.let { return it }
        require(visiting.add(emitted)) { "Circular emitted class ownership: $emitted" }
        val declaration = correspondence[emitted]
        val candidates = if (declaration?.parent() != null) {
            val parent = match(declaration.parent(), visiting)
            val structural = rebuilt.values.filter { candidate ->
                if (candidate.name in assigned) return@filter false
                val inner = candidate.innerClasses.firstOrNull { it.name == candidate.name } ?: return@filter false
                val owner = candidate.outerClass ?: inner.outerName ?: allocationOwners[candidate.name]?.singleOrNull()
                if (owner != parent) return@filter false
                if (declaration.kind() == Kind.ANONYMOUS) inner.innerName.isNullOrEmpty()
                else inner.innerName == declaration.simpleName() &&
                    (declaration.kind() == Kind.MEMBER || candidate.outerMethod == null || declaration.enclosingMethod() == null ||
                        "${candidate.outerMethod} ${candidate.outerMethodDesc}" == declaration.enclosingMethod())
            }
            if (structural.size <= 1 || declaration.kind() == Kind.MEMBER) structural else {
                // Allocations in one multiline expression can share a line. A line
                // inside an actual method body distinguishes those anonymous classes.
                val bodies = structural.filter { candidate -> candidate.methods.any { method ->
                    !method.name.startsWith("<") && method.instructions.filterIsInstance<LineNumberNode>()
                        .any { it.line in declaration.line() + 1..declaration.endLine() }
                } }
                val located = bodies.ifEmpty { structural.filter { candidate ->
                    declaration.line() in allocationLines[parent to candidate.name].orEmpty() || candidate.methods.any { method ->
                        method.instructions.filterIsInstance<LineNumberNode>().any { it.line in declaration.line()..declaration.endLine() }
                    }
                } }
                require(located.isNotEmpty() || structural.any { candidate ->
                    allocationLines[parent to candidate.name].orEmpty().any { it > 0 } ||
                        candidate.methods.any { method -> method.instructions.any { it is LineNumberNode } }
                }) {
                    "Missing line-number data for $emitted or its enclosing class $parent; recompile with -g:lines,source"
                }
                located
            }
        } else listOfNotNull(rebuilt[emitted])
        require(candidates.size == 1) { "Missing or ambiguous compiled class for $emitted: ${candidates.map { it.name }}" }
        val found = candidates.single().name
        require(assigned.add(found)) { "Compiled class $found corresponds to more than one original class" }
        compiledByEmitted[emitted] = found
        visiting.remove(emitted)
        return found
    }
}

private class HelperPlacement(
    private val rebuilt: Map<String, ClassNode>,
    private val markers: Map<String, ClassNode>,
    private val inverseClasses: MutableMap<String, String>,
) {
    fun restoreAll() { for (name in rebuilt.keys) restoreHelper(name) }

    // Generated member helpers follow the owner recorded by the compiler, not a guessed '$' prefix.
    private fun markerHost(name: String): String? {
        val owners = linkedSetOf<String>()
        for (node in rebuilt.values) for (method in node.methods) {
            if (Type.getArgumentTypes(method.desc).none { it.descriptor == "L$name;" }) continue
            if (method.name != "<init>" || method.access and ACC_SYNTHETIC == 0) return null
            var host = node
            val visited = mutableSetOf<String>()
            while (visited.add(host.name)) {
                val parent = host.outerClass ?: host.innerClasses.firstOrNull { it.name == host.name }?.outerName ?: break
                host = rebuilt[parent] ?: break
            }
            owners += host.name
        }
        return owners.singleOrNull()
    }
    private fun restoreHelper(name: String, visiting: MutableSet<String> = mutableSetOf()): String {
        inverseClasses[name]?.let { return it }
        require(visiting.add(name)) { "Circular generated class ownership: $name" }
        val node = rebuilt.getValue(name)
        val inner = node.innerClasses.firstOrNull { it.name == name }
        val host = node.outerClass ?: inner?.outerName ?:
            if (node.fields.isEmpty() && node.methods.isEmpty()) markerHost(name) else null
        val base = if (host != null && host in rebuilt) {
            require(name.startsWith("$host$")) { "Generated class $name has an unexpected binary name for host $host" }
            restoreHelper(host, visiting) + name.removePrefix(host)
        } else name
        var target = base
        var suffix = 1
        while (target in inverseClasses.values || target in markers && (node.fields.isNotEmpty() || node.methods.isNotEmpty())) target = "${base}_${suffix++}"
        inverseClasses[name] = target
        visiting.remove(name)
        return target
    }
}

private fun inverseMemberNames(
    names: NamingPlan,
    original: Map<String, ClassNode>,
    rebuilt: Map<String, ClassNode>,
    correspondence: Map<String, EmittedClass>,
    compiledByEmitted: Map<String, String>,
    inverseClasses: Map<String, String>,
    libraries: Map<String, ClassSymbols>,
): CanonicalMap {
    val forwardTypes = object : Remapper(ASM9) {
        override fun map(name: String): String = names.classes()[name]?.let { compiledByEmitted[it] ?: it } ?: name
    }
    val fields = linkedMapOf<FieldSig, String>()
    val methods = linkedMapOf<MethodSig, String>()
    for ((key, name) in names.fields()) {
        val owner = forwardTypes.mapType(key.owner())
        val descriptor = forwardTypes.mapDesc(key.descriptor())
        val exists = rebuilt.getValue(owner).fields.any { it.name == name && it.desc == descriptor }
        val input = original.getValue(key.owner()).fields.single { it.name == key.name() && it.desc == key.descriptor() }
        val hidden = correspondence[names.classes().getValue(key.owner())]?.hiddenMembers().orEmpty()
        require(exists || input.access and ACC_SYNTHETIC != 0 || "$name ${forwardDescriptor(key.descriptor(), names)}" in hidden) {
            "Missing compiled field $owner.$name:$descriptor"
        }
        if (exists) fields[FieldSig(owner, name, descriptor)] = key.name()
    }
    for ((key, name) in names.methods()) {
        val owner = forwardTypes.mapType(key.owner())
        val descriptor = forwardTypes.mapMethodDesc(key.descriptor())
        val exists = rebuilt.getValue(owner).methods.any { it.name == name && it.desc == descriptor }
        val input = original.getValue(key.owner()).methods.single { it.name == key.name() && it.desc == key.descriptor() }
        // Initializers may disappear when their work becomes constant initializers; compiler accessors may be regenerated.
        val hidden = correspondence[names.classes().getValue(key.owner())]?.hiddenMembers().orEmpty()
        require(exists || input.access and (ACC_SYNTHETIC or ACC_BRIDGE) != 0 || key.name() == "<clinit>"
            || "$name ${forwardDescriptor(key.descriptor(), names)}" in hidden) {
            "Missing compiled method $owner.$name$descriptor"
        }
        if (exists) methods[MethodSig(owner, name, descriptor)] = key.name()
    }
    restoreGeneratedOverrides(rebuilt, methods, libraries)
    return CanonicalMap(classes = inverseClasses, fields = fields, methods = methods)
}

private fun restoreGeneratedOverrides(classes: Map<String, ClassNode>, names: MutableMap<MethodSig, String>,
                                      libraries: Map<String, ClassSymbols>) {
    val symbols = libraries + classes.mapValues { it.value.symbols() }
    for (node in classes.values) for (method in node.methods) {
        val key = MethodSig(node.name, method.name, method.desc)
        if (key in names || method.name.startsWith("<") || method.access and (ACC_STATIC or ACC_PRIVATE) != 0) continue
        val targets = linkedSetOf<String>()
        val visited = mutableSetOf<String>()
        fun visit(owner: String) {
            if (!visited.add(owner)) return
            val parent = symbols[owner] ?: return
            val inherited = MethodSig(owner, method.name, method.desc)
            val access = parent.methodAccess[inherited]
            if (access != null && access and (ACC_STATIC or ACC_PRIVATE) == 0 &&
                (access and (ACC_PUBLIC or ACC_PROTECTED) != 0 || owner.substringBeforeLast('/', "") == node.name.substringBeforeLast('/', ""))) {
                names[inherited]?.let(targets::add)
            }
            parent.superName?.let(::visit)
            parent.interfaces.forEach(::visit)
        }
        node.superName?.let(::visit)
        node.interfaces.forEach(::visit)
        require(targets.size <= 1) { "Ambiguous restored name for generated override $key: $targets" }
        targets.singleOrNull()?.let { names[key] = it }
    }
}

private fun forwardDescriptor(descriptor: String, names: NamingPlan): String {
    val remapper = object : Remapper(ASM9) {
        override fun map(name: String): String = names.classes()[name] ?: name
    }
    return if (descriptor.startsWith("(")) remapper.mapMethodDesc(descriptor) else remapper.mapDesc(descriptor)
}

private fun validateRestoredMembers(directory: Path) {
    for (node in readClasses(directory).values) {
        require(node.methods.map { it.name to it.desc }.distinct().size == node.methods.size) { "Restored method collision in ${node.name}" }
        require(node.fields.map { it.name to it.desc }.distinct().size == node.fields.size) { "Restored field collision in ${node.name}" }
    }
}

private fun restoreLegacyClassLiterals(owner: ClassNode, names: Map<String, String>) {
    val types = object : Remapper(ASM9) {
        override fun map(name: String): String = names[name] ?: name
    }
    val helpers = owner.methods.filter { method ->
        method.access and ACC_STATIC != 0 && method.desc == "(Ljava/lang/String;)Ljava/lang/Class;" &&
            method.tryCatchBlocks.any { it.type == "java/lang/ClassNotFoundException" } &&
            method.instructions.filterIsInstance<MethodInsnNode>().any { it.owner == "java/lang/Class" && it.name == "forName" }
    }.map { it.name }.toSet()
    if (helpers.isEmpty()) return
    for (method in owner.methods) for (call in method.instructions.filterIsInstance<MethodInsnNode>()) {
        if (call.opcode != INVOKESTATIC || call.owner != owner.name || call.name !in helpers) continue
        val literal = call.previous as? LdcInsnNode ?: continue
        val value = literal.cst as? String ?: continue
        val copy = call.next ?: continue
        val store = copy.next as? FieldInsnNode ?: continue
        if (copy.opcode != DUP || store.opcode != PUTSTATIC || store.owner != owner.name || store.desc != "Ljava/lang/Class;") continue
        val cache = owner.fields.singleOrNull { it.name == store.name && it.desc == store.desc } ?: continue
        if (cache.access and ACC_SYNTHETIC == 0) continue
        val type = value.replace('.', '/')
        val original = if (type.startsWith("[")) types.mapDesc(type) else types.mapType(type)
        literal.cst = original.replace('/', '.')
    }
}
