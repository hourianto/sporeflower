package j2me.bytecode

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.collections.shouldContain
import io.kotest.matchers.shouldBe
import io.kotest.matchers.shouldNotBe
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes
import org.objectweb.asm.tree.ClassNode
import org.objectweb.asm.tree.FieldInsnNode
import org.objectweb.asm.tree.MethodInsnNode
import java.nio.file.Files
import java.util.zip.ZipEntry
import java.util.zip.ZipFile
import java.util.zip.ZipOutputStream

class BytecodeJarRemapperTest : FunSpec({
    test("remapJarBytecode writes renamed classes members and resources") {
        val root = Files.createTempDirectory("bytecode-remap")
        val inputJar = root.resolve("game.jar")
        val outputJar = defaultRemappedJarPath(inputJar, root.resolve("out"))
        val field = FieldSig("a", "a", "I")
        val method = MethodSig("a", "a", "()I")

        writeInputJar(inputJar)

        val stats = remapJarBytecode(
            inputJar = inputJar,
            outputJar = outputJar,
            mappings = CanonicalMap(
                classes = mapOf("a" to "Foo"),
                fields = mapOf(field to "value"),
                methods = mapOf(method to "getValue"),
            ),
            symbolsByClass = mapOf("a" to ClassSymbols(listOf(field), listOf(method))),
        )

        stats.path shouldBe outputJar
        stats.classCount shouldBe 1
        stats.resourceCount shouldBe 2
        outputJar.fileName.toString() shouldBe "game_remapped.jar"

        ZipFile(outputJar.toFile()).use { zip ->
            zip.getEntry("a.class") shouldBe null
            zip.getEntry("defpackage/Foo.class") shouldNotBe null
            zip.getEntry("assets/data.txt") shouldNotBe null
            zip.getInputStream(zip.getEntry("assets/data.txt")).use { it.readBytes().decodeToString() } shouldBe "hello"
            zip.getInputStream(zip.getEntry("main.class")).use { input ->
                input.readBytes() shouldBe pngHeaderBytes()
            }

            val node = ClassNode()
            zip.getInputStream(zip.getEntry("defpackage/Foo.class")).use { input ->
                ClassReader(input.readBytes()).accept(node, 0)
            }
            node.name shouldBe "defpackage/Foo"
            node.fields.map { it.name } shouldContain "value"
            node.methods.map { it.name } shouldContain "getValue"
        }
    }

    test("remapJarBytecode renames references whose owner inherits the declaration") {
        val root = Files.createTempDirectory("bytecode-inherited-reference")
        val inputJar = root.resolve("game.jar")
        val outputJar = root.resolve("out.jar")
        val field = FieldSig("Parent", "f", "I")
        val method = MethodSig("Parent", "m", "()V")

        ZipOutputStream(Files.newOutputStream(inputJar)).use { zip ->
            for ((owner, bytes) in mapOf(
                "Parent" to parentClassBytes(),
                "Child" to childClassBytes(),
            )) {
                zip.putNextEntry(ZipEntry("$owner.class"))
                zip.write(bytes)
                zip.closeEntry()
            }
        }

        remapJarBytecode(
            inputJar,
            outputJar,
            CanonicalMap(fields = mapOf(field to "value"), methods = mapOf(method to "tick")),
            mapOf(
                "Parent" to ClassSymbols(listOf(field), listOf(method)),
                "Child" to ClassSymbols(emptyList(), emptyList(), superName = "Parent"),
            ),
        )

        ZipFile(outputJar.toFile()).use { zip ->
            val node = ClassNode()
            zip.getInputStream(zip.getEntry("defpackage/Child.class")).use { ClassReader(it.readBytes()).accept(node, 0) }
            val instructions = node.methods.single { it.name == "useInherited" }.instructions.toArray()
            instructions.filterIsInstance<MethodInsnNode>().single().name shouldBe "tick"
            instructions.filterIsInstance<FieldInsnNode>().single().name shouldBe "value"
        }
    }
})

private fun writeInputJar(path: java.nio.file.Path) {
    ZipOutputStream(Files.newOutputStream(path)).use { zip ->
        zip.putNextEntry(ZipEntry("a.class"))
        zip.write(inputClassBytes())
        zip.closeEntry()

        zip.putNextEntry(ZipEntry("assets/data.txt"))
        zip.write("hello".encodeToByteArray())
        zip.closeEntry()

        zip.putNextEntry(ZipEntry("main.class"))
        zip.write(pngHeaderBytes())
        zip.closeEntry()
    }
}

private fun pngHeaderBytes(): ByteArray =
    byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

private fun inputClassBytes(): ByteArray {
    val writer = ClassWriter(0)
    writer.visit(Opcodes.V1_2, Opcodes.ACC_PUBLIC, "a", null, "java/lang/Object", null)
    writer.visitField(Opcodes.ACC_PUBLIC, "a", "I", null, null).visitEnd()

    writer.visitMethod(Opcodes.ACC_PUBLIC, "<init>", "()V", null, null).also { method ->
        method.visitCode()
        method.visitVarInsn(Opcodes.ALOAD, 0)
        method.visitMethodInsn(Opcodes.INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false)
        method.visitInsn(Opcodes.RETURN)
        method.visitMaxs(1, 1)
        method.visitEnd()
    }

    writer.visitMethod(Opcodes.ACC_PUBLIC, "a", "()I", null, null).also { method ->
        method.visitCode()
        method.visitVarInsn(Opcodes.ALOAD, 0)
        method.visitFieldInsn(Opcodes.GETFIELD, "a", "a", "I")
        method.visitInsn(Opcodes.IRETURN)
        method.visitMaxs(1, 1)
        method.visitEnd()
    }

    writer.visitEnd()
    return writer.toByteArray()
}

private fun parentClassBytes(): ByteArray = ClassWriter(0).apply {
    visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Parent", null, "java/lang/Object", null)
    visitField(Opcodes.ACC_PUBLIC, "f", "I", null, null).visitEnd()
    visitMethod(Opcodes.ACC_PUBLIC, "m", "()V", null, null).visitEnd()
    visitEnd()
}.toByteArray()

private fun childClassBytes(): ByteArray = ClassWriter(0).apply {
    visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Child", null, "Parent", null)
    visitMethod(Opcodes.ACC_PUBLIC, "useInherited", "()V", null, null).apply {
        visitVarInsn(Opcodes.ALOAD, 0)
        visitMethodInsn(Opcodes.INVOKEVIRTUAL, "Child", "m", "()V", false)
        visitVarInsn(Opcodes.ALOAD, 0)
        visitFieldInsn(Opcodes.GETFIELD, "Child", "f", "I")
        visitInsn(Opcodes.POP)
        visitInsn(Opcodes.RETURN)
        visitMaxs(1, 1)
        visitEnd()
    }
    visitEnd()
}.toByteArray()
