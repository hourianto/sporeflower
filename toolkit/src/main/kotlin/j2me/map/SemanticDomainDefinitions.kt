package j2me.map

import com.github.javaparser.ast.expr.AnnotationExpr
import j2me.common.JavaSourceContext
import j2me.model.SemanticBitField
import j2me.model.SemanticDomain
import j2me.model.SemanticNumberFormat

internal fun completeSemanticDomain(
    domain: SemanticDomain,
    annotations: Iterable<AnnotationExpr>,
    context: JavaSourceContext,
    builder: SemanticMapBuilder,
): SemanticDomain {
    val fields = annotations.filter { it.nameAsString.substringAfterLast('.') == "BitField" }.map { annotation ->
        SemanticBitField(
            builder.resolveDomain(annotationClassName(annotation), context),
            annotationInteger(annotation, "shift", 0),
            annotationInteger(annotation, "bits"),
            annotationValue(annotation, "signed")?.let {
                require(it.isBooleanLiteralExpr) { "@BitField signed must be a boolean literal" }
                it.asBooleanLiteralExpr().value
            } ?: false,
            annotationLong(annotation, "selectorMask", 0),
            annotationLong(annotation, "selectorValue", 0),
        )
    }
    val format = annotations.singleOrNull { it.nameAsString.substringAfterLast('.') == "NumericDomain" }?.let { annotation ->
        val value = annotationValue(annotation, "format")
        require(value != null && value.isStringLiteralExpr) { "@NumericDomain requires a format string" }
        SemanticNumberFormat(value.asStringLiteralExpr().asString(), annotationInteger(annotation, "fractionBits", 0))
    }
    return domain.copy(bitFields = fields, format = format)
}

internal fun annotationLong(annotation: AnnotationExpr, name: String, default: Long? = null): Long =
    annotationValue(annotation, name)?.let(::parseIntegralConstant)
        ?: default ?: throw IllegalArgumentException("@${annotation.nameAsString} $name is missing")
