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

/** Built-in API contracts win over project annotations; peers must not bind a key twice. */
private class BindingTable<K, V>(private val label: String) {
    private data class Binding<V>(val value: V, val authority: MapAuthority)
    private val entries = linkedMapOf<K, Binding<V>>()
    val values: Map<K, V> get() = entries.mapValues { it.value.value }
    operator fun get(key: K): V? = entries[key]?.value

    fun bind(key: K, value: V, authority: MapAuthority) {
        val previous = entries[key]
        if (previous?.authority == MapAuthority.BUILTIN && authority == MapAuthority.PROJECT) return
        require(previous == null) { "duplicate $label for $key" }
        entries[key] = Binding(value, authority)
    }
}

internal class SemanticMapBuilder(
    private val domains: MutableMap<String, SemanticDomain>,
) {
    private data class MutableArraySemantics(
        val indexDomains: BindingTable<Int, String> = BindingTable("@IndexDomain"),
        val slotDomains: BindingTable<Int, String> = BindingTable("@Slots"),
        val records: BindingTable<Int, SemanticRecordLayout> = BindingTable("@Records"),
    )

    private val realValues = BindingTable<FieldSig, String>("semantic value binding")
    private val scalarDomains = BindingTable<SemanticTarget, String>("semantic binding")
    private val elementDomains = BindingTable<SemanticTarget, String>("semantic element binding")
    private val arraySemantics = linkedMapOf<SemanticTarget, MutableArraySemantics>()
    private val returnDomainSources = BindingTable<MethodSig, Int>("@DomainFromParameter binding")
    private val callDomains = BindingTable<SemanticCallSite, String>("@CallDomain binding")
    private val slotDomainSources = BindingTable<SemanticTarget, SemanticSlotSource>("@DomainFromSlot binding")
    private val conditionalDomains = linkedMapOf<SemanticTarget, MutableList<SemanticCondition>>()
    private val containers = linkedMapOf<SemanticTarget, SemanticContainer>()
    private val classNames = linkedSetOf<SemanticTarget>()

    fun bindRealValue(field: FieldSig, domain: String, authority: MapAuthority) = realValues.bind(field, domain, authority)

    private fun array(target: SemanticTarget) = arraySemantics.getOrPut(target, ::MutableArraySemantics)

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
        realValues = realValues.values,
        scalarDomains = scalarDomains.values,
        arraySemantics = arraySemantics.mapValues { (target, semantics) ->
            SemanticArraySemantics(
                indexDomains = semantics.indexDomains.values,
                slotDomains = semantics.slotDomains.values,
                elementDomain = elementDomains[target],
                records = semantics.records.values,
            )
        },
        returnDomainSources = returnDomainSources.values,
        callDomains = callDomains.values,
        conditionalDomains = conditionalDomains,
        containers = containers,
        slotDomainSources = slotDomainSources.values,
        classNames = classNames,
    )

    fun bindDeclarationSemantics(
        target: SemanticTarget,
        type: Type,
        annotations: Iterable<AnnotationExpr>,
        context: JavaSourceContext,
        authority: MapAuthority,
    ) {
        annotationsNamed(annotations, "ClassName").forEach {
            require(it.isMarkerAnnotationExpr) { "@ClassName takes no arguments" }
            require(classNames.add(target)) { "duplicate @ClassName binding for $target" }
        }
        consumerDomainAnnotation(annotations)?.let { annotation ->
            val domain = resolveConsumerDomain(annotation, context)
            if (type.sort == Type.ARRAY) {
                array(target)
                elementDomains.bind(target, domain, authority)
            } else scalarDomains.bind(target, domain, authority)
        }
        // ASM's getDimensions() assumes an array type and reads past the
        // descriptor buffer for some primitives, notably double.
        val dimensions = if (type.sort == Type.ARRAY) type.dimensions else 0
        parseSlotDomains(annotations, dimensions, context).forEach { (dimension, domain) ->
            array(target).slotDomains.bind(dimension, domain, authority)
        }
        parseIndexDomains(annotations, context).forEach { (dimension, domain) ->
            array(target).indexDomains.bind(dimension, domain, authority)
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
            array(target).records.bind(dimension, layout, authority)
        }
        annotationsNamed(annotations, "DomainFromSlot").forEach { annotation ->
            require(target !is SemanticTarget.Field) { "@DomainFromSlot requires a method parameter or return" }
            slotDomainSources.bind(target, SemanticSlotSource(
                annotationInteger(annotation, "parameter"), annotationInteger(annotation, "slot"),
                annotationValue(annotation, "dimension")?.let { annotationInteger(annotation, "dimension") },
            ), authority)
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
                val parameter = annotationValue(annotation, "parameter")?.let { annotationInteger(annotation, "parameter") }
                require(parameter == null || parameter >= 0) { "@CallDomain parameter must be nonnegative" }
                val domain = resolveDomain(annotationClassName(annotation), context)
                require(domains.getValue(domain).kind != SemanticDomainKind.SLOTS) {
                    "@CallDomain requires a scalar domain"
                }
                callDomains.bind(SemanticCallSite(method, offset, parameter), domain, authority)
            }
            parseReturnDomainSource(it)?.let { sourceParameter ->
                require(consumerDomainAnnotation(it) == null) {
                    "@DomainFromParameter cannot be combined with @Domain or @Flags"
                }
                require(sourceParameter in parameters.indices) {
                    "@DomainFromParameter index $sourceParameter is invalid for $method"
                }
                returnDomainSources.bind(method, sourceParameter, authority)
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
        val dimension = annotationInteger(annotation, "dimension")
        val domain = resolveDomain(annotationClassName(annotation, "value"), context)
        requireDomainKind(domain, SemanticDomainKind.VALUE, "IndexDomain")
        dimension to domain
    }

    private fun parseSlotDomains(
        annotations: Iterable<AnnotationExpr>,
        dimensions: Int,
        context: JavaSourceContext,
    ): List<Pair<Int, String>> = annotationsNamed(annotations, "Slots").map { annotation ->
        require(dimensions > 0) { "@Slots requires an array declaration" }
        require(dimensions == 1 || annotationValue(annotation, "dimension") != null) {
            "@Slots on a multidimensional array requires an explicit dimension"
        }
        val dimension = annotationInteger(annotation, "dimension", 0)
        require(dimension < dimensions) { "@Slots dimension must be between 0 and ${dimensions - 1}" }
        val domain = resolveDomain(annotationClassName(annotation), context)
        requireDomainKind(domain, SemanticDomainKind.SLOTS, "Slots")
        dimension to domain
    }
}

private fun consumerDomainAnnotation(annotations: Iterable<AnnotationExpr>): AnnotationExpr? {
    val matches = annotations.filter { it.nameAsString.substringAfterLast('.') in setOf("Domain", "Flags") }
    require(matches.size <= 1) { "only one of @Domain or @Flags may be used on a declaration" }
    return matches.singleOrNull()
}

private fun parseReturnDomainSource(annotations: Iterable<AnnotationExpr>): Int? {
    val matches = annotationsNamed(annotations, "DomainFromParameter")
    require(matches.size <= 1) { "only one @DomainFromParameter annotation may be used on a method" }
    val annotation = matches.singleOrNull() ?: return null
    return annotationInteger(annotation, "value")
}
