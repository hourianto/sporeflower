package j2me.semantic

import j2me.common.mappedClassName
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.SemanticDomainKind
import j2me.model.SemanticMap
import j2me.model.SemanticTarget
import j2me.symbols.MemberResolver
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

private val scalarKinds = arrayOf(SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS, SemanticDomainKind.PACKED,
    SemanticDomainKind.NUMERIC, SemanticDomainKind.STRING)
private val numericKinds = arrayOf(SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS, SemanticDomainKind.PACKED, SemanticDomainKind.NUMERIC)
private val boxedIntegral = setOf("java/lang/Byte", "java/lang/Short", "java/lang/Character", "java/lang/Integer", "java/lang/Long")

private fun requireScalarType(semantic: SemanticMap, domain: String, type: Type, message: String) {
    requireDomainKind(semantic, domain, *scalarKinds)
    val string = semantic.domains.getValue(domain).kind == SemanticDomainKind.STRING
    require(if (string) type.descriptor in setOf("Ljava/lang/String;", "Ljava/lang/Object;")
        else type.isSemanticIntegral() || type.sort == Type.OBJECT && type.internalName in boxedIntegral + "java/lang/Object") { message }
    if (semantic.domains.getValue(domain).bitFields.isNotEmpty() && type.isSemanticIntegral()) {
        val width = when (type.sort) { Type.BYTE -> 8; Type.SHORT, Type.CHAR -> 16; Type.LONG -> 64; else -> 32 }
        require(semantic.domains.getValue(domain).bitFields.all { it.shift + it.bits <= width }) {
            "Packed fields exceed the $width-bit storage type: $domain"
        }
    }
}

