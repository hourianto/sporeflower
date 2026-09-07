package j2me.cli

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

class FullrunSelectionTest {
    @TempDir lateinit var root: Path

    @Test fun `automatic selection excludes disabled projects before applying limit`() {
        project("a-disabled", "[fullrun]\nenabled = false\n")
        val default = project("b-default")
        val enabled = project("c-enabled", "[fullrun]\nenabled = true\n")
        root.resolve("not-a-project").createDirectories()

        assertEquals(listOf(default, enabled), selectProjects(root, emptyList(), 0))
        assertEquals(listOf(default), selectProjects(root, emptyList(), 1))
    }

    @Test fun `explicit names and paths override exclusion`() {
        val disabled = project("disabled", "[fullrun]\nenabled = false\n")
        for (requested in listOf("disabled", "./disabled", disabled.toString())) {
            assertEquals(listOf(disabled), selectProjects(root, listOf(requested), 0))
        }
        assertEquals(emptyList<Path>(), selectProjects(root, emptyList(), 0))
    }

    @Test fun `malformed settings stay selected for failure reporting`() {
        val syntaxError = project("syntax-error", "[fullrun\n")
        val wrongType = project("wrong-type", "[fullrun]\nenabled = \"false\"\n")
        assertEquals(listOf(syntaxError, wrongType), selectProjects(root, emptyList(), 0))
    }

    private fun project(name: String, extra: String = ""): Path =
        root.resolve(name).createDirectories().also {
            it.resolve("j2me.toml").writeText("jar = \"game.jar\"\n\n$extra")
        }
}
