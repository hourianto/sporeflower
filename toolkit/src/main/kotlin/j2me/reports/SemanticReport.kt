package j2me.reports

import j2me.model.SemanticDomainKind
import j2me.model.SemanticMap
import j2me.model.SemanticTarget
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

data class SemanticDomainStats(
    val id: String,
    val kind: SemanticDomainKind,
    val valueTotal: Int,
    val syntheticValueTotal: Int,
    val fieldBindings: Int,
    val returnBindings: Int,
    val parameterBindings: Int,
    val indexBindings: Int,
    val slotBindings: Int,
    val elementBindings: Int,
    val slotValueLinks: Int,
    val recordBindings: Int,
    val planeBindings: Int,
    val bitFieldLinks: Int,
    val callBindings: Int,
    val conditionalBindings: Int,
    val containerBindings: Int,
)

data class SemanticStats(
    val domains: List<SemanticDomainStats>,
    val fieldArrayBindings: Int,
    val returnArrayBindings: Int,
    val parameterArrayBindings: Int,
    val returnDomainSources: Int,
    val slotDomainSources: Int,
) {
    val domainTotal: Int get() = domains.size
    val valueDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.VALUE }
    val flagDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.FLAGS }
    val slotDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.SLOTS }
    val packedDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.PACKED }
    val numericDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.NUMERIC }
    val stringDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.STRING }
    val valueTotal: Int get() = domains.sumOf { it.valueTotal }
    val syntheticValueTotal: Int get() = domains.sumOf { it.syntheticValueTotal }
    val realValueTotal: Int get() = valueTotal - syntheticValueTotal
    val fieldBindings: Int get() = domains.sumOf { it.fieldBindings }
    val returnBindings: Int get() = domains.sumOf { it.returnBindings }
    val parameterBindings: Int get() = domains.sumOf { it.parameterBindings }
    val indexBindings: Int get() = domains.sumOf { it.indexBindings }
    val slotBindings: Int get() = domains.sumOf { it.slotBindings }
    val elementBindings: Int get() = domains.sumOf { it.elementBindings }
    val slotValueLinks: Int get() = domains.sumOf { it.slotValueLinks }
    val recordBindings: Int get() = domains.sumOf { it.recordBindings }
    val planeBindings: Int get() = domains.sumOf { it.planeBindings }
    val bitFieldLinks: Int get() = domains.sumOf { it.bitFieldLinks }
    val callBindings: Int get() = domains.sumOf { it.callBindings }
    val conditionalBindings: Int get() = domains.sumOf { it.conditionalBindings }
    val containerBindings: Int get() = domains.sumOf { it.containerBindings }
    val arrayBindingTotal: Int get() = fieldArrayBindings + returnArrayBindings + parameterArrayBindings
}

fun semanticStats(semantic: SemanticMap): SemanticStats {
    fun count(values: Collection<String>, domain: String): Int = values.count { it == domain }
    fun countScalars(domain: String, targetKind: (SemanticTarget) -> Boolean): Int =
        semantic.scalarDomains.count { (target, boundDomain) -> boundDomain == domain && targetKind(target) }

    val domains = semantic.domains.values.sortedBy { it.id }.map { definition ->
        val domain = definition.id
        SemanticDomainStats(
            id = domain,
            kind = definition.kind,
            valueTotal = definition.syntheticValues.size + definition.syntheticStrings.size + count(semantic.realValues.values, domain),
            syntheticValueTotal = definition.syntheticValues.size + definition.syntheticStrings.size,
            fieldBindings = countScalars(domain) { it is SemanticTarget.Field },
            returnBindings = countScalars(domain) { it is SemanticTarget.Return },
            parameterBindings = countScalars(domain) { it is SemanticTarget.Parameter },
            indexBindings = semantic.arraySemantics.values.sumOf { count(it.indexDomains.values, domain) },
            slotBindings = semantic.arraySemantics.values.sumOf { count(it.slotDomains.values, domain) },
            elementBindings = semantic.arraySemantics.values.count { it.elementDomain == domain },
            slotValueLinks = semantic.domains.values.sumOf { source ->
                source.syntheticValues.count { it.elementDomain == domain }
            },
            recordBindings = semantic.arraySemantics.values.sumOf { array -> array.records.values.count { it.domain == domain && !it.planes } },
            planeBindings = semantic.arraySemantics.values.sumOf { array -> array.records.values.count { it.domain == domain && it.planes } },
            bitFieldLinks = semantic.domains.values.sumOf { source -> source.bitFields.count { it.domain == domain } },
            callBindings = semantic.callDomains.values.count { it == domain },
            conditionalBindings = semantic.conditionalDomains.values.sumOf { cases -> cases.count { it.domain == domain } },
            containerBindings = semantic.containers.values.sumOf { listOfNotNull(it.elements, it.keys, it.values).count { value -> value == domain } },
        )
    }

    return SemanticStats(
        domains,
        semantic.arraySemantics.keys.count { it is SemanticTarget.Field },
        semantic.arraySemantics.keys.count { it is SemanticTarget.Return },
        semantic.arraySemantics.keys.count { it is SemanticTarget.Parameter },
        semantic.returnDomainSources.size,
        semantic.slotDomainSources.size,
    )
}

