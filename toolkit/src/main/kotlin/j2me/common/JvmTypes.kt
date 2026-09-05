package j2me.common

import org.objectweb.asm.Type

val primitiveToDescriptor = mapOf(
    "byte" to "B",
    "char" to "C",
    "double" to "D",
    "float" to "F",
    "int" to "I",
    "long" to "J",
    "short" to "S",
    "boolean" to "Z",
    "void" to "V",
)

val descriptorToPrimitive = primitiveToDescriptor.entries.associate { (name, desc) -> desc.single() to name }

val primitiveTypeNames: Set<String> = primitiveToDescriptor.keys

fun asmTypeToName(type: Type): String {
    return when (type.sort) {
        Type.VOID -> "void"
        Type.BOOLEAN -> "boolean"
        Type.CHAR -> "char"
        Type.BYTE -> "byte"
        Type.SHORT -> "short"
        Type.INT -> "int"
        Type.FLOAT -> "float"
        Type.LONG -> "long"
        Type.DOUBLE -> "double"
        Type.OBJECT -> type.internalName
        Type.ARRAY -> asmTypeToName(type.elementType) + "[]".repeat(type.dimensions)
        else -> error("Unsupported ASM type: $type")
    }
}
