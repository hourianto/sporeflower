package j2me.validation

import j2me.bytecode.CanonicalAsmRemapper
import j2me.bytecode.readClasses
import j2me.bytecode.symbols
import j2me.common.internalPackageName
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.symbols.MemberResolver
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.Type
import org.objectweb.asm.tree.*
import org.objectweb.asm.tree.analysis.Analyzer
import org.objectweb.asm.tree.analysis.SourceInterpreter
import java.nio.file.Path

/** Validate the completed names, including changes caused solely by package placement. */
internal fun validateRealizedNames(input: Path, names: CanonicalMap, libraries: Map<String, ClassSymbols>) {
    val classes = readClasses(input)
    val original = libraries + classes.mapValues { it.value.symbols() }
    val remapper = CanonicalAsmRemapper(names, original)
    val emitted = original.map { (owner, symbols) ->
        val fields = symbols.fields.associateWith { FieldSig(remapper.mapType(owner), remapper.mapFieldName(owner, it.name, it.desc), remapper.mapDesc(it.desc)) }
        val methods = symbols.methods.associateWith { MethodSig(remapper.mapType(owner), remapper.mapMethodName(owner, it.name, it.desc), remapper.mapMethodDesc(it.desc)) }
        remapper.mapType(owner) to symbols.copy(
            fields = fields.values.toList(), methods = methods.values.toList(),
            fieldAccess = fields.entries.associate { it.value to (symbols.fieldAccess[it.key] ?: 0) },
            methodAccess = methods.entries.associate { it.value to (symbols.methodAccess[it.key] ?: 0) },
            superName = symbols.superName?.let(remapper::mapType), interfaces = symbols.interfaces.map(remapper::mapType),
        )
    }.toMap()
    require(emitted.size == original.size) { "Completed class names collide with an application or API class" }
    val before = MemberResolver(original)
    val after = MemberResolver(emitted)
    val issues = linkedSetOf<String>()
    val ancestors = mutableMapOf<String, Set<String>>()
    fun ancestry(owner: String): Set<String> = ancestors.getOrPut(owner) {
        val seen = linkedSetOf<String>()
        val pending = ArrayDeque(listOf(owner))
        while (pending.isNotEmpty()) {
            val next = pending.removeFirst()
            if (!seen.add(next)) continue
            original[next]?.let { pending.addAll(it.interfaces); it.superName?.let(pending::add) }
        }
        seen
    }
    fun samePackage(a: String, b: String, mapped: Boolean): Boolean =
        internalPackageName(if (mapped) remapper.mapType(a) else a) == internalPackageName(if (mapped) remapper.mapType(b) else b)

    fun classAccess(caller: String, target: String) {
        val type = if (target.startsWith("[")) Type.getType(target).elementType else Type.getObjectType(target)
        if (type.sort != Type.OBJECT) return
        val name = type.internalName
        val access = original[name]?.access ?: return
        if (access and ACC_PUBLIC == 0 && samePackage(caller, name, false) && !samePackage(caller, name, true)) {
            issues += "$caller loses access to class $name (${remapper.mapType(caller)} -> ${remapper.mapType(name)})"
        }
    }
    fun memberAccess(caller: String, owner: String, member: String, access: Int, symbolicOwner: String, receiverIsThis: () -> Boolean) {
        if (!samePackage(caller, owner, false) || samePackage(caller, owner, true) || access and (ACC_PUBLIC or ACC_PRIVATE) != 0) return
        // Outside its package, a protected instance access also constrains the receiver.
        val protectedAccess = access and ACC_PROTECTED != 0 && owner in ancestry(caller) &&
            (access and ACC_STATIC != 0 || caller in ancestry(symbolicOwner) || receiverIsThis())
        if (!protectedAccess) issues += "$caller loses package access to $owner.$member"
    }
    for ((owner, node) in classes) {
        node.superName?.let { classAccess(owner, it) }
        node.interfaces.forEach { classAccess(owner, it) }
        for (method in node.methods) {
            val frames by lazy { runCatching { Analyzer(SourceInterpreter()).analyze(owner, method) }.getOrNull() }
            fun receiverIsThis(insn: AbstractInsnNode, arguments: Int): Boolean {
                if (method.access and ACC_STATIC != 0) return false
                val frame = frames?.get(method.instructions.indexOf(insn)) ?: return false
                val value = frame.getStack(frame.stackSize - arguments - 1)
                return value.insns.isNotEmpty() && value.insns.all { it is VarInsnNode && it.opcode == ALOAD && it.`var` == 0 }
            }
            for (insn in method.instructions) {
                when (insn) {
                    is TypeInsnNode -> classAccess(owner, insn.desc)
                    is LdcInsnNode -> (insn.cst as? Type)?.takeIf { it.sort == Type.OBJECT || it.sort == Type.ARRAY }?.let { classAccess(owner, it.internalName) }
                    is FieldInsnNode -> {
                        classAccess(owner, insn.owner)
                        val declaration = before.field(FieldSig(insn.owner, insn.name, insn.desc))
                        val access = original[declaration.owner]?.fieldAccess?.get(declaration) ?: continue
                        memberAccess(owner, declaration.owner, declaration.name, access, insn.owner) {
                            receiverIsThis(insn, if (insn.opcode == PUTFIELD) 1 else 0)
                        }
                        val expected = FieldSig(remapper.mapType(declaration.owner), remapper.mapFieldName(declaration.owner, declaration.name, declaration.desc), remapper.mapDesc(declaration.desc))
                        val actual = after.field(expected.copy(owner = remapper.mapType(insn.owner)))
                        if (actual != expected) issues += "$owner.${method.name}: field $declaration would bind to $actual"
                    }
                    is MethodInsnNode -> {
                        classAccess(owner, insn.owner)
                        val declaration = before.method(MethodSig(insn.owner, insn.name, insn.desc))
                        val access = original[declaration.owner]?.methodAccess?.get(declaration) ?: continue
                        memberAccess(owner, declaration.owner, declaration.name + declaration.desc, access, insn.owner) {
                            receiverIsThis(insn, Type.getArgumentTypes(insn.desc).size)
                        }
                        if (insn.opcode == INVOKESTATIC) {
                            val expected = MethodSig(remapper.mapType(declaration.owner), remapper.mapMethodName(declaration.owner, declaration.name, declaration.desc), remapper.mapMethodDesc(declaration.desc))
                            val actual = after.method(expected.copy(owner = remapper.mapType(insn.owner)))
                            if (actual != expected) issues += "$owner.${method.name}: static method $declaration would bind to $actual"
                        }
                    }
                }
            }
        }
    }
    val methods = original.values.flatMap { symbols -> symbols.methods.filter {
        !it.name.startsWith("<") && (symbols.methodAccess[it] ?: 0) and (ACC_STATIC or ACC_PRIVATE) == 0
    } }
    for (group in methods.groupBy { it.desc }.values) {
        val caches = arrayOf(mutableMapOf<Pair<MethodSig, MethodSig>, Boolean>(), mutableMapOf<Pair<MethodSig, MethodSig>, Boolean>())
        fun name(method: MethodSig, mapped: Boolean) = if (mapped) remapper.mapMethodName(method.owner, method.name, method.desc) else method.name
        fun overrides(child: MethodSig, parent: MethodSig, mapped: Boolean): Boolean {
            if (child.owner == parent.owner || name(child, mapped) != name(parent, mapped) || parent.owner !in ancestry(child.owner)) return false
            val cache = caches[if (mapped) 1 else 0]
            val key = child to parent
            cache[key]?.let { return it }
            cache[key] = false
            val access = original.getValue(parent.owner).methodAccess[parent] ?: 0
            if (access and ACC_FINAL != 0) return false
            // A public intermediate override can carry a package-private family across packages.
            val result = access and (ACC_PUBLIC or ACC_PROTECTED) != 0 || samePackage(child.owner, parent.owner, mapped) ||
                group.any { middle -> overrides(child, middle, mapped) && overrides(middle, parent, mapped) }
            cache[key] = result
            return result
        }
        fun related(a: MethodSig, b: MethodSig, mapped: Boolean): Boolean {
            if (overrides(a, b, mapped) || overrides(b, a, mapped)) return true
            val aInterface = original.getValue(a.owner).access and ACC_INTERFACE != 0
            val bInterface = original.getValue(b.owner).access and ACC_INTERFACE != 0
            if (aInterface == bInterface || name(a, mapped) != name(b, mapped)) return false
            val implementation = if (aInterface) b else a
            if ((original.getValue(implementation.owner).methodAccess[implementation] ?: 0) and ACC_PUBLIC == 0) return false
            return classes.keys.any { a.owner in ancestry(it) && b.owner in ancestry(it) }
        }
        val candidates = linkedSetOf<Pair<MethodSig, MethodSig>>()
        for (mapped in listOf(false, true)) for (bucket in group.groupBy { name(it, mapped) }.values) {
            for (i in bucket.indices) for (j in i + 1 until bucket.size) candidates += bucket[i] to bucket[j]
        }
        for ((a, b) in candidates) {
            if (a.owner == b.owner || a.owner !in classes && b.owner !in classes) continue
            if (related(a, b, false) != related(a, b, true)) issues += "Override relationship changes: $a and $b"
        }
    }
    if (issues.isNotEmpty()) throw MappingValidationException(issues.map { ValidationIssue(it) })
}
