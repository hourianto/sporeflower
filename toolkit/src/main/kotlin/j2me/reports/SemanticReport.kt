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
)

data class SemanticStats(
    val domains: List<SemanticDomainStats>,
    val fieldArrayBindings: Int,
    val returnArrayBindings: Int,
    val parameterArrayBindings: Int,
    val returnDomainSources: Int,
) {
    val domainTotal: Int get() = domains.size
    val valueDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.VALUE }
    val flagDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.FLAGS }
    val slotDomainTotal: Int get() = domains.count { it.kind == SemanticDomainKind.SLOTS }
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
            valueTotal = definition.syntheticValues.size + count(semantic.realValues.values, domain),
            syntheticValueTotal = definition.syntheticValues.size,
            fieldBindings = countScalars(domain) { it is SemanticTarget.Field },
            returnBindings = countScalars(domain) { it is SemanticTarget.Return },
            parameterBindings = countScalars(domain) { it is SemanticTarget.Parameter },
            indexBindings = semantic.arraySemantics.values.sumOf { count(it.indexDomains.values, domain) },
            slotBindings = semantic.arraySemantics.values.sumOf { count(it.slotDomains.values, domain) },
            elementBindings = semantic.arraySemantics.values.count { it.elementDomain == domain },
            slotValueLinks = semantic.domains.values.sumOf { source ->
                source.syntheticValues.count { it.elementDomain == domain }
            },
        )
    }

    return SemanticStats(
        domains,
        semantic.arraySemantics.keys.count { it is SemanticTarget.Field },
        semantic.arraySemantics.keys.count { it is SemanticTarget.Return },
        semantic.arraySemantics.keys.count { it is SemanticTarget.Parameter },
        semantic.returnDomainSources.size,
    )
}

fun writeSemanticReport(path: Path, semantic: SemanticMap): SemanticStats {
    val stats = semanticStats(semantic)
    val content = buildString {
        appendLine("Semantic mappings")
        appendLine("=================")
        appendLine()
        appendLine(
            "Domains: ${stats.domainTotal} " +
                "(${stats.valueDomainTotal} value, ${stats.flagDomainTotal} flags, ${stats.slotDomainTotal} slots)",
        )
        appendLine("Named values: ${stats.valueTotal} (${stats.realValueTotal} bytecode fields, ${stats.syntheticValueTotal} source-only)")
        appendLine(
            "Scalar bindings: ${stats.fieldBindings} fields, ${stats.returnBindings} returns, " +
                "${stats.parameterBindings} parameters",
        )
        appendLine("Return domain sources: ${stats.returnDomainSources}")
        appendLine(
            "Array bindings: ${stats.arrayBindingTotal} arrays " +
                "(${stats.fieldArrayBindings} fields, ${stats.returnArrayBindings} returns, " +
                "${stats.parameterArrayBindings} parameters)",
        )
        appendLine(
            "Array semantics: ${stats.indexBindings} index dimensions, ${stats.slotBindings} slot dimensions, " +
                "${stats.elementBindings} leaf-value bindings, ${stats.slotValueLinks} slot-value links",
        )
        appendLine()
        appendLine("| Domain | Kind | Values | Fields | Returns | Parameters | Indexes | Slots | Elements | Slot links |")
        appendLine("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for (domain in stats.domains) {
            val values = if (domain.syntheticValueTotal == 0) {
                domain.valueTotal.toString()
            } else {
                "${domain.valueTotal} (${domain.syntheticValueTotal} source-only)"
            }
            appendLine(
                "| ${domain.id} | ${domain.kind.name.lowercase()} | $values | ${domain.fieldBindings} | " +
                    "${domain.returnBindings} | ${domain.parameterBindings} | ${domain.indexBindings} | " +
                    "${domain.slotBindings} | ${domain.elementBindings} | ${domain.slotValueLinks} |",
            )
        }
    }
    path.parent?.createDirectories()
    path.writeText(content)
    return stats
}
