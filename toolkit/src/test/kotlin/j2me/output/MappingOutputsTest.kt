package j2me.output

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.string.shouldContain
import io.kotest.matchers.string.shouldNotContain
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import kotlin.io.path.readText

class MappingOutputsTest : FunSpec({
    test("writeTinyMapping emits expected local variable slots") {
        val root = Files.createTempDirectory("tiny-out")
        val tinyPath = root.resolve("mapping.tiny")

        val instanceSig = MethodSig("af", "m1", "(IJ)V")
        val staticSig = MethodSig("af", "m2", "(DJ)V")

        val cmap = CanonicalMap(
            classes = emptyMap(),
            fields = emptyMap(),
            methods = mapOf(instanceSig to "m1Renamed", staticSig to "m2Renamed"),
            methodArgs = mapOf(instanceSig to listOf("x", "y"), staticSig to listOf("d", "l")),
        )

        writeTinyMapping(
            tinyPath,
            cmap,
            symbolsByClass = mapOf(
                "af" to ClassSymbols(
                    fields = emptyList(),
                    methods = listOf(instanceSig, staticSig),
                    methodAccess = mapOf(instanceSig to 0, staticSig to Opcodes.ACC_STATIC),
                ),
            ),
            allOwners = listOf("af"),
        )

        val tiny = tinyPath.readText()
        tiny shouldContain "c\taf\tdefpackage/af"
        tiny shouldContain "\tm\t(IJ)V\tm1\tm1Renamed"
        tiny shouldContain "\t\tp\t1\tp0\tx"
        tiny shouldContain "\t\tp\t2\tp1\ty"
        tiny shouldContain "\tm\t(DJ)V\tm2\tm2Renamed"
        tiny shouldContain "\t\tp\t0\tp0\td"
        tiny shouldContain "\t\tp\t2\tp1\tl"
    }

    test("writeTinyMapping omits generated invalid class scopes and members") {
        val root = Files.createTempDirectory("tiny-keyword-out")
        val tinyPath = root.resolve("mapping.tiny")
        val fieldSig = FieldSig("do", "a", "I")
        val methodSig = MethodSig("do", "a", "()V")

        writeTinyMapping(
            tinyPath,
            CanonicalMap(
                fields = mapOf(fieldSig to "counter"),
                methods = mapOf(methodSig to "tick"),
                methodArgs = mapOf(methodSig to emptyList()),
            ),
            symbolsByClass = mapOf(
                "do" to ClassSymbols(emptyList(), listOf(methodSig), methodAccess = mapOf(methodSig to 0)),
            ),
            allOwners = listOf("do"),
        )

        val tiny = tinyPath.readText()
        tiny shouldNotContain "\nc\tdo\t"
        tiny shouldNotContain "\tf\tI\ta\tcounter"
        tiny shouldNotContain "\tm\t()V\ta\ttick"
    }

    test("writeTinyMapping still packages explicit default-package class renames") {
        val root = Files.createTempDirectory("tiny-explicit-class-out")
        val tinyPath = root.resolve("mapping.tiny")

        writeTinyMapping(
            tinyPath,
            CanonicalMap(classes = mapOf("do" to "CityMapRenderState")),
            symbolsByClass = emptyMap(),
            allOwners = listOf("do"),
        )

        val tiny = tinyPath.readText()
        tiny shouldContain "c\tdo\tdefpackage/CityMapRenderState"
    }
})
