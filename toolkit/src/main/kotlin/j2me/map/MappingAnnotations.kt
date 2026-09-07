package j2me.map

import com.github.javaparser.ast.expr.*

internal fun annotationNamed(annotations: Iterable<AnnotationExpr>, name: String): AnnotationExpr? =
    annotations.firstOrNull { it.nameAsString.substringAfterLast('.') == name }

internal fun annotationValue(annotation: AnnotationExpr, name: String = "value"): Expression? = when (annotation) {
    is SingleMemberAnnotationExpr -> if (name == "value") annotation.memberValue else null
    is NormalAnnotationExpr -> annotation.pairs.firstOrNull { it.nameAsString == name }?.value
    else -> null
}

internal fun annotationClassName(annotation: AnnotationExpr, name: String = "value"): String {
    val expr = annotationValue(annotation, name)
        ?: throw IllegalArgumentException("@$name annotation value is missing")
    return (expr as? ClassExpr)?.typeAsString
        ?: throw IllegalArgumentException("annotation value must be a class literal, got: $expr")
}

internal fun parseIntegralConstant(expr: Expression): Long = when (expr) {
    is CharLiteralExpr -> expr.asChar().code.toLong()
    is IntegerLiteralExpr -> expr.asNumber().toLong()
    is LongLiteralExpr -> expr.asNumber().toLong()
    is UnaryExpr -> when (expr.operator) {
        UnaryExpr.Operator.MINUS -> -parseIntegralConstant(expr.expression)
        UnaryExpr.Operator.PLUS -> parseIntegralConstant(expr.expression)
        else -> throw IllegalArgumentException("constant must be an integral literal, got: $expr")
    }
    else -> throw IllegalArgumentException("constant must be an integral literal, got: $expr")
}

internal fun annotationInteger(annotation: AnnotationExpr, name: String, default: Int? = null): Int {
    val expression = annotationValue(annotation, name)
        ?: return default ?: throw IllegalArgumentException("@${annotation.nameAsString} $name is missing")
    val value = parseIntegralConstant(expression)
    require(value in 0..Int.MAX_VALUE.toLong()) { "@${annotation.nameAsString} $name must be between 0 and ${Int.MAX_VALUE}" }
    return value.toInt()
}

internal fun annotationsNamed(annotations: Iterable<AnnotationExpr>, name: String): List<AnnotationExpr> =
    annotations.filter { it.nameAsString.substringAfterLast('.') == name }

internal fun annotationLong(annotation: AnnotationExpr, name: String, default: Long? = null): Long =
    annotationValue(annotation, name)?.let(::parseIntegralConstant)
        ?: default ?: throw IllegalArgumentException("@${annotation.nameAsString} $name is missing")
