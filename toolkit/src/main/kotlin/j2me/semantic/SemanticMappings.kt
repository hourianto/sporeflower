package j2me.semantic

import j2me.common.mappedClassName
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.SemanticDomainKind
import j2me.model.SemanticMap
import j2me.model.SemanticTarget
import org.jetbrains.java.decompiler.api.SemanticMappingData
import org.jetbrains.java.decompiler.api.SemanticMappingData.*
import org.objectweb.asm.Opcodes
import org.objectweb.asm.Type
import org.objectweb.asm.commons.Remapper

private fun semanticOwner(id: String): String =
    if ('.' in id) id.replace('.', '/') else "defpackage/$id"

private fun descriptorMapper(cmap: CanonicalMap): Remapper = object : Remapper(Opcodes.ASM9) {
    override fun map(internalName: String?): String? =
        internalName?.let { mappedClassName(it, cmap) }
}

private fun requireDomainKind(semantic: SemanticMap, domain: String, vararg allowed: SemanticDomainKind) {
    val actual = semantic.domains[domain]?.kind ?: error("Unknown semantic domain: $domain")
    require(actual in allowed) {
        "Semantic domain '$domain' has kind $actual, expected ${allowed.joinToString()}"
    }
}

private fun constantLong(desc: String, raw: String): Long = when (desc) {
    "B", "S", "C", "I", "J" -> raw.toLong()
    else -> throw IllegalArgumentException("Unsupported semantic constant descriptor: $desc")
}

private fun Type.isSemanticIntegral(): Boolean = when (sort) {
    Type.BYTE, Type.CHAR, Type.SHORT, Type.INT, Type.LONG -> true
    else -> false
}

fun validateSemanticMap(
    semantic: SemanticMap,
    canonical: CanonicalMap,
    symbolsByClass: Map<String, ClassSymbols>,
    classpathSymbolsByClass: Map<String, ClassSymbols> = emptyMap(),
) {
    val allSymbols = classpathSymbolsByClass + symbolsByClass
    val emittedClasses = symbolsByClass.keys.mapTo(linkedSetOf()) { mappedClassName(it, canonical) }
    for (domain in semantic.domains.values) {
        val owner = semanticOwner(domain.id)
        if (domain.syntheticValues.isNotEmpty()) {
            require(owner !in emittedClasses) {
                "Synthetic semantic domain '${domain.id}' collides with generated class $owner"
            }
        }
        if (domain.kind == SemanticDomainKind.SLOTS) {
            require(domain.syntheticValues.all { it.desc in setOf("B", "S", "C", "I") }) {
                "Slot domain '${domain.id}' must use integral index constants"
            }
        }
    }

    val domainValues = semantic.domains.keys.associateWith { linkedMapOf<Long, String>() }.toMutableMap()
    for (domain in semantic.domains.values) {
        for (value in domain.syntheticValues) {
            val previous = domainValues.getValue(domain.id).putIfAbsent(value.value, value.name)
            require(previous == null) {
                "Semantic domain '${domain.id}' maps ${value.value} to both '$previous' and '${value.name}'"
            }
            value.elementDomain?.let {
                require(domain.kind == SemanticDomainKind.SLOTS) {
                    "@SlotValue is only valid in a slot domain: ${domain.id}.${value.name}"
                }
                requireDomainKind(semantic, it, SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS)
            }
        }
    }

    for ((field, domain) in semantic.realValues) {
        requireDomainKind(semantic, domain, SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS, SemanticDomainKind.SLOTS)
        val ownerSymbols = allSymbols[field.owner]
            ?: throw IllegalArgumentException("Semantic constant owner not found: ${field.owner}")
        require(field in ownerSymbols.fields) { "Semantic constant field not found: $field" }
        val access = ownerSymbols.fieldAccess[field] ?: 0
        require((access and Opcodes.ACC_STATIC) != 0 && (access and Opcodes.ACC_FINAL) != 0) {
            "Semantic constant field must be static final: $field"
        }
        val rawValue = ownerSymbols.fieldConstantValues[field]
            ?: throw IllegalArgumentException("Semantic constant field has no ConstantValue: $field")
        val value = constantLong(field.desc, rawValue)
        if (semantic.domains.getValue(domain).kind == SemanticDomainKind.SLOTS) {
            require(field.desc in setOf("B", "S", "C", "I")) {
                "Slot domain '$domain' must use integral index constants: $field"
            }
        }
        val name = canonical.fields[field] ?: field.name
        val previous = domainValues.getValue(domain).putIfAbsent(value, name)
        require(previous == null) {
            "Semantic domain '$domain' maps $value to both '$previous' and '$name'"
        }
    }

    for ((target, domain) in semantic.scalarDomains) {
        requireDomainKind(semantic, domain, SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS)
        require(semanticTargetType(target, allSymbols).isSemanticIntegral()) {
            "Semantic scalar binding requires an integral value: ${target.description()}"
        }
    }

    for ((target, semantics) in semantic.arraySemantics) {
        require(!semantics.isEmpty()) { "Empty semantic array binding: ${target.description()}" }
        val type = semanticTargetType(target, allSymbols)
        require(type.sort == Type.ARRAY) { "Semantic array binding requires an array: ${target.description()}" }

        for ((dimension, domain) in semantics.indexDomains) {
            requireDomainKind(semantic, domain, SemanticDomainKind.VALUE)
            require(dimension in 0 until type.dimensions) {
                "Semantic array dimension $dimension is invalid for ${target.description()}"
            }
            require(dimension !in semantics.slotDomains) {
                "Array dimension $dimension cannot have both @IndexDomain and @Slots: ${target.description()}"
            }
        }
        for ((dimension, domain) in semantics.slotDomains) {
            requireDomainKind(semantic, domain, SemanticDomainKind.SLOTS)
            require(dimension in 0 until type.dimensions) {
                "Semantic slot dimension $dimension is invalid for ${target.description()}"
            }
            requireSlotValueType(
                semantic,
                domain,
                type,
                "@SlotValue requires integral array elements: ${target.description()}",
            )
        }
        semantics.elementDomain?.let { domain ->
            requireDomainKind(semantic, domain, SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS)
            requireSemanticArrayElements(
                type,
                "Semantic array element binding requires integral leaf values: ${target.description()}",
            )
        }
    }

    for ((method, sourceParameter) in semantic.returnDomainSources) {
        val descriptor = Type.getMethodType(method.desc)
        require(semanticTargetType(SemanticTarget.Return(method), allSymbols).isSemanticIntegral()) {
            "@DomainFromParameter requires an integral return value: $method"
        }
        require(sourceParameter in descriptor.argumentTypes.indices) {
            "@DomainFromParameter index $sourceParameter is invalid for $method"
        }
        require(descriptor.argumentTypes[sourceParameter].isSemanticIntegral()) {
            "@DomainFromParameter requires an integral source parameter: $method parameter $sourceParameter"
        }
        require(SemanticTarget.Return(method) !in semantic.scalarDomains) {
            "@DomainFromParameter cannot be combined with a fixed return domain: $method"
        }
    }
}

