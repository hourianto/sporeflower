package j2me.output

import j2me.common.validateClassName
import j2me.common.mappedClassName
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import net.fabricmc.mappingio.MappedElementKind
import net.fabricmc.mappingio.MappingWriter
import net.fabricmc.mappingio.format.MappingFormat
import net.fabricmc.mappingio.tree.MemoryMappingTree
import net.fabricmc.mappingio.tree.VisitOrder
import org.objectweb.asm.Type
import java.nio.file.Path
import kotlin.io.path.createDirectories

private fun shouldEmitTinyOwner(owner: String, cmap: CanonicalMap): Boolean =
    owner in cmap.classes || validateClassName(mappedClassName(owner, cmap))

private fun buildMappingTree(
    cmap: CanonicalMap,
    allOwners: Iterable<String>,
    symbolsByClass: Map<String, ClassSymbols>,
): MemoryMappingTree {
    val fieldByOwner = cmap.fields.entries.groupBy { it.key.owner }
    val methodByOwner = cmap.methods.entries.groupBy { it.key.owner }
    val owners = allOwners.toSet() + cmap.classes.keys + fieldByOwner.keys + methodByOwner.keys

    val tree = MemoryMappingTree()
    tree.visitNamespaces("official", listOf("named"))

    for (owner in owners.sorted()) {
        if (!shouldEmitTinyOwner(owner, cmap)) {
            continue
        }

        tree.visitClass(owner)
        tree.visitDstName(MappedElementKind.CLASS, 0, mappedClassName(owner, cmap))

        val fields = fieldByOwner[owner].orEmpty().sortedWith(compareBy({ it.key.name }, { it.key.desc }, { it.value }))
        for ((sig, target) in fields) {
            tree.visitField(sig.name, sig.desc)
            tree.visitDstName(MappedElementKind.FIELD, 0, target)
        }

        val methods = methodByOwner[owner].orEmpty().sortedWith(compareBy({ it.key.name }, { it.key.desc }, { it.value }))
        for ((sig, target) in methods) {
            tree.visitMethod(sig.name, sig.desc)
            tree.visitDstName(MappedElementKind.METHOD, 0, target)

            val params = cmap.methodArgs[sig] ?: continue
            val argTypes = Type.getArgumentTypes(sig.desc)
            if (argTypes.size != params.size) {
                continue
            }

            var lvIndex = if (symbolsByClass[sig.owner]?.isMethodStatic(sig) == true) 0 else 1
            for ((idx, argType) in argTypes.withIndex()) {
                val paramName = params[idx]
                if (!paramName.isNullOrBlank()) {
                    tree.visitMethodArg(idx, lvIndex, "p$idx")
                    tree.visitDstName(MappedElementKind.METHOD_ARG, 0, paramName)
                }
                lvIndex += argType.size
            }
        }
    }

    return tree
}

fun writeTinyMapping(
    path: Path,
    cmap: CanonicalMap,
    symbolsByClass: Map<String, ClassSymbols>,
    allOwners: Iterable<String>,
) {
    val tree = buildMappingTree(
        cmap = cmap,
        allOwners = allOwners,
        symbolsByClass = symbolsByClass,
    )
    path.parent?.createDirectories()
    MappingWriter.create(path, MappingFormat.TINY_2_FILE).use { writer ->
        tree.accept(writer, VisitOrder.createByInputOrder())
    }
}
