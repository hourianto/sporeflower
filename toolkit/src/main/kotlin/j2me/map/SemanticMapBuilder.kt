package j2me.map

import com.github.javaparser.ast.body.Parameter
import com.github.javaparser.ast.expr.AnnotationExpr
import j2me.common.JavaSourceContext
import j2me.model.FieldSig
import j2me.model.MethodParameterSig
import j2me.model.MethodSig
import j2me.model.SemanticArraySemantics
import j2me.model.SemanticDomain
import j2me.model.SemanticDomainKind
import j2me.model.SemanticMap
import j2me.model.SemanticTarget
import j2me.model.SemanticRecordLayout
import j2me.model.SemanticCallSite
import j2me.model.SemanticSlotSource
import j2me.model.SemanticCondition
import j2me.model.SemanticContainer
import org.objectweb.asm.Type

internal enum class MapAuthority {
    BUILTIN,
    PROJECT,
}

internal class SemanticMapBuilder(
    private val domains: MutableMap<String, SemanticDomain>,
) {
    private data class MutableArraySemantics(
        val indexDomains: MutableMap<Int, String> = linkedMapOf(),
        val slotDomains: MutableMap<Int, String> = linkedMapOf(),
        var elementDomain: String? = null,
        val records: MutableMap<Int, SemanticRecordLayout> = linkedMapOf(),
    )

    private val realValues = linkedMapOf<FieldSig, String>()
    private val builtinRealValues = linkedSetOf<FieldSig>()
    private val scalarDomains = linkedMapOf<SemanticTarget, String>()
    private val builtinScalarDomains = linkedSetOf<SemanticTarget>()
    private val arraySemantics = linkedMapOf<SemanticTarget, MutableArraySemantics>()
    private val builtinElementDomains = linkedSetOf<SemanticTarget>()
    private val builtinIndexDomains = linkedSetOf<Pair<SemanticTarget, Int>>()
    private val builtinSlotDomains = linkedSetOf<Pair<SemanticTarget, Int>>()
    private val returnDomainSources = linkedMapOf<MethodSig, Int>()
    private val builtinReturnDomainSources = linkedSetOf<MethodSig>()
    private val callDomains = linkedMapOf<SemanticCallSite, String>()
    private val slotDomainSources = linkedMapOf<SemanticTarget, SemanticSlotSource>()
    private val conditionalDomains = linkedMapOf<SemanticTarget, MutableList<SemanticCondition>>()
    private val containers = linkedMapOf<SemanticTarget, SemanticContainer>()

    fun bindRealValue(field: FieldSig, domain: String, authority: MapAuthority) {
        bindAuthoritatively(field, authority, builtinRealValues) {
            require(realValues.putIfAbsent(field, domain) == null) { "duplicate semantic value binding for $field" }
        }
    }

    private fun bindScalar(target: SemanticTarget, domain: String, authority: MapAuthority) {
        bindAuthoritatively(target, authority, builtinScalarDomains) {
            require(scalarDomains.putIfAbsent(target, domain) == null) { "duplicate semantic binding for $target" }
        }
    }

    private fun bindReturnDomainSource(method: MethodSig, sourceParameter: Int, authority: MapAuthority) {
        bindAuthoritatively(method, authority, builtinReturnDomainSources) {
            require(returnDomainSources.putIfAbsent(method, sourceParameter) == null) {
                "duplicate @DomainFromParameter binding for $method"
            }
        }
    }

    private fun bindElement(target: SemanticTarget, domain: String, authority: MapAuthority) {
        bindAuthoritatively(target, authority, builtinElementDomains) {
            val semantics = arraySemantics.getOrPut(target, ::MutableArraySemantics)
            require(semantics.elementDomain == null) { "duplicate semantic element binding for $target" }
            semantics.elementDomain = domain
        }
    }

    private fun bindIndex(target: SemanticTarget, dimension: Int, domain: String, authority: MapAuthority) {
        val key = target to dimension
        bindAuthoritatively(key, authority, builtinIndexDomains) {
            val semantics = arraySemantics.getOrPut(target, ::MutableArraySemantics)
            require(semantics.indexDomains.putIfAbsent(dimension, domain) == null) {
                "duplicate @IndexDomain for dimension $dimension"
            }
        }
    }

    private fun bindSlots(target: SemanticTarget, dimension: Int, domain: String, authority: MapAuthority) {
        val key = target to dimension
        bindAuthoritatively(key, authority, builtinSlotDomains) {
            val semantics = arraySemantics.getOrPut(target, ::MutableArraySemantics)
            require(semantics.slotDomains.putIfAbsent(dimension, domain) == null) {
                "duplicate @Slots for dimension $dimension"
            }
        }
    }

    private inline fun <T> bindAuthoritatively(
        key: T,
        authority: MapAuthority,
        builtinKeys: MutableSet<T>,
        bind: () -> Unit,
    ) {
        if (authority == MapAuthority.PROJECT && key in builtinKeys) return
        bind()
        if (authority == MapAuthority.BUILTIN) builtinKeys += key
    }

    fun resolveDomain(rawName: String, context: JavaSourceContext): String {
        val raw = rawName.removeSuffix(".class")
        context.imports.explicit[raw]?.let { imported ->
            require(imported in domains) { "unknown semantic domain: $imported" }
            return imported
        }
        if ('.' in raw) {
            require(raw in domains) { "unknown semantic domain: $raw" }
            return raw
        }
        if (context.packageName.isNotBlank()) {
            val samePackage = "${context.packageName}.$raw"
            if (samePackage in domains) return samePackage
        }
        val imported = context.imports.wildcardPackages.map { "$it.$raw" }.filter { it in domains }.distinct()
        require(imported.size <= 1) { "ambiguous semantic domain '$raw': ${imported.joinToString()}" }
        imported.singleOrNull()?.let { return it }
        if (raw in domains) return raw
        val matches = domains.keys.filter { it.substringAfterLast('.') == raw }
        require(matches.size == 1) {
            if (matches.isEmpty()) "unknown semantic domain: $raw" else "ambiguous semantic domain '$raw': ${matches.joinToString()}"
        }
        return matches.single()
    }

    fun build(): SemanticMap = SemanticMap(
        domains = domains,
        realValues = realValues,
        scalarDomains = scalarDomains,
        arraySemantics = arraySemantics.mapValues { (_, semantics) ->
            SemanticArraySemantics(
                indexDomains = semantics.indexDomains,
                slotDomains = semantics.slotDomains,
                elementDomain = semantics.elementDomain,
                records = semantics.records,
            )
        },
        returnDomainSources = returnDomainSources,
        callDomains = callDomains,
        conditionalDomains = conditionalDomains,
        containers = containers,
        slotDomainSources = slotDomainSources,
    )

    fun bindDeclarationSemantics(
        target: SemanticTarget,
        type: Type,
        annotations: Iterable<AnnotationExpr>,
        context: JavaSourceContext,
        authority: MapAuthority,
    ) {
        consumerDomainAnnotation(annotations)?.let { annotation ->
            val domain = resolveConsumerDomain(annotation, context)
            if (type.sort == Type.ARRAY) bindElement(target, domain, authority) else bindScalar(target, domain, authority)
        }
        // ASM's getDimensions() assumes an array type and reads past the
        // descriptor buffer for some primitives, notably double.
        val dimensions = if (type.sort == Type.ARRAY) type.dimensions else 0
        parseSlotDomains(annotations, dimensions, context).forEach { (dimension, domain) ->
            bindSlots(target, dimension, domain, authority)
        }
        parseIndexDomains(annotations, context).forEach { (dimension, domain) ->
            bindIndex(target, dimension, domain, authority)
        }
        annotations.filter { it.nameAsString.substringAfterLast('.') in setOf("Records", "Planes") }.forEach { annotation ->
            require(dimensions > 0) { "@Records/@Planes requires an array declaration" }
            val dimension = annotationInteger(annotation, "dimension", if (dimensions == 1) 0 else null)
            val stride = annotationInteger(annotation, "stride")
            val offset = annotationInteger(annotation, "offset", 0)
            require(stride > 0) { "@Records stride must be positive" }
            val domain = resolveDomain(annotationClassName(annotation), context)
            requireDomainKind(domain, SemanticDomainKind.SLOTS, "Records")
            val layout = SemanticRecordLayout(domain, stride, offset, annotation.nameAsString.substringAfterLast('.') == "Planes")
            require(arraySemantics.getOrPut(target, ::MutableArraySemantics).records.putIfAbsent(dimension, layout) == null) {
                "duplicate @Records for dimension $dimension"
            }
        }
        annotationsNamed(annotations, "DomainFromSlot").forEach { annotation ->
            require(target !is SemanticTarget.Field) { "@DomainFromSlot requires a method parameter or return" }
            require(slotDomainSources.putIfAbsent(target, SemanticSlotSource(
                annotationInteger(annotation, "parameter"), annotationInteger(annotation, "slot"),
                annotationValue(annotation, "dimension")?.let { annotationInteger(annotation, "dimension") },
            )) == null) { "duplicate @DomainFromSlot binding for $target" }
        }
        annotationsNamed(annotations, "DomainWhen").forEach { annotation ->
            require(target !is SemanticTarget.Field) { "@DomainWhen requires a method parameter or return" }
            val equal = annotationValue(annotation, "equals")?.let(::parseIntegralConstant)
            val notEqual = annotationValue(annotation, "notEquals")?.let(::parseIntegralConstant)
            val otherwise = annotationValue(annotation, "otherwise")?.let {
                require(it.isBooleanLiteralExpr && it.asBooleanLiteralExpr().value) { "@DomainWhen otherwise must be true" }
                true
            } ?: false
            require(listOf(equal != null, notEqual != null, otherwise).count { it } == 1) {
                "@DomainWhen requires exactly one of equals, notEquals, or otherwise = true"
            }
            conditionalDomains.getOrPut(target) { mutableListOf() } += SemanticCondition(
                annotationInteger(annotation, "parameter"), equal,
                resolveDomain(annotationClassName(annotation), context), notEqual, otherwise,
            )
        }
        for (name in listOf("Elements", "Keys", "Values")) {
            val matches = annotationsNamed(annotations, name)
            require(matches.size <= 1) { "duplicate @$name binding" }
            matches.singleOrNull()?.let { annotation ->
                val domain = resolveDomain(annotationClassName(annotation), context)
                val container = containers[target] ?: SemanticContainer()
                require(when (name) { "Elements" -> container.elements; "Keys" -> container.keys; else -> container.values } == null) {
                    "duplicate @$name binding for $target"
                }
                containers[target] = when (name) {
                    "Elements" -> container.copy(elements = domain)
                    "Keys" -> container.copy(keys = domain)
                    else -> container.copy(values = domain)
                }
            }
        }
    }

    fun bindCallableSemantics(
        method: MethodSig,
        returnAnnotations: Iterable<AnnotationExpr>?,
        parameters: List<Parameter>,
        context: JavaSourceContext,
        authority: MapAuthority,
    ) {
        returnAnnotations?.let {
            annotationsNamed(it, "CallDomain").forEach { annotation ->
                val offset = annotationInteger(annotation, "offset")
                val domain = resolveDomain(annotationClassName(annotation), context)
                require(domains.getValue(domain).kind != SemanticDomainKind.SLOTS) {
                    "@CallDomain requires a scalar domain"
                }
                require(callDomains.putIfAbsent(SemanticCallSite(method, offset), domain) == null) {
                    "duplicate @CallDomain at bytecode offset $offset in $method"
                }
            }
            parseReturnDomainSource(it)?.let { sourceParameter ->
                require(consumerDomainAnnotation(it) == null) {
                    "@DomainFromParameter cannot be combined with @Domain or @Flags"
                }
                require(sourceParameter in parameters.indices) {
                    "@DomainFromParameter index $sourceParameter is invalid for $method"
                }
                bindReturnDomainSource(method, sourceParameter, authority)
            }
            bindDeclarationSemantics(SemanticTarget.Return(method), Type.getReturnType(method.desc), it, context, authority)
        }
        val argumentTypes = Type.getArgumentTypes(method.desc)
        parameters.forEachIndexed { index, parameter ->
            bindDeclarationSemantics(
                SemanticTarget.Parameter(MethodParameterSig(method, index)),
                argumentTypes[index],
                parameter.annotations,
                context,
                authority,
            )
        }
    }

    private fun resolveConsumerDomain(annotation: AnnotationExpr, context: JavaSourceContext): String {
        val domain = resolveDomain(annotationClassName(annotation), context)
        val expected = when (annotation.nameAsString.substringAfterLast('.')) {
            "Domain" -> SemanticDomainKind.VALUE
            "Flags" -> SemanticDomainKind.FLAGS
            else -> error("unsupported semantic consumer annotation: ${annotation.nameAsString}")
        }
        if (expected == SemanticDomainKind.VALUE) {
            require(domains.getValue(domain).kind in setOf(SemanticDomainKind.VALUE, SemanticDomainKind.PACKED, SemanticDomainKind.NUMERIC, SemanticDomainKind.STRING)) {
                "@Domain requires a value, packed, numeric, or string domain: $domain"
            }
        } else requireDomainKind(domain, expected, annotation.nameAsString.substringAfterLast('.'))
        return domain
    }

    private fun requireDomainKind(domain: String, expected: SemanticDomainKind, annotation: String) {
        val expectedAnnotation = when (expected) {
            SemanticDomainKind.VALUE -> "ValueDomain"
            SemanticDomainKind.FLAGS -> "FlagDomain"
            SemanticDomainKind.SLOTS -> "SlotDomain"
            SemanticDomainKind.PACKED -> "PackedDomain"
            SemanticDomainKind.NUMERIC -> "NumericDomain"
            SemanticDomainKind.STRING -> "StringDomain"
        }
        require(domains.getValue(domain).kind == expected) {
            "@$annotation requires a @$expectedAnnotation, got: $domain"
        }
    }

    private fun parseIndexDomains(
        annotations: Iterable<AnnotationExpr>,
        context: JavaSourceContext,
    ): List<Pair<Int, String>> = annotationsNamed(annotations, "IndexDomain").map { annotation ->
        val dimensionExpr = annotationValue(annotation, "dimension")
            ?: throw IllegalArgumentException("@IndexDomain dimension is missing")
        val rawDimension = parseIntegralConstant(dimensionExpr)
        require(rawDimension in 0..Int.MAX_VALUE.toLong()) {
            "@IndexDomain dimension must be between 0 and ${Int.MAX_VALUE}"
        }
        val domain = resolveDomain(annotationClassName(annotation, "value"), context)
        requireDomainKind(domain, SemanticDomainKind.VALUE, "IndexDomain")
        rawDimension.toInt() to domain
    }

    private fun parseSlotDomains(
        annotations: Iterable<AnnotationExpr>,
        dimensions: Int,
        context: JavaSourceContext,
    ): List<Pair<Int, String>> = annotationsNamed(annotations, "Slots").map { annotation ->
        require(dimensions > 0) { "@Slots requires an array declaration" }
        val dimension = annotationValue(annotation, "dimension")?.let { expression ->
            val raw = parseIntegralConstant(expression)
            require(raw in 0L until dimensions.toLong()) {
                "@Slots dimension must be between 0 and ${dimensions - 1}"
            }
            raw.toInt()
        } ?: run {
            require(dimensions == 1) { "@Slots on a multidimensional array requires an explicit dimension" }
            0
        }
        val domain = resolveDomain(annotationClassName(annotation), context)
        requireDomainKind(domain, SemanticDomainKind.SLOTS, "Slots")
        dimension to domain
    }
}