private fun semanticTargetType(target: SemanticTarget, symbolsByClass: Map<String, ClassSymbols>): Type = when (target) {
    is SemanticTarget.Field -> {
        val field = target.field
        require(field in symbolsByClass[field.owner].orEmptyFields()) { "Semantic field binding not found: $field" }
        Type.getType(field.desc)
    }
    is SemanticTarget.Return -> {
        val method = target.method
        require(method in symbolsByClass[method.owner].orEmptyMethods()) { "Semantic method binding not found: $method" }
        Type.getReturnType(method.desc)
    }
    is SemanticTarget.Parameter -> {
        val parameter = target.parameter
        val method = parameter.method
        require(method in symbolsByClass[method.owner].orEmptyMethods()) { "Semantic method binding not found: $method" }
        val arguments = Type.getArgumentTypes(method.desc)
        require(parameter.index in arguments.indices) { "Semantic parameter index ${parameter.index} is invalid for $method" }
        arguments[parameter.index]
    }
}

private fun SemanticTarget.description(): String = when (this) {
    is SemanticTarget.Field -> "field $field"
    is SemanticTarget.Return -> "return of $method"
    is SemanticTarget.Parameter -> "parameter ${parameter.index} of ${parameter.method}"
}

private fun requireSemanticArrayElements(type: Type, message: String) {
    require(type.sort == Type.ARRAY && type.elementType.isSemanticIntegral()) { message }
}

private fun requireSlotValueType(semantic: SemanticMap, domain: String, type: Type, message: String) {
    if (semantic.domains.getValue(domain).syntheticValues.any { it.elementDomain != null }) {
        requireSemanticArrayElements(type, message)
    }
}

private fun ClassSymbols?.orEmptyFields(): List<FieldSig> = this?.fields.orEmpty()
private fun ClassSymbols?.orEmptyMethods(): List<MethodSig> = this?.methods.orEmpty()

