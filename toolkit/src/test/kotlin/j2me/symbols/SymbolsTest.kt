package j2me.symbols

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.collections.shouldContainExactly
import io.kotest.matchers.shouldBe
import j2me.model.FieldSig
import j2me.model.MethodSig
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import kotlin.io.path.writeBytes

private fun classBytes(owner: String): ByteArray = ClassWriter(0).apply {
    visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, owner, null, "java/lang/Object", null)
    visitEnd()
}.toByteArray()

class SymbolsTest : FunSpec({
    test("call instruction byte offsets survive symbol caching") {
        val jar = Files.createTempFile("call-offsets", ".jar")
        val caller = MethodSig("Caller", "decode", "()I")
        val callee = MethodSig("Reader", "read", "(I)I")
        val bytes = ClassWriter(0).apply {
            visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, caller.owner, null, "java/lang/Object", null)
            visitMethod(Opcodes.ACC_STATIC, caller.name, caller.desc, null, null).apply {
                visitCode()
                visitIntInsn(Opcodes.SIPUSH, 300) // Three bytes, unlike an instruction ordinal.
                visitMethodInsn(Opcodes.INVOKESTATIC, callee.owner, callee.name, callee.desc, false)
                visitMethodInsn(Opcodes.INVOKESTATIC, callee.owner, callee.name, callee.desc, false)
                visitInsn(Opcodes.IRETURN)
                visitMaxs(1, 0)
                visitEnd()
            }
            visitEnd()
        }.toByteArray()
        ZipOutputStream(jar.toFile().outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("Caller.class"))
            zip.write(bytes)
            zip.closeEntry()
        }
        val symbols = parseClassSymbols(jar, "Caller")
        symbols.methodCalls shouldBe mapOf(caller to mapOf(3 to callee, 6 to callee))
        val cache = Files.createTempFile("call-symbol-cache", ".json")
        writeSymbolCache(cache, jar, listOf("Caller"), mapOf("Caller" to symbols))
        loadSymbolCache(cache, jar)?.second?.get("Caller") shouldBe symbols
    }

    test("readClassBytesByOwner reads class entries from jar") {
        val jarPath = Files.createTempFile("fixture", ".jar")
        ZipOutputStream(jarPath.toFile().outputStream()).use { zos ->
            zos.putNextEntry(ZipEntry("META-INF/MANIFEST.MF"))
            zos.write("Manifest-Version: 1.0\n".toByteArray())
            zos.closeEntry()

            zos.putNextEntry(ZipEntry("a.class"))
            zos.write(classBytes("a"))
            zos.closeEntry()

            zos.putNextEntry(ZipEntry("pkg/B.class"))
            zos.write(classBytes("pkg/B"))
            zos.closeEntry()

            zos.putNextEntry(ZipEntry("main.class"))
            zos.write(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A))
            zos.closeEntry()

            zos.putNextEntry(ZipEntry("assets/image.png"))
            zos.write(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47))
            zos.closeEntry()
        }

        readClassBytesByOwner(jarPath).keys.sorted() shouldContainExactly listOf("a", "pkg/B")
        readClassBytesByOwner(jarPath, listOf("main")) shouldBe emptyMap()
    }

    test("readClassBytesByOwner trusts class identity rather than a misleading zip path") {
        val jarPath = Files.createTempFile("misnamed-class", ".jar")
        ZipOutputStream(jarPath.toFile().outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("wrong/Path.class"))
            zip.write(classBytes("actual/Owner"))
            zip.closeEntry()
        }

        readClassBytesByOwner(jarPath).keys shouldBe setOf("actual/Owner")
        readClassBytesByOwner(jarPath, listOf("actual/Owner")).keys shouldBe setOf("actual/Owner")
    }

    test("parseClassSymbols preserves field and method access flags") {
        val jarPath = Files.createTempFile("symbol-access", ".jar")
        val owner = "AccessSample"
        val field = FieldSig(owner, "secret", "I")
        val method = MethodSig(owner, "utility", "()V")
        val writer = ClassWriter(0).apply {
            visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, owner, null, "java/lang/Object", null)
            visitField(Opcodes.ACC_PRIVATE or Opcodes.ACC_STATIC, field.name, field.desc, null, null).visitEnd()
            visitMethod(Opcodes.ACC_PROTECTED or Opcodes.ACC_STATIC, method.name, method.desc, null, null).visitEnd()
            visitEnd()
        }
        ZipOutputStream(jarPath.toFile().outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("$owner.class"))
            zip.write(writer.toByteArray())
            zip.closeEntry()
        }

        val symbols = parseClassSymbols(jarPath, owner)
        symbols.fieldAccess[field] shouldBe (Opcodes.ACC_PRIVATE or Opcodes.ACC_STATIC)
        symbols.methodAccess[method] shouldBe (Opcodes.ACC_PROTECTED or Opcodes.ACC_STATIC)
        symbols.isMethodStatic(method) shouldBe true
    }

    test("readClassBytesByOwner fails for invalid archive") {
        val bad = Files.createTempFile("bad", ".jar")
        bad.writeBytes("not-a-zip".toByteArray())

        val exc = shouldThrow<IllegalArgumentException> {
            readClassBytesByOwner(bad)
        }
        exc.message.orEmpty().contains("Failed to read JAR as zip") shouldBe true
    }

    test("collectSymbolUsage counts project method and field instructions") {
        val targetWriter = ClassWriter(0)
        targetWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Target", null, "java/lang/Object", null)
        targetWriter.visitEnd()

        val callerWriter = ClassWriter(0)
        callerWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Caller", null, "java/lang/Object", null)
        callerWriter.visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, "run", "()V", null, null).apply {
            visitCode()
            visitMethodInsn(Opcodes.INVOKESTATIC, "Target", "ping", "()V", false)
            visitFieldInsn(Opcodes.GETSTATIC, "Target", "value", "I")
            visitInsn(Opcodes.POP)
            visitInsn(Opcodes.ICONST_1)
            visitFieldInsn(Opcodes.PUTSTATIC, "Target", "value", "I")
            visitMethodInsn(Opcodes.INVOKESTATIC, "External", "ignored", "()V", false)
            visitInsn(Opcodes.RETURN)
            visitMaxs(1, 0)
            visitEnd()
        }
        callerWriter.visitEnd()

        val usage = collectSymbolUsage(
            classBytesByOwner = mapOf(
                "Caller" to callerWriter.toByteArray(),
                "Target" to targetWriter.toByteArray(),
            ),
            classes = listOf("Caller", "Target"),
            workers = 2,
        )
        val method = MethodSig("Target", "ping", "()V")
        val field = FieldSig("Target", "value", "I")

        usage.methodRefs[method] shouldBe 1
        usage.methodCallers[method] shouldBe setOf("Caller.run()V")
        usage.fieldReads[field] shouldBe 1
        usage.fieldWrites[field] shouldBe 1
        usage.fieldAccessors[field] shouldBe setOf("Caller.run()V")
        usage.methodRefs.keys.none { it.owner == "External" } shouldBe true
    }

    test("collectSymbolUsage counts distinct caller methods") {
        val targetWriter = ClassWriter(0)
        targetWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Target", null, "java/lang/Object", null)
        targetWriter.visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, "ping", "()V", null, null).visitEnd()
        targetWriter.visitEnd()

        val callerWriter = ClassWriter(0)
        callerWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Caller", null, "java/lang/Object", null)
        for (name in listOf("first", "second")) {
            callerWriter.visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, name, "()V", null, null).apply {
                visitMethodInsn(Opcodes.INVOKESTATIC, "Target", "ping", "()V", false)
                visitInsn(Opcodes.RETURN)
                visitMaxs(0, 0)
                visitEnd()
            }
        }
        callerWriter.visitEnd()

        val usage = collectSymbolUsage(
            classBytesByOwner = mapOf("Caller" to callerWriter.toByteArray(), "Target" to targetWriter.toByteArray()),
            classes = listOf("Caller", "Target"),
            workers = 2,
        )
        val method = MethodSig("Target", "ping", "()V")

        usage.methodRefs[method] shouldBe 2
        usage.methodCallers[method] shouldBe setOf("Caller.first()V", "Caller.second()V")
    }

    test("collectSymbolUsage resolves references through an internal superclass") {
        val parentWriter = ClassWriter(0)
        parentWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Parent", null, "java/lang/Object", null)
        parentWriter.visitField(Opcodes.ACC_PUBLIC, "value", "I", null, null).visitEnd()
        parentWriter.visitMethod(Opcodes.ACC_PUBLIC, "ping", "()V", null, null).visitEnd()
        parentWriter.visitEnd()

        val childWriter = ClassWriter(0)
        childWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Child", null, "Parent", null)
        childWriter.visitEnd()

        val callerWriter = ClassWriter(0)
        callerWriter.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "Caller", null, "java/lang/Object", null)
        callerWriter.visitMethod(Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC, "run", "(LChild;)V", null, null).apply {
            visitVarInsn(Opcodes.ALOAD, 0)
            visitMethodInsn(Opcodes.INVOKEVIRTUAL, "Child", "ping", "()V", false)
            visitVarInsn(Opcodes.ALOAD, 0)
            visitFieldInsn(Opcodes.GETFIELD, "Child", "value", "I")
            visitInsn(Opcodes.POP)
            visitInsn(Opcodes.RETURN)
            visitMaxs(1, 1)
            visitEnd()
        }
        callerWriter.visitEnd()

        val usage = collectSymbolUsage(
            classBytesByOwner = mapOf(
                "Caller" to callerWriter.toByteArray(),
                "Child" to childWriter.toByteArray(),
                "Parent" to parentWriter.toByteArray(),
            ),
            classes = listOf("Caller", "Child", "Parent"),
            workers = 2,
        )

        usage.methodRefs[MethodSig("Parent", "ping", "()V")] shouldBe 1
        usage.methodRefs[MethodSig("Child", "ping", "()V")] shouldBe null
        usage.fieldReads[FieldSig("Parent", "value", "I")] shouldBe 1
        usage.fieldReads[FieldSig("Child", "value", "I")] shouldBe null
    }
})
