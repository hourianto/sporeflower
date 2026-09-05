package j2me.bytecode

import j2me.common.isJavaClassFile
import j2me.common.mappedClassName
import j2me.model.*
import j2me.symbols.MemberResolver
import org.objectweb.asm.ClassReader
import org.objectweb.asm.Opcodes.*
import org.objectweb.asm.Type
import org.objectweb.asm.tree.*
import org.objectweb.asm.tree.analysis.Analyzer
import org.objectweb.asm.tree.analysis.AnalyzerException
import org.objectweb.asm.tree.analysis.SourceInterpreter
import org.objectweb.asm.tree.analysis.SourceValue
import java.nio.file.Path
import java.util.IdentityHashMap
import java.util.zip.ZipFile

data class ClassNameLiteral(val target: SemanticTarget, val offset: Int, val original: String, val replacement: String)
data class ClassNameRemapping(val literals: List<ClassNameLiteral>, val warnings: List<String>)

/** Resolve once, against original instructions, for both the JAR and the source renderer. */
fun resolveClassNameRemapping(
    jar: Path,
    mappings: CanonicalMap,
    symbols: Map<String, ClassSymbols>,
    contracts: Set<SemanticTarget> = emptySet(),
): ClassNameRemapping {
    val resolver = MemberResolver(symbols)
    fun bound(target: SemanticTarget): Boolean = target in contracts || when (target) {
        is SemanticTarget.Field -> SemanticTarget.Field(resolver.field(target.field)) in contracts
        is SemanticTarget.Return -> SemanticTarget.Return(resolver.method(target.method)) in contracts
        is SemanticTarget.Parameter -> SemanticTarget.Parameter(target.parameter.copy(method = resolver.method(target.parameter.method))) in contracts
    }
    fun field(insn: FieldInsnNode) = SemanticTarget.Field(FieldSig(insn.owner, insn.name, insn.desc))
    fun method(insn: MethodInsnNode) = MethodSig(insn.owner, insn.name, insn.desc)
    fun parameter(insn: MethodInsnNode, index: Int): Boolean =
        (index == 0 && insn.opcode == INVOKESTATIC && insn.owner == "java/lang/Class" && insn.name == "forName"
            && insn.desc in setOf("(Ljava/lang/String;)Ljava/lang/Class;", "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;")) ||
            bound(SemanticTarget.Parameter(MethodParameterSig(method(insn), index)))
    fun replacement(text: String): String? {
        val dimensions = text.takeWhile { it == '[' }.length
        val array = dimensions > 0
        if (array && (text.getOrNull(dimensions) != 'L' || !text.endsWith(';'))) return null
        val name = if (array) text.substring(dimensions + 1, text.length - 1) else text
        // Class.forName uses binary names, not internal JVM names or resource paths.
        if ('/' in name) return null
        val owner = name.replace('.', '/')
        if (owner !in symbols) return null
        val renamed = mappedClassName(owner, mappings).replace('/', '.')
        val result = if (array) "[".repeat(dimensions) + "L$renamed;" else renamed
        return result.takeUnless { it == text }
    }
    val literals = mutableListOf<ClassNameLiteral>()
    val warnings = linkedSetOf<String>()
    ZipFile(jar.toFile()).use { zip ->
        for (entry in zip.entries().asSequence()) {
            if (!entry.name.endsWith(".class") || entry.name.startsWith("META-INF/")) continue
            val bytes = zip.getInputStream(entry).use { it.readBytes() }
            if (!isJavaClassFile(bytes)) continue
            var offset = -1
            val offsets = IdentityHashMap<AbstractInsnNode, Int>()
            val node = object : ClassNode(ASM9) {
                override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodNode {
                    val result = object : MethodNode(ASM9, access, name, descriptor, signature, exceptions) {
                        override fun visitLdcInsn(value: Any) {
                            super.visitLdcInsn(value)
                            offsets[instructions.last] = offset
                        }
                    }
                    methods.add(result)
                    return result
                }
            }
            val reader = object : ClassReader(bytes) {
                override fun readBytecodeInstructionOffset(bytecodeOffset: Int) { offset = bytecodeOffset }
            }
            reader.accept(node, ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
            for (decl in node.fields) {
                val target = SemanticTarget.Field(FieldSig(node.name, decl.name, decl.desc))
                val original = decl.value as? String ?: continue
                if (bound(target)) replacement(original)?.let { literals += ClassNameLiteral(target, -1, original, it) }
            }
            for (decl in node.methods) {
                val target = SemanticTarget.Return(MethodSig(node.name, decl.name, decl.desc))
                if (decl.instructions.size() == 0) continue
                // Most methods have no relevant sink. Avoid running a data-flow analysis on them.
                if (!bound(target) && decl.instructions.none { insn ->
                    insn is MethodInsnNode && Type.getArgumentTypes(insn.desc).indices.any { parameter(insn, it) } ||
                        insn is FieldInsnNode && bound(field(insn)) ||
                        insn.opcode == AASTORE && contracts.any { it is SemanticTarget.Parameter && it.parameter.method == target.method }
                }) continue
                val uses = mutableListOf<Pair<SourceValue, Boolean>>()
                val arraySinks = mutableListOf<SourceValue>()
                val stores = mutableListOf<Pair<SourceValue, SourceValue>>()
                val arrayOrigins = mutableSetOf<AbstractInsnNode>()
                val parameterSlots = mutableMapOf<Int, Int>()
                var slot = if (decl.access and ACC_STATIC != 0) 0 else 1
                Type.getArgumentTypes(decl.desc).forEachIndexed { index, type -> parameterSlots[slot] = index; slot += type.size }
                fun consume(value: SourceValue, className: Boolean, descriptor: String? = null) {
                    if (className && descriptor == "[Ljava/lang/String;") arraySinks += value
                    else uses += value to className
                }
                val interpreter = object : SourceInterpreter(ASM9) {
                    override fun newParameterValue(isInstanceMethod: Boolean, local: Int, type: Type): SourceValue {
                        val marker = InsnNode(NOP)
                        val index = parameterSlots[local]
                        if (type.descriptor == "[Ljava/lang/String;" && index != null
                            && bound(SemanticTarget.Parameter(MethodParameterSig(target.method, index)))) arrayOrigins += marker
                        return SourceValue(type.size, marker)
                    }
                    // Keep literal origins through locals, stack copies and casts; a new call/field read is a boundary.
                    override fun copyOperation(insn: AbstractInsnNode, value: SourceValue) = value
                    override fun unaryOperation(insn: AbstractInsnNode, value: SourceValue): SourceValue? {
                        if (insn.opcode == CHECKCAST) return value
                        when (insn.opcode) {
                            PUTSTATIC -> consume(value, bound(field(insn as FieldInsnNode)), insn.desc)
                            ARETURN -> consume(value, bound(target), Type.getReturnType(decl.desc).descriptor)
                            else -> consume(value, false)
                        }
                        return super.unaryOperation(insn, value)
                    }
                    override fun binaryOperation(insn: AbstractInsnNode, left: SourceValue, right: SourceValue): SourceValue? {
                        consume(left, false)
                        if (insn.opcode == PUTFIELD) consume(right, bound(field(insn as FieldInsnNode)), insn.desc)
                        else consume(right, false)
                        return super.binaryOperation(insn, left, right)
                    }
                    override fun ternaryOperation(insn: AbstractInsnNode, array: SourceValue, index: SourceValue, value: SourceValue): SourceValue? {
                        if (insn.opcode == AASTORE) stores += array to value else consume(value, false)
                        return super.ternaryOperation(insn, array, index, value)
                    }
                    override fun naryOperation(insn: AbstractInsnNode, values: MutableList<out SourceValue>): SourceValue? {
                        if (insn is MethodInsnNode) {
                            val start = if (insn.opcode == INVOKESTATIC) 0 else 1
                            if (start == 1) consume(values[0], false)
                            Type.getArgumentTypes(insn.desc).forEachIndexed { index, type ->
                                consume(values[start + index], parameter(insn, index), type.descriptor)
                            }
                        } else values.forEach { consume(it, false) }
                        return super.naryOperation(insn, values)
                    }
                }
                try {
                    Analyzer(interpreter).analyze(node.name, decl)
                } catch (e: AnalyzerException) {
                    warnings += "Class-name analysis skipped ${target.method}: ${e.message}"
                    continue
                }
                arraySinks.flatMap { it.insns }.filterTo(arrayOrigins) { it is TypeInsnNode && it.opcode == ANEWARRAY && it.desc == "java/lang/String" }
                fun classArray(origin: AbstractInsnNode): Boolean = origin in arrayOrigins || when (origin) {
                    is FieldInsnNode -> origin.desc == "[Ljava/lang/String;" && bound(field(origin))
                    is MethodInsnNode -> Type.getReturnType(origin.desc).descriptor == "[Ljava/lang/String;" && bound(SemanticTarget.Return(method(origin)))
                    else -> false
                }
                for ((array, value) in stores) consume(value, array.insns.isNotEmpty() && array.insns.all(::classArray))
                val selected = uses.filter { it.second }.flatMap { it.first.insns }.toSet()
                val ordinary = uses.filterNot { it.second }.flatMap { it.first.insns }.toSet()
                for (insn in selected) {
                    val original = (insn as? LdcInsnNode)?.cst as? String ?: continue
                    val renamed = replacement(original) ?: continue
                    val location = requireNotNull(offsets[insn])
                    if (insn in ordinary) {
                        warnings += "Class-name literal kept at ${target.method}, offset $location: also used as ordinary text; separate its definitions."
                    } else literals += ClassNameLiteral(target, location, original, renamed)
                }
            }
        }
    }
    return ClassNameRemapping(literals, warnings.toList())
}
