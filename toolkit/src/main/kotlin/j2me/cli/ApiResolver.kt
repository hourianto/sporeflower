package j2me.cli

import j2me.common.isJavaClassFile
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassVisitor
import org.objectweb.asm.FieldVisitor
import org.objectweb.asm.MethodVisitor
import org.objectweb.asm.Opcodes
import org.objectweb.asm.Type
import org.objectweb.asm.tree.ClassNode
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.attribute.FileTime
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.jar.JarFile
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.io.path.createDirectories
import kotlin.io.path.isRegularFile

private data class ApiMember(val name: String, val descriptor: String, val field: Boolean, val static: Boolean? = null)
private data class ApiClass(val bytes: ByteArray, val parents: List<String>, val members: Set<ApiMember>) {
    fun declares(member: ApiMember): Boolean = if (member.static == null) {
        member.copy(static = false) in members || member.copy(static = true) in members
    } else {
        member in members
    }
}
private data class ApiLibrary(val path: Path, val classes: Map<String, ApiClass>, val fallback: Boolean)
private data class CachedApiLibrary(val size: Long, val modified: FileTime, val library: ApiLibrary)
private val apiLibraryCache = ConcurrentHashMap<Path, CachedApiLibrary>()
private val apiSnapshotLock = Any()

private fun manifestAttribute(jar: JarFile, name: String): String? = try {
    jar.manifest?.mainAttributes?.getValue(name)
} catch (_: IOException) {
    // Manifest metadata is advisory. Broken headers must not reject usable code.
    null
}

