package j2me.map

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.model.CanonicalMap
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.validation.MappingValidationException
import java.nio.file.Files
import kotlin.io.path.createDirectories
import kotlin.io.path.writeText

class MapParserTest : FunSpec({
    test("legacy classes.map is rejected instead of silently ignored") {
        val root = Files.createTempDirectory("legacy-classes-map")
        val mapsDir = root.resolve("mappings")
        mapsDir.createDirectories()
        mapsDir.resolve("classes.map").writeText("class GameEngine /* was af */;\n")

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(mapsDir, setOf("af"))
        }

        exc.message.orEmpty() shouldContain "Unsupported legacy mapping file"
        exc.message.orEmpty() shouldContain "class ReadableName /* was obfuscated/Owner */"
    }

    test("loadJavaLikeMap parses members and descriptors via JavaParser") {
        val cmap = loadFixture(
            "map-load",
            setOf("af", "x"),
            "GameEngine.map" to """
            class GameEngine /* was af */ {
                int counter /* was a */;
                int tick(int frame, long delta) /* was c */;
                static int helper(int value) /* was d */;
            }
            """,
            "GameSceneManager.map" to """
            class GameSceneManager /* was x */ {
                GameEngine engine /* was j */;
                GameEngine getEngine() /* was m */;
            }
            """,
        )

        cmap.classes shouldBe mapOf("af" to "GameEngine", "x" to "GameSceneManager")
        cmap.fields[FieldSig("af", "a", "I")] shouldBe "counter"
        cmap.fields[FieldSig("x", "j", "Laf;")] shouldBe "engine"
        val cSig = MethodSig("af", "c", "(IJ)I")
        cmap.methods[cSig] shouldBe "tick"
        cmap.methodArgs[cSig] shouldBe listOf("frame", "delta")
    }

    test("inline class owner comment is required") {
        val root = Files.createTempDirectory("map-inline-required")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameEngine.map").writeText(
            """
            class GameEngine {
                int counter /* was a */;
            }
            """.trimIndent() + "\n",
        )

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(mapsDir, setOf("af"))
        }
        exc.message.orEmpty().contains("missing class owner mapping") shouldBe true
    }

    test("package declaration scopes readable owner name") {
        val root = Files.createTempDirectory("map-package-decl")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameCanvas.map").writeText(
            """
            package game;
            class GameCanvas /* was af */ {
                int counter /* was a */;
            }
            """.trimIndent() + "\n",
        )

        val cmap = loadJavaLikeMap(mapsDir, setOf("af"))
        cmap.classes["af"] shouldBe "game/GameCanvas"
        cmap.fields[FieldSig("af", "a", "I")] shouldBe "counter"
    }

    test("package declaration resolves simple original owner relative to package when needed") {
        val root = Files.createTempDirectory("map-package-obf-owner")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("SampleServerMidlet.map").writeText(
            """
            package sample.network;
            class SampleServerMidlet /* was server */ {
            }
            """.trimIndent() + "\n",
        )

        val owner = "sample/network/server"
        val cmap = loadJavaLikeMap(mapsDir, setOf(owner))
        cmap.classes[owner] shouldBe "sample/network/SampleServerMidlet"
    }

    test("loadJavaLikeMap marks already-mapped classes as ignored without remapping them") {
        val owner = "game/ui/SettingsScreen"
        val cmap = loadFixture(
            "map-already-mapped-class",
            setOf(owner, "h"),
            "SettingsScreen.map" to """
            package game.ui;

            @AlreadyMapped
            class SettingsScreen {
            }
            """,
            "Holder.map" to """
            package game.ui;

            class Holder /* was h */ {
                SettingsScreen settings /* was s */;
            }
            """,
        )

        cmap.ignoredClasses shouldBe setOf(owner)
        cmap.classes.containsKey(owner) shouldBe false
        cmap.fields[FieldSig("h", "s", "Lgame/ui/SettingsScreen;")] shouldBe "settings"
    }

    test("loadJavaLikeMap rejects member declarations inside already-mapped classes") {
        val mapsDir = writeFixtureMaps(
            "map-already-mapped-with-members",
            "SettingsScreen.map" to """
            package game.ui;

            @AlreadyMapped
            class SettingsScreen {
                int selectedIndex;
            }
            """,
        )

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(mapsDir, setOf("game/ui/SettingsScreen"))
        }

        exc.message.orEmpty() shouldContain "@AlreadyMapped classes must not declare member mappings"
    }

    test("class owner comment accepts fully qualified original owner") {
        val root = Files.createTempDirectory("map-fq-obf-owner")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("SampleServerMidlet.map").writeText(
            """
            package sample.network;
            class SampleServerMidlet /* was sample.network.server */ {
            }
            """.trimIndent() + "\n",
        )

        val owner = "sample/network/server"
        val cmap = loadJavaLikeMap(mapsDir, setOf(owner))
        cmap.classes[owner] shouldBe "sample/network/SampleServerMidlet"
    }

    test("class owner comment accepts original owner that is a Java keyword") {
        val root = Files.createTempDirectory("map-keyword-obf-owner")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("CityMapRenderState.map").writeText(
            """
            class CityMapRenderState /* was do */ {
            }
            """.trimIndent() + "\n",
        )

        val cmap = loadJavaLikeMap(mapsDir, setOf("do"))
        cmap.classes["do"] shouldBe "CityMapRenderState"
    }

    test("conflicting duplicate method mapping raises validation exception") {
        val root = Files.createTempDirectory("map-dupe")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameEngine.map").writeText(
            """
            class GameEngine /* was af */ {
                int tick(int frame, long delta) /* was c */;
                int render(int frame, long delta) /* was c */;
            }
            """.trimIndent() + "\n",
        )

        val exc = shouldThrow<Exception> {
            loadJavaLikeMap(mapsDir, setOf("af"))
        }
        exc.message.orEmpty().contains("duplicate method mapping with conflicting target") shouldBe true
    }

    test("loadJavaLikeMap accepts Java-style comments around declarations") {
        val root = Files.createTempDirectory("map-comments")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameEngine.map").writeText(
            """
            /* Header comment spanning lines. */
            class GameEngine /* was af */ { // owner block
                // field mapping
                int counter /* was a */; /* trailing */
                int tick(int frame, long delta) /* was c */; // method mapping
            }
            """.trimIndent() + "\n",
        )

        val cmap = loadJavaLikeMap(mapsDir, setOf("af"))

        cmap.fields[FieldSig("af", "a", "I")] shouldBe "counter"
        cmap.methods[MethodSig("af", "c", "(IJ)I")] shouldBe "tick"
    }

    test("loadJavaLikeMap resolves simple java.lang types") {
        val cmap = loadFixture(
            "map-java-lang",
            setOf("af"),
            "GameEngine.map" to """
            class GameEngine /* was af */ {
                String title /* was a */;
                void setTitle(String value) /* was b */;
                void fail(Error error) /* was c */;
            }
            """,
        )

        cmap.fields[FieldSig("af", "a", "Ljava/lang/String;")] shouldBe "title"
        cmap.methods[MethodSig("af", "b", "(Ljava/lang/String;)V")] shouldBe "setTitle"
        cmap.methods[MethodSig("af", "c", "(Ljava/lang/Error;)V")] shouldBe "fail"
    }

    test("loadJavaLikeMap resolves same-package mapped simple names") {
        val cmap = loadFixture(
            "map-same-package-simple",
            setOf("a", "b"),
            "Sprite.map" to """
            package game;
            class Sprite /* was a */ {
            }
            """,
            "Holder.map" to """
            package game;
            class Holder /* was b */ {
                Sprite sprite /* was s */;
                void setSprite(Sprite value) /* was m */;
            }
            """,
        )

        cmap.fields[FieldSig("b", "s", "La;")] shouldBe "sprite"
        cmap.methods[MethodSig("b", "m", "(La;)V")] shouldBe "setSprite"
    }

    test("loadJavaLikeMap resolves unique mapped project simple names across packages") {
        val cmap = loadFixture(
            "map-unique-project-simple",
            setOf("com/appon/util/a", "b"),
            "AssetHandle.map" to """
            package com.appon.util;
            class AssetHandle /* was com/appon/util/a */ {
            }
            """,
            "Globals.map" to """
            package com.appon.prosketch;
            class Globals /* was b */ {
                AssetHandle backgroundAsset /* was c */;
            }
            """,
        )

        cmap.fields[FieldSig("b", "c", "Lcom/appon/util/a;")] shouldBe "backgroundAsset"
    }

    test("loadJavaLikeMap resolves explicit imports for external simple names") {
        val cmap = loadFixture(
            "map-explicit-import",
            setOf("b"),
            setOf("java/util/Hashtable"),
            "Holder.map" to """
            package game;
            import java.util.Hashtable;
            class Holder /* was b */ {
                Hashtable table /* was t */;
            }
            """,
        )

        cmap.fields[FieldSig("b", "t", "Ljava/util/Hashtable;")] shouldBe "table"
    }

    test("loadJavaLikeMap resolves wildcard imports against known classpath classes") {
        val cmap = loadFixture(
            "map-wildcard-import-api",
            setOf("b"),
            setOf(
                "javax/microedition/lcdui/Canvas",
                "javax/microedition/lcdui/Graphics",
            ),
            "CanvasHolder.map" to """
            package game;
            import javax.microedition.lcdui.*;
            class CanvasHolder /* was b */ {
                Canvas canvas /* was c */;
                void paint(Graphics graphics) /* was p */;
            }
            """,
        )

        cmap.fields[FieldSig("b", "c", "Ljavax/microedition/lcdui/Canvas;")] shouldBe "canvas"
        cmap.methods[MethodSig("b", "p", "(Ljavax/microedition/lcdui/Graphics;)V")] shouldBe "paint"
    }

    test("loadJavaLikeMap resolves wildcard imports against mapped project packages") {
        val cmap = loadFixture(
            "map-wildcard-import-project",
            setOf("com/appon/util/a", "b"),
            "AssetHandle.map" to """
            package com.appon.util;
            class AssetHandle /* was com/appon/util/a */ {
            }
            """,
            "Globals.map" to """
            package com.appon.prosketch;
            import com.appon.util.*;
            class Globals /* was b */ {
                AssetHandle backgroundAsset /* was c */;
            }
            """,
        )

        cmap.fields[FieldSig("b", "c", "Lcom/appon/util/a;")] shouldBe "backgroundAsset"
    }

    test("loadJavaLikeMap rejects ambiguous wildcard imports") {
        val mapsDir = writeFixtureMaps(
            "map-ambiguous-wildcard-import",
            "Holder.map" to """
            package game;
            import one.*;
            import two.*;
            class Holder /* was h */ {
                Asset asset /* was a */;
            }
            """,
        )

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(
                mapsDir,
                knownProjectClasses = setOf("h"),
                knownClasspathClasses = setOf("one/Asset", "two/Asset"),
            )
        }

        exc.message.orEmpty() shouldContain "ambiguous class type 'Asset'"
        exc.message.orEmpty() shouldContain "one.Asset"
        exc.message.orEmpty() shouldContain "two.Asset"
    }

    test("loadJavaLikeMap lets explicit imports disambiguate wildcard imports") {
        val cmap = loadFixture(
            "map-explicit-over-wildcard",
            setOf("s", "f"),
            setOf("javax/microedition/lcdui/Screen"),
            "Screen.map" to """
            package com.appon.framework;
            class Screen /* was s */ {
            }
            """,
            "Factory.map" to """
            package game;
            import javax.microedition.lcdui.*;
            import com.appon.framework.*;
            import com.appon.framework.Screen;
            class Factory /* was f */ {
                Screen create() /* was c */;
            }
            """,
        )

        cmap.methods[MethodSig("f", "c", "()Ls;")] shouldBe "create"
    }

    test("loadJavaLikeMap resolves dotted inner class names against known classpath classes") {
        val cmap = loadFixture(
            "map-dotted-inner-class",
            setOf("h"),
            setOf("java/util/Map${'$'}Entry"),
            "Holder.map" to """
            class Holder /* was h */ {
                java.util.Map.Entry entry /* was e */;
            }
            """,
        )

        cmap.fields[FieldSig("h", "e", "Ljava/util/Map${'$'}Entry;")] shouldBe "entry"
    }

    test("loadJavaLikeMap rejects unknown dotted class names") {
        val mapsDir = writeFixtureMaps(
            "map-unknown-dotted-type",
            "Holder.map" to """
            class Holder /* was h */ {
                foo.bar.Baz value /* was v */;
            }
            """,
        )

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(mapsDir, knownProjectClasses = setOf("h"))
        }

        exc.message.orEmpty() shouldContain "unknown class type 'foo.bar.Baz'"
    }

    test("loadJavaLikeMap rejects ambiguous mapped project simple names") {
        val mapsDir = writeFixtureMaps(
            "map-ambiguous-project-simple",
            "FirstAsset.map" to """
            package one;
            class Asset /* was a */ {
            }
            """,
            "SecondAsset.map" to """
            package two;
            class Asset /* was b */ {
            }
            """,
            "Holder.map" to """
            package game;
            class Holder /* was h */ {
                Asset asset /* was a */;
            }
            """,
        )

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(mapsDir, setOf("a", "b", "h"))
        }

        exc.message.orEmpty() shouldContain "ambiguous class type 'Asset'"
        exc.message.orEmpty() shouldContain "one.Asset"
        exc.message.orEmpty() shouldContain "two.Asset"
    }

    test("loadJavaLikeMap accepts mixed defpackage owner spellings through aliasing") {
        val cmap = loadFixture(
            "map-defpackage-alias",
            setOf("af", "x"),
            "GameEngine.map" to """
            package defpackage;
            class GameEngine /* was af */ {
                int counter /* was a */;
            }
            """,
            "Holder.map" to """
            class Holder /* was x */ {
                GameEngine engine /* was e */;
                defpackage.GameEngine engine2 /* was f */;
            }
            """,
        )

        cmap.fields[FieldSig("x", "e", "Laf;")] shouldBe "engine"
        cmap.fields[FieldSig("x", "f", "Laf;")] shouldBe "engine2"
    }

    test("loadJavaLikeMap supports multiple top-level classes in a single .map file") {
        val cmap = loadFixture(
            "map-multi-class",
            setOf("af", "x"),
            "Combined.map" to """
            class GameEngine /* was af */ {
                int counter /* was a */;
            }

            class Holder /* was x */ {
                GameEngine engine /* was e */;
                int tick() /* was c */;
            }
            """,
        )

        cmap.classes["af"] shouldBe "GameEngine"
        cmap.classes["x"] shouldBe "Holder"
        cmap.fields[FieldSig("af", "a", "I")] shouldBe "counter"
        cmap.fields[FieldSig("x", "e", "Laf;")] shouldBe "engine"
        cmap.methods[MethodSig("x", "c", "()I")] shouldBe "tick"
    }

    test("duplicate method mapping includes existing/new target and origin lines") {
        val root = Files.createTempDirectory("map-dupe-context")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameEngine.map").writeText(
            """
            class GameEngine /* was af */ {
                int tick(int frame, long delta) /* was c */;
                int render(int frame, long delta) /* was c */;
            }
            """.trimIndent() + "\n",
        )

        val exc = shouldThrow<MappingValidationException> {
            loadJavaLikeMap(mapsDir, setOf("af"))
        }
        val message = exc.message.orEmpty()
        message shouldContain "duplicate method mapping with conflicting target for af.c(IJ)I"
        message shouldContain "existing target: tick"
        message shouldContain "new target: render"
        message shouldContain "GameEngine.map:2"
        message shouldContain "GameEngine.map:3"
    }

    test("duplicate method parameter conflict includes existing and new origins") {
        val root = Files.createTempDirectory("map-dupe-param-context")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameEngine.map").writeText(
            """
            class GameEngine /* was af */ {
                int tick(int frame, long delta) /* was c */;
                int tick(int f, long d) /* was c */;
            }
            """.trimIndent() + "\n",
        )

        val exc = shouldThrow<MappingValidationException> {
            loadJavaLikeMap(mapsDir, setOf("af"))
        }
        val message = exc.message.orEmpty()
        message shouldContain "duplicate method mapping with conflicting parameter names for af.c(IJ)I"
        message shouldContain "existing parameter names: [frame, delta]"
        message shouldContain "new parameter names: [f, d]"
        message shouldContain "GameEngine.map:2"
        message shouldContain "GameEngine.map:3"
    }

    test("parse errors include line, column, source line, and caret") {
        val root = Files.createTempDirectory("map-parse-error")
        val mapsDir = root.resolve("mappings").createDirectories()

        mapsDir.resolve("GameMIDlet.map").writeText(
            """
            class GameMIDlet /* was b */ {
              void destroyApp(boolean) /* was destroyApp */;
            }
            """.trimIndent() + "\n",
        )

        val exc = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMap(mapsDir, setOf("b"))
        }
        val message = exc.message.orEmpty()
        message shouldContain "GameMIDlet.map: failed to parse map file as Java syntax"
        message shouldContain "2:"
        message shouldContain "void destroyApp(boolean) /* was destroyApp */;"
        message shouldContain "^"
    }
})

private fun loadFixture(
    name: String,
    knownProjectClasses: Set<String>,
    vararg maps: Pair<String, String>,
): CanonicalMap = loadFixture(name, knownProjectClasses, emptySet(), *maps)

private fun loadFixture(
    name: String,
    knownProjectClasses: Set<String>,
    knownClasspathClasses: Set<String>,
    vararg maps: Pair<String, String>,
): CanonicalMap =
    loadJavaLikeMap(
        writeFixtureMaps(name, *maps),
        knownProjectClasses = knownProjectClasses,
        knownClasspathClasses = knownClasspathClasses,
    )

private fun writeFixtureMaps(name: String, vararg maps: Pair<String, String>): java.nio.file.Path {
    val mapsDir = Files.createTempDirectory(name).resolve("mappings").createDirectories()
    maps.forEach { (fileName, text) ->
        mapsDir.resolve(fileName).writeText(text.trimIndent() + "\n")
    }
    return mapsDir
}
