package j2me.validation

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import io.kotest.matchers.string.shouldNotContain
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.model.MappingOrigin
import j2me.model.isConstructor
import org.objectweb.asm.Opcodes
import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.charset.StandardCharsets
import java.nio.file.Path

class ValidationTest : FunSpec({
    test("unexpected command failures retain their stack trace") {
        val output = captureStandardError {
            printUserFacingError(IndexOutOfBoundsException("broken index"))
        }

        output shouldContain "Command failed:\n- broken index"
        output shouldContain "java.lang.IndexOutOfBoundsException: broken index"
        output shouldContain "at j2me.validation.ValidationTest"
    }

    test("deliberate argument failures remain concise") {
        val output = captureStandardError {
            printUserFacingError(IllegalArgumentException("invalid mapping declaration"))
        }

        output shouldBe "Command failed:\n- invalid mapping declaration\n"
        output shouldNotContain "IllegalArgumentException"
    }

    test("isConstructor only matches JVM constructor name") {
        MethodSig("a", "<init>", "()V").isConstructor() shouldBe true
        MethodSig("a", "a", "()V").isConstructor() shouldBe false
        MethodSig("pkg/A", "A", "()V").isConstructor() shouldBe false
    }

    test("validateMap reports missing method with resolved symbol and raw descriptor") {
        val symbolsByClass = mapOf(
            "af" to ClassSymbols(fields = emptyList(), methods = emptyList(), methodAccess = emptyMap()),
        )
        val cmap = CanonicalMap(
            methods = mapOf(MethodSig("af", "missing", "()V") to "renamed"),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method not found in class af"
        message shouldContain "resolved symbol: void missing() -> renamed"
        message shouldContain "raw descriptor: missing()V"
    }

    test("validateMap reports missing already-mapped ignored class") {
        val cmap = CanonicalMap(ignoredClasses = setOf("game/ui/SettingsScreen"))

        val exc = shouldThrow<MappingValidationException> {
            validateMap(emptyMap(), cmap)
        }

        exc.message.orEmpty() shouldContain "@AlreadyMapped class not found in jar: game/ui/SettingsScreen"
    }

    test("validateMap reports missing method with candidate hints") {
        val cSig = MethodSig("af", "c", "(I)V")
        val dSig = MethodSig("af", "d", "()I")

        val symbolsByClass = mapOf(
            "af" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(cSig, dSig),
                methodAccess = mapOf(cSig to 0, dSig to Opcodes.ACC_STATIC),
            ),
        )

        val cmap = CanonicalMap(
            classes = emptyMap(),
            fields = emptyMap(),
            methods = mapOf(MethodSig("af", "x", "()V") to "renamed"),
            methodArgs = emptyMap(),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method not found in class af"
        message shouldContain "= available method(s) in af (shown 2/2):"
        message shouldContain "=   int d() [raw: d()I]"
        message shouldContain "=   void c(int) [raw: c(I)V]"
        message shouldContain "hint: no methods named 'x' exist in class af"
    }

    test("validateMap missing method with same-name overloads reports closest-arity hint") {
        val cSig = MethodSig("af", "c", "(IJ)I")
        val symbolsByClass = mapOf(
            "af" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(cSig),
                methodAccess = mapOf(cSig to 0),
            ),
        )
        val wanted = MethodSig("af", "c", "(III)V")
        val cmap = CanonicalMap(
            methods = mapOf(wanted to "paint"),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method not found in class af"
        message shouldContain "candidates in af matching name 'c' (shown 1/1):"
        message shouldContain "=   int c(int, long) [raw: c(IJ)I]"
        message shouldContain "hint: signature has 3 parameters but closest match has 2"
    }

    test("validateMap missing field lists available fields with types") {
        val symbolsByClass = mapOf(
            "af" to ClassSymbols(
                fields = listOf(FieldSig("af", "a", "I"), FieldSig("af", "b", "J")),
                methods = emptyList(),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(
            fields = mapOf(FieldSig("af", "x", "I") to "renamed"),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: field not found in class af"
        message shouldContain "= available field(s) in af (shown 2/2):"
        message shouldContain "=   int a [raw: a:I]"
        message shouldContain "=   long b [raw: b:J]"
        message shouldContain "hint: no fields named 'x' exist in class af"
    }

    test("validateMap field collision includes mapping origins and actionable hint") {
        val fieldA = FieldSig("af", "a", "I")
        val fieldB = FieldSig("af", "b", "J")
        val symbolsByClass = mapOf(
            "af" to ClassSymbols(
                fields = listOf(fieldA, fieldB),
                methods = emptyList(),
                methodAccess = emptyMap(),
            ),
        )
        val cmap = CanonicalMap(
            fields = mapOf(fieldA to "value", fieldB to "value"),
            fieldOrigins = mapOf(
                fieldA to MappingOrigin(Path.of("/tmp/mappings/GameEngine.map"), 2, "int value /* was a */;"),
                fieldB to MappingOrigin(Path.of("/tmp/mappings/GameEngine.map"), 3, "long value /* was b */;"),
            ),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: field name collision in class af"
        message shouldContain "GameEngine.map:2"
        message shouldContain "GameEngine.map:3"
        message shouldContain "both a:I and b:J map to 'value'"
        message shouldContain "hint: rename one of the mapped fields to resolve the collision"
    }

    test("validateMap detects collision for non-constructor methods named like class") {
        val classNamedMethod = MethodSig("a", "a", "()V")
        val otherMethod = MethodSig("a", "b", "()V")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(classNamedMethod, otherMethod),
                methodAccess = mapOf(classNamedMethod to 0, otherMethod to 0),
            ),
        )
        val cmap = CanonicalMap(
            methods = mapOf(
                classNamedMethod to "conflictName",
                otherMethod to "conflictName",
            ),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method name collision in class a"
        message shouldContain "both a()V and b()V map to 'conflictName()V'"
    }

    test("validateMap rejects inconsistent names for bytecode override family") {
        val baseBack = MethodSig("f", "b", "()V")
        val childBack = MethodSig("u", "b", "()V")
        val symbolsByClass = mapOf(
            "f" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(baseBack),
                methodAccess = mapOf(baseBack to Opcodes.ACC_PUBLIC),
            ),
            "u" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(childBack),
                methodAccess = mapOf(childBack to Opcodes.ACC_PUBLIC),
                superName = "f",
            ),
        )
        val cmap = CanonicalMap(
            methods = mapOf(
                baseBack to "handleBack",
                childBack to "tick",
            ),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method override family renamed inconsistently in class u"
        message shouldContain "subclass method: u.b()V -> tick"
        message shouldContain "inherited method: f.b()V -> handleBack"
        message shouldContain "hint: use the same target name for every method in the bytecode override family"
    }

    test("validateMap rejects rename that creates source override from unrelated bytecode method") {
        val baseBack = MethodSig("f", "b", "()V")
        val childMenu = MethodSig("u", "g", "()V")
        val symbolsByClass = mapOf(
            "f" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(baseBack),
                methodAccess = mapOf(baseBack to Opcodes.ACC_PUBLIC),
            ),
            "u" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(childMenu),
                methodAccess = mapOf(childMenu to Opcodes.ACC_PUBLIC),
                superName = "f",
            ),
        )
        val cmap = CanonicalMap(
            methods = mapOf(
                baseBack to "handleBack",
                childMenu to "handleBack",
            ),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, cmap)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method rename creates new source override in class u"
        message shouldContain "subclass method: u.g()V -> handleBack"
        message shouldContain "inherited method: f.b()V -> handleBack"
        message shouldContain "hint: choose a distinct target name so unrelated bytecode methods do not become a Java override"
    }

    test("validateMap recognizes covariant return override families") {
        val baseValue = MethodSig("a", "x", "()Ljava/lang/Object;")
        val childValue = MethodSig("b", "x", "()Ljava/lang/String;")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(baseValue),
                methodAccess = mapOf(baseValue to Opcodes.ACC_PUBLIC),
            ),
            "b" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(childValue),
                methodAccess = mapOf(childValue to Opcodes.ACC_PUBLIC),
                superName = "a",
            ),
        )

        validateMap(
            symbolsByClass,
            CanonicalMap(methods = mapOf(baseValue to "getValue", childValue to "getValue")),
        )
        shouldThrow<MappingValidationException> {
            validateMap(
                symbolsByClass,
                CanonicalMap(methods = mapOf(baseValue to "getValue", childValue to "getText")),
            )
        }
    }

    test("validateMap rejects renaming external interface implementation away from required source method") {
        val runnableRun = MethodSig("java/lang/Runnable", "run", "()V")
        val playbackRun = MethodSig("AudioPlayer", "run", "()V")
        val projectSymbolsByClass = mapOf(
            "AudioPlayer" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(playbackRun),
                methodAccess = mapOf(playbackRun to Opcodes.ACC_PUBLIC),
                interfaces = listOf("java/lang/Runnable"),
            ),
        )
        val classpathSymbolsByClass = mapOf(
            "java/lang/Runnable" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(runnableRun),
                methodAccess = mapOf(runnableRun to (Opcodes.ACC_PUBLIC or Opcodes.ACC_ABSTRACT)),
            ),
        )
        val cmap = CanonicalMap(
            methods = mapOf(playbackRun to "runPlaybackThread"),
        )

        val exc = shouldThrow<MappingValidationException> {
            validateMap(projectSymbolsByClass, cmap, classpathSymbolsByClass = classpathSymbolsByClass)
        }

        val message = exc.message.orEmpty()
        message shouldContain "error: method override family renamed inconsistently in class AudioPlayer"
        message shouldContain "subclass method: AudioPlayer.run()V -> runPlaybackThread"
        message shouldContain "inherited method: java/lang/Runnable.run()V -> run"
        message shouldContain "hint: use the same target name for every method in the bytecode override family"
    }

    test("validateMap rejects static method collisions across a class hierarchy") {
        val desc = "(Ljavax/microedition/lcdui/Graphics;IIIII)V"
        val baseFill = MethodSig("i", "b", desc)
        val childFill = MethodSig("e", "a", desc)
        val symbolsByClass = mapOf(
            "i" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(baseFill),
                methodAccess = mapOf(baseFill to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC)),
            ),
            "e" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(childFill),
                methodAccess = mapOf(childFill to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC)),
                superName = "i",
            ),
        )
        val cmap = CanonicalMap(
            classes = mapOf("i" to "BaseGameCanvas", "e" to "MainGame"),
            methods = mapOf(baseFill to "fillRect", childFill to "fillRect"),
        )

        shouldThrow<MappingValidationException> { validateMap(symbolsByClass, cmap) }
    }

    test("validateMap permits distinct static method names across a class hierarchy") {
        val desc = "(Ljavax/microedition/lcdui/Graphics;IIIII)V"
        val baseFill = MethodSig("i", "b", desc)
        val childFill = MethodSig("e", "a", desc)
        val symbolsByClass = mapOf(
            "i" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(baseFill),
                methodAccess = mapOf(baseFill to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC)),
            ),
            "e" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(childFill),
                methodAccess = mapOf(childFill to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC)),
                superName = "i",
            ),
        )

        validateMap(
            symbolsByClass,
            CanonicalMap(methods = mapOf(baseFill to "fillRect", childFill to "fillAlphaRect")),
        )
    }

    test("validateMap rejects return-type-only method collisions") {
        val objectGetter = MethodSig("a", "a", "()Ljava/lang/Object;")
        val stringGetter = MethodSig("a", "b", "()Ljava/lang/String;")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(objectGetter, stringGetter),
                methodAccess = mapOf(objectGetter to 0, stringGetter to 0),
            ),
        )
        val cmap = CanonicalMap(methods = mapOf(objectGetter to "getValue", stringGetter to "getValue"))

        shouldThrow<MappingValidationException> { validateMap(symbolsByClass, cmap) }
    }

    test("validateMap rejects field collisions across a class hierarchy") {
        val baseState = FieldSig("a", "x", "I")
        val childState = FieldSig("b", "y", "I")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(fields = listOf(baseState), methods = emptyList(), methodAccess = emptyMap()),
            "b" to ClassSymbols(
                fields = listOf(childState),
                methods = emptyList(),
                methodAccess = emptyMap(),
                superName = "a",
            ),
        )
        val cmap = CanonicalMap(fields = mapOf(baseState to "state", childState to "state"))

        shouldThrow<MappingValidationException> { validateMap(symbolsByClass, cmap) }
    }

    test("validateMap ignores private ancestor methods that are not source-visible") {
        val privateBase = MethodSig("a", "x", "()V")
        val childMethod = MethodSig("b", "y", "()V")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(privateBase),
                methodAccess = mapOf(privateBase to Opcodes.ACC_PRIVATE),
            ),
            "b" to ClassSymbols(
                fields = emptyList(),
                methods = listOf(childMethod),
                methodAccess = mapOf(childMethod to Opcodes.ACC_PUBLIC),
                superName = "a",
            ),
        )

        validateMap(
            symbolsByClass,
            CanonicalMap(methods = mapOf(privateBase to "helper", childMethod to "helper")),
        )
    }

    test("validateMap ignores private ancestor fields that are not source-visible") {
        val privateBase = FieldSig("a", "x", "I")
        val childField = FieldSig("b", "y", "I")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(
                fields = listOf(privateBase),
                methods = emptyList(),
                fieldAccess = mapOf(privateBase to Opcodes.ACC_PRIVATE),
            ),
            "b" to ClassSymbols(
                fields = listOf(childField),
                methods = emptyList(),
                fieldAccess = mapOf(childField to Opcodes.ACC_PUBLIC),
                superName = "a",
            ),
        )

        validateMap(
            symbolsByClass,
            CanonicalMap(fields = mapOf(privateBase to "state", childField to "state")),
        )
    }

    test("validateMap uses emitted packages for package-private method visibility") {
        val baseMethod = MethodSig("a", "x", "()V")
        val childMethod = MethodSig("b", "y", "()V")
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(emptyList(), listOf(baseMethod), methodAccess = mapOf(baseMethod to 0)),
            "b" to ClassSymbols(
                emptyList(),
                listOf(childMethod),
                methodAccess = mapOf(childMethod to Opcodes.ACC_PUBLIC),
                superName = "a",
            ),
        )
        val distinctPackages = CanonicalMap(
            classes = mapOf("a" to "base/Base", "b" to "child/Child"),
            methods = mapOf(baseMethod to "work", childMethod to "work"),
        )

        validateMap(symbolsByClass, distinctPackages)
        shouldThrow<MappingValidationException> {
            validateMap(symbolsByClass, distinctPackages.copy(classes = mapOf("a" to "game/Base", "b" to "game/Child")))
        }
    }

    test("validateMap detects collisions in the actual emitted default package") {
        val symbolsByClass = mapOf(
            "a" to ClassSymbols(emptyList(), emptyList()),
            "defpackage/a" to ClassSymbols(emptyList(), emptyList()),
        )

        val exc = shouldThrow<MappingValidationException> { validateMap(symbolsByClass, CanonicalMap()) }
        exc.message.orEmpty() shouldContain "class name collision"
        exc.message.orEmpty() shouldContain "defpackage/a"
    }
})

private fun captureStandardError(block: () -> Unit): String {
    val previous = System.err
    val bytes = ByteArrayOutputStream()
    try {
        System.setErr(PrintStream(bytes, true, StandardCharsets.UTF_8))
        block()
    } finally {
        System.setErr(previous)
    }
    return bytes.toString(StandardCharsets.UTF_8)
}
