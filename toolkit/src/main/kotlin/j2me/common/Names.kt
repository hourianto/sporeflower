package j2me.common

import com.github.javaparser.ParseProblemException
import com.github.javaparser.ast.type.ArrayType
import com.github.javaparser.ast.type.ClassOrInterfaceType
import com.github.javaparser.ast.type.IntersectionType
import com.github.javaparser.ast.type.Type
import com.github.javaparser.ast.type.UnionType

private val identRegex = Regex("^[A-Za-z_$][A-Za-z0-9_$]*$")
val wasMemberRegex = Regex("/\\*\\s*was\\s+([A-Za-z_$][A-Za-z0-9_$]*)\\s*\\*/")
val wasClassOwnerRegex = Regex("/\\*\\s*was\\s+([A-Za-z_$][A-Za-z0-9_$]*(?:[./][A-Za-z_$][A-Za-z0-9_$]*)*)\\s*\\*/")

val javaLangCommonTypes = setOf(
    "Boolean",
    "Byte",
    "Character",
    "Class",
    "Double",
    "Enum",
    "Error",
    "Exception",
    "Float",
    "IllegalArgumentException",
    "Integer",
    "Long",
    "Math",
    "Number",
    "Object",
    "Runnable",
    "RuntimeException",
    "Short",
    "String",
    "StringBuffer",
    "StringBuilder",
    "System",
    "Thread",
    "Throwable",
    "Void",
)

private val javaKeywords = setOf(
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const",
    "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float",
    "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native",
    "new", "package", "private", "protected", "public", "return", "short", "static", "strictfp",
    "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void",
    "volatile", "while", "true", "false", "null",
)

fun readableClassToInternal(name: String): String = name.replace('.', '/')

fun internalToJava(name: String): String = name.replace('/', '.')

fun isValidIdentifier(name: String): Boolean = identRegex.matches(name) && name !in javaKeywords

fun validateClassName(internalName: String): Boolean = internalName.split('/').all(::isValidIdentifier)

fun validateBytecodeClassName(internalName: String): Boolean = internalName
    .split('/')
    .all { identRegex.matches(it) }

fun isValidReadableClassName(name: String): Boolean {
    val parts = name.split('.')
    return parts.isNotEmpty() && parts.all(::isValidIdentifier)
}

private fun eraseType(type: Type): String {
    return when (type) {
        is ArrayType -> eraseType(type.componentType) + "[]"
        is ClassOrInterfaceType -> {
            val scope = type.scope.map { eraseType(it) + "." }.orElse("")
            scope + type.nameAsString
        }
        is UnionType -> eraseType(type.elements.first())
        is IntersectionType -> eraseType(type.elements.first())
        else -> type.asString()
    }
}

fun normalizeTypeExpr(typeExpr: String): String {
    val normalized = typeExpr.trim().replace("...", "[]")
    if (normalized.isBlank()) {
        return normalized
    }
    return try {
        eraseType(parseTypeNode(normalized))
    } catch (_: ParseProblemException) {
        normalized.split(Regex("\\s+")).filter { it.isNotBlank() }.joinToString(" ").replace(" []", "[]")
    }
}

fun normalizeTypeNode(type: Type): String = eraseType(type)