private fun apiLibrary(path: Path): ApiLibrary {
    val size = Files.size(path)
    val modified = Files.getLastModifiedTime(path)
    // Replace the previous inventory on rebuild instead of retaining every
    // version of a local SDK or stub jar for the lifetime of a fullrun process.
    return apiLibraryCache.compute(path) { _, cached ->
        if (cached != null && cached.size == size && cached.modified == modified) return@compute cached
        val classes = linkedMapOf<String, ApiClass>()
        JarFile(path.toFile()).use { jar ->
            for (entry in jar.entries()) {
                if (entry.isDirectory || !entry.name.endsWith(".class") || entry.name.startsWith("META-INF/")) continue
                val bytes = jar.getInputStream(entry).use { it.readBytes() }
                if (!isJavaClassFile(bytes)) continue
                val node = ClassNode()
                ClassReader(bytes).accept(node, ClassReader.SKIP_CODE or ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
                classes[node.name] = ApiClass(
                    bytes, listOfNotNull(node.superName) + node.interfaces,
                    node.methods.filter { it.access and Opcodes.ACC_PRIVATE == 0 }
                        .map { ApiMember(it.name, it.desc, false, it.access and Opcodes.ACC_STATIC != 0) }.toSet() +
                        node.fields.filter { it.access and Opcodes.ACC_PRIVATE == 0 }
                            .map { ApiMember(it.name, it.desc, true, it.access and Opcodes.ACC_STATIC != 0) },
                )
            }
            CachedApiLibrary(size, modified, ApiLibrary(path, classes, manifestAttribute(jar, "J2ME-Stub-Kind") == "compile-only"))
        }
    }!!.library
}

private data class ApiRequirements(
    val members: Map<String, Set<ApiMember>>,
    val classes: Set<String>,
    val configuration: String?,
    val floatingPoint: Boolean,
    val projectClasses: Map<String, ApiClass>,
)

private fun apiRequirements(projectJar: Path): ApiRequirements {
    val members = linkedMapOf<String, MutableSet<ApiMember>>()
    val classes = linkedSetOf<String>()
    val projectClasses = linkedMapOf<String, ApiClass>()
    var floatingPoint = false
    fun addType(type: Type) {
        when (type.sort) {
            Type.OBJECT -> classes += type.internalName
            Type.ARRAY -> addType(type.elementType)
            Type.FLOAT, Type.DOUBLE -> floatingPoint = true
            Type.METHOD -> {
                type.argumentTypes.forEach(::addType)
                addType(type.returnType)
            }
        }
    }
    JarFile(projectJar.toFile()).use { jar ->
        for (entry in jar.entries()) {
            if (entry.isDirectory || !entry.name.endsWith(".class") || entry.name.startsWith("META-INF/")) continue
            val bytes = jar.getInputStream(entry).use { it.readBytes() }
            if (!isJavaClassFile(bytes)) continue
            val reader = ClassReader(bytes)
            val declaredMembers = linkedSetOf<ApiMember>()
            val chars = CharArray(reader.maxStringLength)
            // Constant-pool references also cover unused or guarded dependencies.
            // JVM return descriptors remain significant when choosing API revisions.
            for (index in 1 until reader.itemCount) {
                val offset = reader.getItem(index)
                if (offset == 0) continue
                when (reader.readByte(offset - 1)) {
                    4, 6 -> floatingPoint = true
                    7 -> {
                        val name = reader.readUTF8(offset, chars)
                        if (name.startsWith("[")) addType(Type.getType(name)) else classes += name
                    }
                    9, 10, 11 -> {
                        val owner = reader.readClass(offset, chars)
                        val nameAndType = reader.getItem(reader.readUnsignedShort(offset + 2))
                        val member = ApiMember(
                            reader.readUTF8(nameAndType, chars), reader.readUTF8(nameAndType + 2, chars),
                            reader.readByte(offset - 1) == 9,
                        )
                        members.getOrPut(owner) { linkedSetOf() } += member
                        addType(Type.getType(member.descriptor))
                    }
                }
            }
            // Skip optional debug ranges when reading invocation kinds: malformed
            // local-variable metadata must not prevent dependency resolution.
            reader.accept(object : ClassVisitor(Opcodes.ASM9) {
                override fun visitField(access: Int, name: String, descriptor: String, signature: String?, value: Any?): FieldVisitor? {
                    addType(Type.getType(descriptor))
                    declaredMembers += ApiMember(name, descriptor, true, access and Opcodes.ACC_STATIC != 0)
                    return null
                }

                override fun visitMethod(access: Int, name: String, descriptor: String, signature: String?, exceptions: Array<out String>?): MethodVisitor {
                    addType(Type.getMethodType(descriptor))
                    declaredMembers += ApiMember(name, descriptor, false, access and Opcodes.ACC_STATIC != 0)
                    return object : MethodVisitor(Opcodes.ASM9) {
                        override fun visitInsn(opcode: Int) {
                            // Constants and integer conversions can introduce
                            // floating point without any float descriptor or CP entry.
                            when (opcode) {
                                Opcodes.FCONST_0, Opcodes.FCONST_1, Opcodes.FCONST_2, Opcodes.DCONST_0, Opcodes.DCONST_1,
                                Opcodes.I2F, Opcodes.I2D, Opcodes.L2F, Opcodes.L2D -> floatingPoint = true
                            }
                        }

                        override fun visitIntInsn(opcode: Int, operand: Int) {
                            if (opcode == Opcodes.NEWARRAY && (operand == Opcodes.T_FLOAT || operand == Opcodes.T_DOUBLE)) {
                                floatingPoint = true
                            }
                        }

                        override fun visitMethodInsn(opcode: Int, owner: String, name: String, descriptor: String, isInterface: Boolean) {
                            members.getOrPut(owner) { linkedSetOf() } += ApiMember(name, descriptor, false, opcode == Opcodes.INVOKESTATIC)
                        }

                        override fun visitFieldInsn(opcode: Int, owner: String, name: String, descriptor: String) {
                            members.getOrPut(owner) { linkedSetOf() } += ApiMember(name, descriptor, true, opcode == Opcodes.GETSTATIC || opcode == Opcodes.PUTSTATIC)
                        }
                    }
                }
            }, ClassReader.SKIP_DEBUG or ClassReader.SKIP_FRAMES)
            projectClasses[reader.className] = ApiClass(bytes, listOfNotNull(reader.superName) + reader.interfaces, declaredMembers)
        }
        // Instruction references refine the CP's unknown invocation kind. Count
        // each requirement once instead of giving used signatures double weight.
        for (references in members.values) {
            val refined = references.filter { it.static != null }.map { it.copy(static = null) }.toSet()
            references.removeAll(refined)
        }
        return ApiRequirements(members, classes, manifestAttribute(jar, "MicroEdition-Configuration"), floatingPoint, projectClasses)
    }
}

/**
 * Use the same class definitions for mapping, decompilation, and recompilation.
 * An ordinary classpath hides later definitions, including newer optional APIs
 * and incompatible vendor revisions. Select whole class definitions, never merge
 * methods or invent missing signatures, and retain the original class bytes.
 */
internal fun resolveApiJars(projectJar: Path, jars: List<Path>, cacheDir: Path): List<Path> {
    if (jars.isEmpty()) return emptyList()
    val libraries = jars.map { it.toAbsolutePath().normalize() }.distinct().sorted().map(::apiLibrary)
    val requirements = apiRequirements(projectJar)
    val providers = linkedMapOf<String, MutableList<ApiLibrary>>()
    for (library in libraries) {
        for (owner in library.classes.keys) providers.getOrPut(owner) { mutableListOf() } += library
    }
    val demands = requirements.members.mapValues { it.value.toMutableSet() }.toMutableMap()

    fun supports(library: ApiLibrary, owner: String, member: ApiMember, visited: MutableSet<String> = hashSetOf()): Boolean {
        if (!visited.add(owner)) return false
        try {
            val alternatives = library.classes[owner]?.let { listOf(it) } ?: providers[owner].orEmpty().map { it.classes.getValue(owner) }
            return alternatives.any { definition ->
                definition.declares(member) || member.name != "<init>" &&
                    definition.parents.any { supports(library, it, member, visited) }
            }
        } finally {
            visited.remove(owner)
        }
    }

    // Comparators ask for the same score repeatedly. Requirements only change
    // between propagation rounds, so cache each score for the current round.
    val missingCounts = mutableMapOf<Pair<Path, String>, Int>()
    fun missing(library: ApiLibrary, owner: String): Int =
        missingCounts.getOrPut(library.path to owner) { demands[owner].orEmpty().count { !supports(library, owner, it) } }

    fun inheritedApiOwner(owner: String, member: ApiMember, visited: MutableSet<String> = hashSetOf()): String? {
        if (!visited.add(owner)) return null
        val own = requirements.projectClasses[owner]
            ?: return owner.takeIf { providers[owner].orEmpty().any { supports(it, owner, member) } }
        if (member.name == "<init>" || own.declares(member)) return null
        return own.parents.firstNotNullOfOrNull { inheritedApiOwner(it, member, visited.toMutableSet()) }
    }
    // A bytecode reference may name a project subclass instead of the API that
    // declares the inherited method (for example, a Thread subclass's interrupt).
    // Resolve those references before choosing the core library version.
    for ((owner, members) in requirements.members) {
        if (owner !in requirements.projectClasses) continue
        for (member in members) {
            val inheritedOwner = inheritedApiOwner(owner, member) ?: continue
            demands.getOrPut(inheritedOwner) { linkedSetOf() } += member
        }
    }

    val bootCandidates = providers["java/lang/Object"].orEmpty()
    // MIDP can add java.* classes (for example IllegalStateException) outside
    // CLDC. Use standalone cores to identify that boundary when available.
    val standaloneCores = bootCandidates.filter { "javax/microedition/midlet/MIDlet" !in it.classes }
    val coreOwners = standaloneCores.ifEmpty { bootCandidates }.flatMap { it.classes.keys }
        .filter { it.startsWith("java/") }.toSet()
    val needsFloatingPoint = requirements.floatingPoint || requirements.classes.any { it == "java/lang/Float" || it == "java/lang/Double" } ||
        requirements.members.keys.any { it == "java/lang/Float" || it == "java/lang/Double" }
    val wantsCldc10 = requirements.configuration.equals("CLDC-1.0", ignoreCase = true) && !needsFloatingPoint
    fun chooseBoot(): ApiLibrary? {
        val requiredCore = (requirements.classes + demands.keys +
            if (needsFloatingPoint) setOf("java/lang/Float", "java/lang/Double") else emptySet()).filter { it in coreOwners }
        val memberOwners = demands.keys.filter { it in coreOwners }
        return bootCandidates.minWithOrNull(compareBy<ApiLibrary>(
            { library -> requiredCore.count { it !in library.classes } },
            { library -> memberOwners.sumOf { missing(library, it) } },
            // Prefer a dedicated CLDC jar when compatible. A proven device-specific
            // core reference can still require the monolithic SDK's definition.
            { if ("javax/microedition/midlet/MIDlet" in it.classes) 1 else 0 },
            { if (("java/lang/Float" !in it.classes) == wantsCldc10) 0 else 1 },
            { it.classes.size },
            { it.path.toString() },
        ))
    }
    var boot = chooseBoot()

    // Do not accidentally reintroduce CLDC 1.1 classes from a later fallback
    // jar after selecting the 1.0 core. Optional packages remain available.
    fun select(owner: String, alternatives: List<ApiLibrary>): ApiLibrary =
        if (owner in coreOwners && boot != null) {
            checkNotNull(boot)
        } else {
            alternatives.minWith(compareBy<ApiLibrary>(
                { missing(it, owner) },
                { if (it.fallback) 1 else 0 },
                { if ("java/lang/Object" in it.classes) 1 else 0 },
                { it.classes.size },
                { it.path.toString() },
            ))
        }
    fun selectAll(): Map<String, ApiLibrary> = providers.filterKeys { it !in coreOwners || it in checkNotNull(boot).classes }
        .mapValues { (owner, alternatives) -> select(owner, alternatives) }
    var selected = selectAll()

    // An inherited call constrains its declaration's provider as well. Otherwise
    // a compatible child could be combined with an older, incompatible parent.
    do {
        var changed = false
        for ((owner, members) in demands.mapValues { it.value.toSet() }) {
            val definition = selected[owner]?.classes?.get(owner) ?: continue
            for (member in members) {
                if (member.name == "<init>" || definition.declares(member)) continue
                val parent = definition.parents.firstOrNull { parent ->
                    providers[parent].orEmpty().any { supports(it, parent, member) }
                } ?: continue
                if (demands.getOrPut(parent) { linkedSetOf() }.add(member)) changed = true
            }
        }
        if (changed) {
            missingCounts.clear()
            boot = chooseBoot()
            selected = selectAll()
        }
    } while (changed)
    return listOf(writeApiSnapshot(selected.toSortedMap(), cacheDir))
}

private fun writeApiSnapshot(selected: Map<String, ApiLibrary>, cacheDir: Path): Path {
    val providerIndex = buildString {
        appendLine("class\tlibrary\tcompile_only")
        for ((owner, library) in selected) appendLine("$owner\t${library.path.fileName}\t${library.fallback}")
    }.toByteArray()
    val digest = MessageDigest.getInstance("SHA-256")
    digest.update("api-snapshot-v3\n".toByteArray())
    digest.update(providerIndex)
    for ((owner, library) in selected) {
        digest.update("$owner\n".toByteArray())
        digest.update(library.classes.getValue(owner).bytes)
    }
    val key = digest.digest().joinToString("") { "%02x".format(it) }
    val snapshot = cacheDir.resolve("$key.jar")
    synchronized(apiSnapshotLock) {
        if (!snapshot.isRegularFile()) {
            cacheDir.createDirectories()
            val temporary = Files.createTempFile(cacheDir, "api-", ".jar.tmp")
            try {
                ZipOutputStream(Files.newOutputStream(temporary)).use { output ->
                    for ((owner, library) in selected) {
                        output.putNextEntry(ZipEntry("$owner.class").apply { time = 0 })
                        output.write(library.classes.getValue(owner).bytes)
                        output.closeEntry()
                    }
                    output.putNextEntry(ZipEntry("META-INF/j2me-api-sources.tsv").apply { time = 0 })
                    output.write(providerIndex)
                    output.closeEntry()
                }
                Files.move(temporary, snapshot, java.nio.file.StandardCopyOption.REPLACE_EXISTING)
            } finally {
                Files.deleteIfExists(temporary)
            }
        }
    }
    return snapshot
}
