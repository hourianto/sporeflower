package j2me.model

import org.objectweb.asm.Opcodes
import java.nio.file.Path

data class FieldSig(
    val owner: String,
    val name: String,
    val desc: String,
)

data class MethodSig(
    val owner: String,
    val name: String,
    val desc: String,
)

data class MappingOrigin(
    val path: Path,
    val line: Int,
    val source: String? = null,
)

fun MethodSig.isConstructor(): Boolean = name == "<init>"

data class ClassSymbols(
    val fields: List<FieldSig>,
    val methods: List<MethodSig>,
    val methodAccess: Map<MethodSig, Int> = emptyMap(),
    val fieldAccess: Map<FieldSig, Int> = emptyMap(),
    val fieldConstantValues: Map<FieldSig, String> = emptyMap(),
    val superName: String? = null,
    val interfaces: List<String> = emptyList(),
) {
    fun isMethodStatic(method: MethodSig): Boolean =
        ((methodAccess[method] ?: 0) and Opcodes.ACC_STATIC) != 0

    fun isGeneratedMethod(method: MethodSig): Boolean =
        ((methodAccess[method] ?: 0) and (Opcodes.ACC_BRIDGE or Opcodes.ACC_SYNTHETIC)) != 0

    fun isGeneratedField(field: FieldSig): Boolean =
        ((fieldAccess[field] ?: 0) and Opcodes.ACC_SYNTHETIC) != 0
}

data class CanonicalMap(
    val classes: Map<String, String> = emptyMap(),
    val fields: Map<FieldSig, String> = emptyMap(),
    val methods: Map<MethodSig, String> = emptyMap(),
    val methodArgs: Map<MethodSig, List<String>> = emptyMap(),
    val fieldOrigins: Map<FieldSig, MappingOrigin> = emptyMap(),
    val methodOrigins: Map<MethodSig, MappingOrigin> = emptyMap(),
    val ignoredClasses: Set<String> = emptySet(),
)

enum class SemanticDomainKind {
    VALUE,
    FLAGS,
    SLOTS,
}

data class SyntheticSemanticValue(
    val name: String,
    val desc: String,
    val value: Long,
    val elementDomain: String? = null,
)

data class SemanticDomain(
    val id: String,
    val kind: SemanticDomainKind,
    val syntheticValues: List<SyntheticSemanticValue> = emptyList(),
)

data class MethodParameterSig(
    val method: MethodSig,
    val index: Int,
)

sealed interface SemanticTarget {
    data class Field(val field: FieldSig) : SemanticTarget
    data class Return(val method: MethodSig) : SemanticTarget
    data class Parameter(val parameter: MethodParameterSig) : SemanticTarget
}

data class SemanticArraySemantics(
    val indexDomains: Map<Int, String> = emptyMap(),
    val slotDomains: Map<Int, String> = emptyMap(),
    val elementDomain: String? = null,
) {
    fun isEmpty(): Boolean = indexDomains.isEmpty() && slotDomains.isEmpty() && elementDomain == null
}

data class SemanticMap(
    val domains: Map<String, SemanticDomain> = emptyMap(),
    val realValues: Map<FieldSig, String> = emptyMap(),
    val scalarDomains: Map<SemanticTarget, String> = emptyMap(),
    val arraySemantics: Map<SemanticTarget, SemanticArraySemantics> = emptyMap(),
    val returnDomainSources: Map<MethodSig, Int> = emptyMap(),
)

data class ProjectMappings(
    val canonical: CanonicalMap,
    val semantic: SemanticMap = SemanticMap(),
)
