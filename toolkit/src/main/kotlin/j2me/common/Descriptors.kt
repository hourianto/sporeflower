package j2me.common

import org.objectweb.asm.Type

data class ParsedTypeDescriptor(
    val typeName: String,
    val nextIndex: Int,
)

fun parseTypeDescriptor(desc: String, start: Int = 0): ParsedTypeDescriptor {
    require(start in desc.indices) { "Bad descriptor index $start for: $desc" }
    val type = Type.getType(desc.substring(start))
    return ParsedTypeDescriptor(asmTypeToName(type), start + type.descriptor.length)
}

fun parseMethodDescriptor(desc: String): Pair<List<String>, String> {
    val methodType = Type.getMethodType(desc)
    val args = methodType.argumentTypes.map(::asmTypeToName)
    val ret = asmTypeToName(methodType.returnType)
    return args to ret
}

fun remapTypeName(typeName: String, classMap: Map<String, String>): String {
    if (typeName in primitiveTypeNames) {
        return typeName
    }

    var arrayDepth = 0
    var base = typeName
    while (base.endsWith("[]")) {
        arrayDepth += 1
        base = base.removeSuffix("[]")
    }

    val mappedBase = if (base in primitiveTypeNames) base else classMap[base] ?: base
    return mappedBase + "[]".repeat(arrayDepth)
}

fun methodArgSlotSize(typeName: String): Int = if (typeName == "long" || typeName == "double") 2 else 1
