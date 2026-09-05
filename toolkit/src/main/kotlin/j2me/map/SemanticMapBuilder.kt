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
            )
        },
        returnDomainSources = returnDomainSources,
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
        parseSlotDomain(annotations, dimensions, context)?.let { (dimension, domain) ->
            bindSlots(target, dimension, domain, authority)
        }
        parseIndexDomains(annotations, context).forEach { (dimension, domain) ->
            bindIndex(target, dimension, domain, authority)
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
        requireDomainKind(domain, expected, annotation.nameAsString.substringAfterLast('.'))
        return domain
    }

    private fun requireDomainKind(domain: String, expected: SemanticDomainKind, annotation: String) {
        val expectedAnnotation = when (expected) {
            SemanticDomainKind.VALUE -> "ValueDomain"
            SemanticDomainKind.FLAGS -> "FlagDomain"
            SemanticDomainKind.SLOTS -> "SlotDomain"
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

    private fun parseSlotDomain(
        annotations: Iterable<AnnotationExpr>,
        dimensions: Int,
        context: JavaSourceContext,
    ): Pair<Int, String>? {
        val matches = annotationsNamed(annotations, "Slots")
        require(matches.size <= 1) { "only one @Slots annotation may be used on a declaration" }
        val annotation = matches.singleOrNull() ?: return null
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
        return dimension to domain
    }
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
