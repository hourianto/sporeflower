package j2me.cli

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import org.tomlj.Toml
import java.nio.file.Path

class ToolkitInstallationTest {
    @TempDir lateinit var temporary: Path

    @Test fun `base override preserves the bundled engine and explicit config path`() {
        val home = temporary.resolve("relocated installation")
        val base = temporary.resolve("custom assets")
        val config = temporary.resolve("settings/local.toml")
        val paths = toolkitPaths(home, base.toString(), config.toString())
        assertEquals(base, paths.base)
        assertEquals(config, paths.globalCfg)
        assertEquals(base.resolve("templates/mappings-doc.md"), paths.mappingsDocTemplate)
        assertEquals(home.resolve("decompiler/sporeflower.jar"), paths.bundledDecompiler)
    }

    @Test fun `relative jar overrides resolve next to the configuration`() {
        val paths = toolkitPaths(temporary, null, null)
        val config = Toml.parse("[vineflower]\nbin = \"../engines/alternate.jar\"\n")
        assertEquals(temporary.resolve("engines/alternate.jar").toString(), configuredDecompiler(paths, config))
        assertEquals(paths.bundledDecompiler.toString(), configuredDecompiler(paths, null))
        assertEquals("custom-decompiler", configuredDecompiler(paths, Toml.parse("[vineflower]\nbin = \"custom-decompiler\"\n")))
    }
}