private data class MappedMember(val owner: String, val name: String, val desc: String)

private fun SemanticTarget.sortKey(): String = when (this) {
    is SemanticTarget.Field -> "field:${field.owner}:${field.name}:${field.desc}"
    is SemanticTarget.Return -> "return:${method.owner}:${method.name}:${method.desc}"
    is SemanticTarget.Parameter ->
        "parameter:${parameter.method.owner}:${parameter.method.name}:${parameter.method.desc}:${parameter.index}"
}

fun buildSemanticMappings(
    semantic: SemanticMap,
    canonical: CanonicalMap,
    symbolsByClass: Map<String, ClassSymbols>,
    classpathSymbolsByClass: Map<String, ClassSymbols> = emptyMap(),
): SemanticMappingData {
    // The pipeline validates once before it creates any output. Keeping emission
    // pure avoids a second full walk and makes this function usable with a model
    // that has already been checked at a higher-level boundary.
    val allSymbols = classpathSymbolsByClass + symbolsByClass
    val descMapper = descriptorMapper(canonical)
    fun mappedFieldMember(field: FieldSig) = MappedMember(
        owner = mappedClassName(field.owner, canonical),
        name = canonical.fields[field] ?: field.name,
        desc = descMapper.mapDesc(field.desc),
    )
    fun mappedMethodMember(method: MethodSig) = MappedMember(
        owner = mappedClassName(method.owner, canonical),
        name = canonical.methods[method] ?: method.name,
        desc = descMapper.mapMethodDesc(method.desc),
    )
    fun mappedTarget(target: SemanticTarget): TargetEntry {
        val (kind, member, index) = when (target) {
            is SemanticTarget.Field -> Triple("field", mappedFieldMember(target.field), null)
            is SemanticTarget.Return -> Triple("return", mappedMethodMember(target.method), null)
            is SemanticTarget.Parameter -> Triple(
                "parameter",
                mappedMethodMember(target.parameter.method),
                target.parameter.index,
            )
        }
        return TargetEntry(kind, member.owner, member.name, member.desc, index)
    }
    fun dimensionDomains(domains: Map<Int, String>) = domains.entries
        .sortedBy { it.key }
        .map { (dimension, domain) -> DimensionEntry(dimension, semanticOwner(domain)) }

    val values = mutableListOf<ValueEntry>()
    for (domain in semantic.domains.values.sortedBy { it.id }) {
        val owner = semanticOwner(domain.id)
        domain.syntheticValues.sortedWith(compareBy({ it.value }, { it.name })).forEach { value ->
            values += ValueEntry(
                owner,
                value.value,
                owner,
                value.name,
                value.desc,
                Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL,
                true,
                value.elementDomain?.let(::semanticOwner),
            )
        }
    }
    for ((field, domain) in semantic.realValues.entries.sortedWith(compareBy({ it.key.owner }, { it.key.name }, { it.key.desc }))) {
        val symbols = requireNotNull(allSymbols[field.owner])
        values += ValueEntry(
            semanticOwner(domain),
            constantLong(field.desc, requireNotNull(symbols.fieldConstantValues[field])),
            mappedClassName(field.owner, canonical),
            canonical.fields[field] ?: field.name,
            descMapper.mapDesc(field.desc),
            symbols.fieldAccess[field] ?: 0,
            false,
            null,
        )
    }

    return SemanticMappingData(
        semantic.domains.values.sortedBy { it.id }.map {
            DomainEntry(semanticOwner(it.id), it.kind.name.lowercase())
        },
        values,
        semantic.scalarDomains.entries.sortedBy { it.key.sortKey() }.map { (target, domain) ->
            ScalarBindingEntry(mappedTarget(target), semanticOwner(domain))
        },
        semantic.arraySemantics.entries.sortedBy { it.key.sortKey() }.map { (target, semantics) ->
            ArrayBindingEntry(
                mappedTarget(target),
                dimensionDomains(semantics.indexDomains),
                dimensionDomains(semantics.slotDomains),
                semantics.elementDomain?.let(::semanticOwner),
            )
        },
        semantic.returnDomainSources.entries
            .sortedWith(compareBy({ it.key.owner }, { it.key.name }, { it.key.desc }))
            .map { (method, sourceParameter) ->
                ReturnDomainSourceEntry(
                    mappedTarget(SemanticTarget.Return(method)),
                    sourceParameter,
                )
            },
    )
}