fun validateSemanticMap(
    semantic: SemanticMap,
    canonical: CanonicalMap,
    symbolsByClass: Map<String, ClassSymbols>,
    classpathSymbolsByClass: Map<String, ClassSymbols> = emptyMap(),
) {
    val allSymbols = classpathSymbolsByClass + symbolsByClass
    val existingClasses = allSymbols.keys.mapTo(linkedSetOf()) { mappedClassName(it, canonical) }
    val normalizedDomains = semantic.domains.keys.groupBy(::semanticOwner)
    require(normalizedDomains.values.none { it.size > 1 }) {
        "Semantic domains have conflicting generated owners: ${normalizedDomains.filterValues { it.size > 1 }}"
    }
    for (domain in semantic.domains.values) {
        var occupied = 0L
        for (mask in domain.exclusiveMasks) {
            require(domain.kind == SemanticDomainKind.FLAGS && mask != 0L && (occupied and mask) == 0L) {
                "Exclusive masks must be nonzero, disjoint masks in a flag domain: ${domain.id}"
            }
            occupied = occupied or mask
        }
        val owner = semanticOwner(domain.id)
        if (domain.syntheticValues.isNotEmpty() || domain.syntheticStrings.isNotEmpty()) {
            require(owner !in existingClasses) {
                "Synthetic semantic domain '${domain.id}' collides with existing class $owner"
            }
        }
        if (domain.kind == SemanticDomainKind.SLOTS) {
            require(domain.syntheticValues.all { it.desc in setOf("B", "S", "C", "I") }) {
                "Slot domain '${domain.id}' must use integral index constants"
            }
        }
        require(domain.syntheticStrings.isEmpty() || domain.kind == SemanticDomainKind.STRING) { "String constants require a string domain" }
        require(domain.syntheticValues.isEmpty() || domain.kind in setOf(SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS, SemanticDomainKind.SLOTS)) {
            "Packed, numeric, and string domains cannot declare integral constants: ${domain.id}"
        }
        require(domain.syntheticStrings.map { it.name }.distinct().size == domain.syntheticStrings.size
            && domain.syntheticStrings.map { it.value }.distinct().size == domain.syntheticStrings.size) {
            "Duplicate string domain names or values: ${domain.id}"
        }
        require((domain.format != null) == (domain.kind == SemanticDomainKind.NUMERIC)) { "Numeric domains require format metadata: ${domain.id}" }
        domain.format?.let { format ->
            require(format.kind in setOf("rgb", "argb", "fixed") && format.fractionBits in 0..62
                && (format.kind == "fixed" || format.fractionBits == 0)) { "Invalid numeric format: $format" }
        }
        require(domain.bitFields.isEmpty() || domain.kind in setOf(SemanticDomainKind.PACKED, SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS)) {
            "@BitField requires a packed, value, or flag domain"
        }
        for (field in domain.bitFields) {
            requireDomainKind(semantic, field.domain, *numericKinds)
            require(field.shift in 0..63 && field.bits in 1..64 && field.shift + field.bits <= 64) { "Invalid packed bit range: $field" }
            require(field.selectorValue and field.selectorMask == field.selectorValue) { "Selector value exceeds selector mask: $field" }
        }
        for ((index, field) in domain.bitFields.withIndex()) {
            val mask = bitMask(field.bits) shl field.shift
            for (other in domain.bitFields.take(index)) {
                val overlap = mask and (bitMask(other.bits) shl other.shift) != 0L
                val exclusive = (field.selectorValue xor other.selectorValue) and field.selectorMask and other.selectorMask != 0L
                require(!overlap || exclusive) { "Overlapping packed fields need mutually exclusive selectors: ${domain.id}" }
            }
        }
    }

    val domainValues = semantic.domains.keys.associateWith { linkedMapOf<Long, String>() }.toMutableMap()
    val visited = mutableSetOf<String>()
    val active = mutableSetOf<String>()
    fun checkPackedCycles(id: String) {
        require(id !in active) { "Cyclic packed-domain definitions: $id" }
        if (!visited.add(id)) return
        active += id
        semantic.domains.getValue(id).bitFields.forEach { checkPackedCycles(it.domain) }
        active -= id
    }
    semantic.domains.keys.forEach(::checkPackedCycles)
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
                requireDomainKind(semantic, it, *scalarKinds)
            }
        }
    }

    val stringValues = semantic.domains.values.associate { it.id to it.syntheticStrings.map { value -> value.value }.toMutableSet() }
    for ((field, domain) in semantic.realValues) {
        requireDomainKind(semantic, domain, SemanticDomainKind.VALUE, SemanticDomainKind.FLAGS, SemanticDomainKind.SLOTS, SemanticDomainKind.STRING)
        val ownerSymbols = allSymbols[field.owner]
            ?: throw IllegalArgumentException("Semantic constant owner not found: ${field.owner}")
        require(field in ownerSymbols.fields) { "Semantic constant field not found: $field" }
        val access = ownerSymbols.fieldAccess[field] ?: 0
        require((access and Opcodes.ACC_STATIC) != 0 && (access and Opcodes.ACC_FINAL) != 0) {
            "Semantic constant field must be static final: $field"
        }
        val rawValue = ownerSymbols.fieldConstantValues[field]
            ?: throw IllegalArgumentException("Semantic constant field has no ConstantValue: $field")
        if (semantic.domains.getValue(domain).kind == SemanticDomainKind.STRING) {
            require(field.desc == "Ljava/lang/String;") { "String domain requires String constants: $field" }
            require(stringValues.getValue(domain).add(rawValue)) { "Duplicate string domain value: $domain" }
            continue
        }
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
        requireScalarType(semantic, domain, semanticTargetType(target, allSymbols),
            "Semantic scalar binding requires a compatible integral, boxed, or String value: ${target.description()}")
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
            require(dimension !in semantics.slotDomains && dimension !in semantics.records) {
                "Array dimension $dimension cannot combine @IndexDomain with @Slots or @Records: ${target.description()}"
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
        for ((dimension, layout) in semantics.records) {
            requireDomainKind(semantic, layout.domain, SemanticDomainKind.SLOTS)
            require(dimension in 0 until type.dimensions) {
                "Semantic record dimension $dimension is invalid for ${target.description()}"
            }
            require(layout.stride > 0 && layout.offset >= 0) { "@Records requires positive stride and nonnegative offset" }
            require(domainValues.getValue(layout.domain).keys.all { it >= 0 && if (layout.planes)
                (it + 1) * layout.stride + layout.offset <= Int.MAX_VALUE else it < layout.stride }) {
                "@Records slot offsets must be within stride ${layout.stride}: ${layout.domain}"
            }
            semantics.slotDomains[dimension]?.let { header ->
                require(domainValues.getValue(header).keys.all { it >= 0 && it < layout.offset }) {
                    "@Slots on a record dimension must describe header positions before offset ${layout.offset}"
                }
            }
            requireSlotValueType(semantic, layout.domain, type,
                "@SlotValue requires integral array elements: ${target.description()}")
        }
        semantics.elementDomain?.let { domain ->
            requireScalarType(semantic, domain, type.elementType, "Semantic array element binding requires compatible leaf values: ${target.description()}")
        }
    }

    for ((site, domain) in semantic.callDomains) {
        semanticTargetType(SemanticTarget.Return(site.method), allSymbols)
        val callee = allSymbols[site.method.owner]?.methodCalls?.get(site.method)?.get(site.offset)
        require(callee != null) { "@CallDomain offset ${site.offset} is not an invocation in ${site.method}" }
        requireScalarType(semantic, domain, Type.getReturnType(callee.desc), "@CallDomain requires an integral call result or compatible boxed/String result: $callee at offset ${site.offset}")
    }

    for ((target, conditions) in semantic.conditionalDomains) {
        val method = when (target) {
            is SemanticTarget.Return -> target.method
            is SemanticTarget.Parameter -> target.parameter.method
            is SemanticTarget.Field -> error("@DomainWhen cannot bind a field")
        }
        require(target !in semantic.scalarDomains && target !in semantic.arraySemantics
            && (target !is SemanticTarget.Return || method !in semantic.returnDomainSources)) {
            "Conditional domains cannot be combined with a fixed binding or @DomainFromParameter"
        }
        require(conditions.map { it.parameter }.distinct().size == 1 && conditions.filter { it.equals != null }.map { it.equals }.distinct().size == conditions.count { it.equals != null }) {
            "Conditional domains require one selector and unique case values"
        }
        require(conditions.count { it.notEquals != null } <= 1 && conditions.count { it.otherwise } <= 1) {
            "Conditional domains require at most one negative or default case"
        }
        conditions.singleOrNull { it.notEquals != null }?.let { negative ->
            require(conditions.none { it.otherwise } && conditions.all { it === negative || it.equals == negative.notEquals }) {
                "Overlapping conditional cases: notEquals may only accompany its complementary equals case"
            }
        }
        require(conditions.none { it.otherwise } || conditions.any { it.equals != null }) {
            "An otherwise case requires an explicit equals case"
        }
        val arguments = Type.getArgumentTypes(method.desc)
        for (condition in conditions) {
            require(condition.parameter in arguments.indices && arguments[condition.parameter].isSemanticIntegral()) {
                "@DomainWhen selector must be an integral parameter: $method"
            }
            val selectorRange = when (arguments[condition.parameter].sort) {
                Type.BYTE -> Byte.MIN_VALUE.toLong()..Byte.MAX_VALUE.toLong()
                Type.SHORT -> Short.MIN_VALUE.toLong()..Short.MAX_VALUE.toLong()
                Type.CHAR -> Char.MIN_VALUE.code.toLong()..Char.MAX_VALUE.code.toLong()
                Type.INT -> Int.MIN_VALUE.toLong()..Int.MAX_VALUE.toLong()
                else -> Long.MIN_VALUE..Long.MAX_VALUE
            }
            require(listOfNotNull(condition.equals, condition.notEquals).all { it in selectorRange }) { "@DomainWhen case value is outside the selector's range: $method" }
            requireScalarType(semantic, condition.domain, semanticTargetType(target, allSymbols), "@DomainWhen target has an incompatible type: $target")
        }
    }
    for ((target, source) in semantic.slotDomainSources) {
        val method = when (target) {
            is SemanticTarget.Return -> target.method
            is SemanticTarget.Parameter -> target.parameter.method
            is SemanticTarget.Field -> error("@DomainFromSlot cannot bind a field")
        }
        require(target !in semantic.scalarDomains && target !in semantic.arraySemantics && target !in semantic.containers
            && target !in semantic.conditionalDomains && (target !is SemanticTarget.Return || method !in semantic.returnDomainSources)) {
            "@DomainFromSlot cannot be combined with another binding on the same target"
        }
        val arguments = Type.getArgumentTypes(method.desc)
        require(source.parameter in arguments.indices && source.slot >= 0) { "Invalid @DomainFromSlot source: $target" }
        val array = arguments[source.parameter]
        require(array.sort == Type.ARRAY && array.elementType.isSemanticIntegral()
            && semanticTargetType(target, allSymbols).isSemanticIntegral()) {
            "@DomainFromSlot requires an integral target and an integral array parameter: $target"
        }
        require(source.dimension != null || array.dimensions == 1) { "@DomainFromSlot requires dimension for a multidimensional array: $target" }
        require((source.dimension ?: 0) == array.dimensions - 1) { "@DomainFromSlot dimension must select the innermost array: $target" }
    }
    for ((target, container) in semantic.containers) {
        val type = semanticTargetType(target, allSymbols)
        require(type.sort == Type.OBJECT && type.internalName in setOf("java/util/Vector", "java/util/Hashtable", "java/util/Enumeration")) {
            "Container domains require Vector, Hashtable, or Enumeration: $target"
        }
        require(if (type.internalName == "java/util/Hashtable") container.elements == null else container.keys == null && container.values == null) {
            "Use @Keys/@Values for Hashtable and @Elements for Vector/Enumeration"
        }
        require(target !in semantic.scalarDomains && target !in semantic.conditionalDomains) { "Container and scalar bindings cannot be combined" }
        listOfNotNull(container.elements, container.keys, container.values).forEach { requireDomainKind(semantic, it, *scalarKinds) }
    }

    for (domain in semantic.domains.values) for (field in domain.bitFields) {
        val values = domainValues.getValue(field.domain).keys
        require(values.all { value -> if (field.bits == 64) true else if (field.signed)
            value >= -(1L shl (field.bits - 1)) && value < (1L shl (field.bits - 1))
            else value >= 0 && value ushr field.bits == 0L }) { "Bit-field domain values do not fit the declared range: $field" }
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

private fun requireSlotValueType(semantic: SemanticMap, domain: String, type: Type, message: String) {
    semantic.domains.getValue(domain).syntheticValues.mapNotNull { it.elementDomain }.forEach {
        requireScalarType(semantic, it, type.elementType, message)
    }
}

private fun bitMask(bits: Int): Long = if (bits == 64) -1 else (1L shl bits) - 1

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
    val members = MemberResolver(allSymbols)
    fun mappedFieldMember(field: FieldSig) = MappedMember(
        owner = mappedClassName(field.owner, canonical),
        name = canonical.fields[field] ?: field.name,
        desc = descMapper.mapDesc(field.desc),
    )
    fun mappedMethodMember(method: MethodSig) = MappedMember(
        owner = mappedClassName(method.owner, canonical),
        // Call-site references can name a subclass that inherits the method.
        // Match the name emitted by the engine's constant-pool resolver while
        // retaining the reference owner, as JVM invocation descriptors do.
        name = canonical.methods[method] ?: canonical.methods[members.method(method)] ?: method.name,
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
    val strings = mutableListOf<StringValueEntry>()
    for (domain in semantic.domains.values.sortedBy { it.id }) {
        val owner = semanticOwner(domain.id)
        domain.syntheticStrings.forEach { strings += StringValueEntry(owner, it.value, owner, it.name,
            Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL, true) }
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
        if (field.desc == "Ljava/lang/String;") {
            strings += StringValueEntry(semanticOwner(domain), requireNotNull(symbols.fieldConstantValues[field]),
                mappedClassName(field.owner, canonical), canonical.fields[field] ?: field.name, symbols.fieldAccess[field] ?: 0, false)
            continue
        }
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
            DomainEntry(semanticOwner(it.id), it.kind.name.lowercase(), it.exclusiveMasks,
                it.bitFields.map { field -> BitFieldEntry(semanticOwner(field.domain), field.shift, field.bits, field.signed, field.selectorMask, field.selectorValue) },
                it.format?.let { format -> NumberFormatEntry(format.kind, format.fractionBits) })
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
                semantics.records.entries.sortedBy { it.key }.map { (dimension, layout) ->
                    RecordLayoutEntry(dimension, semanticOwner(layout.domain), layout.stride, layout.offset, layout.planes)
                },
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
        semantic.callDomains.entries
            .sortedWith(compareBy({ it.key.method.owner }, { it.key.method.name }, { it.key.method.desc }, { it.key.offset }))
            .map { (site, domain) ->
                val callee = allSymbols.getValue(site.method.owner).methodCalls.getValue(site.method).getValue(site.offset)
                CallBindingEntry(mappedTarget(SemanticTarget.Return(site.method)), site.offset,
                    mappedTarget(SemanticTarget.Return(callee)), semanticOwner(domain))
            },
        strings,
        semantic.conditionalDomains.entries.sortedBy { it.key.sortKey() }.flatMap { (target, conditions) -> conditions.map {
            ConditionalBindingEntry(mappedTarget(target), it.parameter, it.equals, semanticOwner(it.domain), it.notEquals, it.otherwise)
        } },
        semantic.containers.entries.sortedBy { it.key.sortKey() }.map { (target, container) ->
            ContainerBindingEntry(mappedTarget(target), container.elements?.let(::semanticOwner),
                container.keys?.let(::semanticOwner), container.values?.let(::semanticOwner))
        },
        semantic.slotDomainSources.entries.sortedBy { it.key.sortKey() }.map { (target, source) ->
            SlotDomainSourceEntry(mappedTarget(target), source.parameter, source.slot, source.dimension ?: 0)
        },
    )
}
