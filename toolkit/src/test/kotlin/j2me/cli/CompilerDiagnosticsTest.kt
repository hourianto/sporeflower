package j2me.cli

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe

class CompilerDiagnosticsTest : FunSpec({
    test("modern javac diagnostics parse error files, messages, and warnings") {
        val diagnostics = parseJavacDiagnostics(
            """
            /tmp/project/Game.java:3: error: cannot find symbol
            /tmp/project/Game.java:8: error: incompatible types: int cannot be converted to byte
            /tmp/project/Game.java:12: warning: [unchecked] unchecked conversion
            warning: [options] bootstrap class path not set in conjunction with -source 8
            2 errors
            2 warnings
            """.trimIndent(),
        )

        diagnostics.warningCount shouldBe 2
        diagnostics.errors.map { it.file } shouldBe listOf("/tmp/project/Game.java", "/tmp/project/Game.java")
        diagnostics.errors.map { it.message } shouldBe listOf(
            "cannot find symbol",
            "incompatible types: int cannot be converted to byte",
        )
        diagnostics.errorLines.size shouldBe 2
    }

    test("legacy javac diagnostics parse old-style problem lines") {
        val diagnostics = parseLegacyJavacDiagnostics(
            """
            /tmp/project/Game.java:42: cannot resolve symbol
            symbol  : class MissingThing
            location: class Game
                MissingThing field;
                ^
            /tmp/project/Game.java:77: incompatible types
            found   : boolean
            required: int
            2 errors
            """.trimIndent(),
        )

        diagnostics.warningCount shouldBe 0
        diagnostics.errors.map { it.file } shouldBe listOf("/tmp/project/Game.java", "/tmp/project/Game.java")
        diagnostics.errors.map { it.message } shouldBe listOf("cannot resolve symbol", "incompatible types")
        diagnostics.errorLines.size shouldBe 2
    }

    test("legacy javac diagnostics do not count warning lines as errors") {
        val diagnostics = parseLegacyJavacDiagnostics(
            """
            /tmp/project/Game.java:9: warning: as of release 1.4, assert is a keyword
                int assert = 1;
                    ^
            /tmp/project/Game.java:12: cannot resolve symbol
            1 error
            1 warning
            """.trimIndent(),
        )

        diagnostics.warningCount shouldBe 1
        diagnostics.errors.map { it.message } shouldBe listOf("cannot resolve symbol")
        diagnostics.errorLines.size shouldBe 1
    }

    test("ecj diagnostics parse problem headers") {
        val diagnostics = parseEcjDiagnostics(
            """
            ----------
            1. ERROR in /tmp/project/Game.java (at line 3)
                Sprite sprite;
                ^^^^^^
            Sprite cannot be resolved to a type
            ----------
            2. WARNING in /tmp/project/Game.java (at line 7)
                int unused;
                    ^^^^^^
            The value of the local variable unused is not used
            ----------
            """.trimIndent(),
        )

        diagnostics.warningCount shouldBe 1
        diagnostics.errors.map { it.file } shouldBe listOf("/tmp/project/Game.java")
        diagnostics.errors.map { it.message } shouldBe listOf("Sprite cannot be resolved to a type")
        diagnostics.errorLines shouldBe listOf("1. ERROR in /tmp/project/Game.java (at line 3)")
    }
})
