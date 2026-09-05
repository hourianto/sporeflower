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
    val methodCalls: Map<MethodSig, Map<Int, MethodSig>> = emptyMap(),
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
    PACKED,
    NUMERIC,
    STRING,
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
    val exclusiveMasks: List<Long> = emptyList(),
    val bitFields: List<SemanticBitField> = emptyList(),
    val format: SemanticNumberFormat? = null,
    val syntheticStrings: List<SemanticStringValue> = emptyList(),
)

data class SemanticBitField(val domain: String, val shift: Int, val bits: Int, val signed: Boolean = false,
                           val selectorMask: Long = 0, val selectorValue: Long = 0)
data class SemanticNumberFormat(val kind: String, val fractionBits: Int = 0)
data class SemanticStringValue(val name: String, val value: String)
data class SemanticCondition(val parameter: Int, val equals: Long?, val domain: String,
                             val notEquals: Long? = null, val otherwise: Boolean = false)
data class SemanticSlotSource(val parameter: Int, val slot: Int, val dimension: Int? = null)
data class SemanticContainer(val elements: String? = null, val keys: String? = null, val values: String? = null)

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
    val records: Map<Int, SemanticRecordLayout> = emptyMap(),
) {
    fun isEmpty(): Boolean = indexDomains.isEmpty() && slotDomains.isEmpty() && elementDomain == null && records.isEmpty()
}

data class SemanticRecordLayout(val domain: String, val stride: Int, val offset: Int = 0, val planes: Boolean = false)
data class SemanticCallSite(val method: MethodSig, val offset: Int)

data class SemanticMap(
    val domains: Map<String, SemanticDomain> = emptyMap(),
    val realValues: Map<FieldSig, String> = emptyMap(),
    val scalarDomains: Map<SemanticTarget, String> = emptyMap(),
    val arraySemantics: Map<SemanticTarget, SemanticArraySemantics> = emptyMap(),
    val returnDomainSources: Map<MethodSig, Int> = emptyMap(),
    val callDomains: Map<SemanticCallSite, String> = emptyMap(),
    val conditionalDomains: Map<SemanticTarget, List<SemanticCondition>> = emptyMap(),
    val containers: Map<SemanticTarget, SemanticContainer> = emptyMap(),
    val slotDomainSources: Map<SemanticTarget, SemanticSlotSource> = emptyMap(),
)

data class ProjectMappings(
    val canonical: CanonicalMap,
    val semantic: SemanticMap = SemanticMap(),
)