fun writeSemanticReport(path: Path, semantic: SemanticMap): SemanticStats {
    val stats = semanticStats(semantic)
    val content = buildString {
        appendLine("Semantic mappings")
        appendLine("=================")
        appendLine()
        appendLine("Counts describe declared contracts and domain references, not source substitutions or proven coverage.")
        appendLine()
        appendLine(
            "Domains: ${stats.domainTotal} " +
                "(${stats.valueDomainTotal} value, ${stats.flagDomainTotal} flags, ${stats.slotDomainTotal} slots, " +
                "${stats.packedDomainTotal} packed, ${stats.numericDomainTotal} numeric, ${stats.stringDomainTotal} string)",
        )
        appendLine("Named values: ${stats.valueTotal} (${stats.realValueTotal} bytecode fields, ${stats.syntheticValueTotal} source-only)")
        appendLine(
            "Scalar bindings: ${stats.fieldBindings} fields, ${stats.returnBindings} returns, " +
                "${stats.parameterBindings} parameters",
        )
        appendLine("Return domain sources: ${stats.returnDomainSources}; table-column sources: ${stats.slotDomainSources}")
        appendLine("Scoped call bindings: ${stats.callBindings}")
        appendLine("Conditional bindings: ${stats.conditionalBindings}; container roles: ${stats.containerBindings}")
        appendLine(
            "Array bindings: ${stats.arrayBindingTotal} arrays " +
                "(${stats.fieldArrayBindings} fields, ${stats.returnArrayBindings} returns, " +
                "${stats.parameterArrayBindings} parameters)",
        )
        appendLine(
            "Array semantics: ${stats.indexBindings} index dimensions, ${stats.slotBindings} slot dimensions, " +
                "${stats.recordBindings} record dimensions, ${stats.planeBindings} plane dimensions, " +
                "${stats.elementBindings} leaf-value bindings, ${stats.slotValueLinks} slot-value links, ${stats.bitFieldLinks} bit-field links",
        )
        appendLine()
        appendLine("| Domain | Kind | Values | Fields | Returns | Parameters | Indexes | Slots | Elements | Slot links | Records | Planes | Bit-field links | Calls | Conditional | Container roles |")
        appendLine("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for (domain in stats.domains) {
            val values = if (domain.syntheticValueTotal == 0) {
                domain.valueTotal.toString()
            } else {
                "${domain.valueTotal} (${domain.syntheticValueTotal} source-only)"
            }
            appendLine(
                "| ${domain.id} | ${domain.kind.name.lowercase()} | $values | ${domain.fieldBindings} | " +
                    "${domain.returnBindings} | ${domain.parameterBindings} | ${domain.indexBindings} | " +
                    "${domain.slotBindings} | ${domain.elementBindings} | ${domain.slotValueLinks} | ${domain.recordBindings} | " +
                    "${domain.planeBindings} | ${domain.bitFieldLinks} | ${domain.callBindings} | ${domain.conditionalBindings} | ${domain.containerBindings} |",
            )
        }
    }
    path.parent?.createDirectories()
    path.writeText(content)
    return stats
}