internal fun annotationInteger(annotation: AnnotationExpr, name: String, default: Int? = null): Int {
    val expression = annotationValue(annotation, name)
        ?: return default ?: throw IllegalArgumentException("@${annotation.nameAsString} $name is missing")
    val value = parseIntegralConstant(expression)
    require(value in 0..Int.MAX_VALUE.toLong()) { "@${annotation.nameAsString} $name must be between 0 and ${Int.MAX_VALUE}" }
    return value.toInt()
}

private fun annotationsNamed(annotations: Iterable<AnnotationExpr>, name: String): List<AnnotationExpr> =
    annotations.filter { it.nameAsString.substringAfterLast('.') == name }

private fun consumerDomainAnnotation(annotations: Iterable<AnnotationExpr>): AnnotationExpr? {
    val matches = annotations.filter { it.nameAsString.substringAfterLast('.') in setOf("Domain", "Flags") }
    require(matches.size <= 1) { "only one of @Domain or @Flags may be used on a declaration" }
    return matches.singleOrNull()
}

private fun parseReturnDomainSource(annotations: Iterable<AnnotationExpr>): Int? {
    val matches = annotationsNamed(annotations, "DomainFromParameter")
    require(matches.size <= 1) { "only one @DomainFromParameter annotation may be used on a method" }
    val annotation = matches.singleOrNull() ?: return null
    val expression = annotationValue(annotation)
        ?: throw IllegalArgumentException("@DomainFromParameter parameter index is missing")
    val sourceParameter = parseIntegralConstant(expression)
    require(sourceParameter in 0..Int.MAX_VALUE.toLong()) {
        "@DomainFromParameter index must be between 0 and ${Int.MAX_VALUE}"
    }
    return sourceParameter.toInt()
}
