package j2me.semantic

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.collections.shouldContainExactly
import io.kotest.matchers.collections.shouldContain
import io.kotest.matchers.maps.shouldContainExactly
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.map.loadJavaLikeMappings
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodParameterSig
import j2me.model.MethodSig
import j2me.model.SemanticArraySemantics
import j2me.model.SemanticDomainKind
import j2me.model.SemanticTarget
import j2me.reports.semanticStats
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import kotlin.io.path.createDirectories
import kotlin.io.path.readText
import kotlin.io.path.writeText

class SemanticMappingsTest : FunSpec({
    test("built-in API semantics apply without project-local copies and remain reusable by project maps") {
        val mapsDir = Files.createTempDirectory("semantic-builtins").resolve("mappings").createDirectories()
        mapsDir.resolve("MenuFactory.map").writeText(
            """
            package game;

            import javax.microedition.lcdui.CommandType;

            class MenuFactory /* was a */ {
                void addCommand(@Domain(CommandType.class) int type) /* was m */;
            }
            """.trimIndent() + "\n",
        )
        mapsDir.resolve("LocalCommandApi.map").writeText(
            """
            package javax.microedition.lcdui;

            @ValueDomain interface LocalCommandType {}

            @External
            class Command {
                @DomainValue(LocalCommandType.class) static final int SCREEN;
                Command(String label, @Domain(LocalCommandType.class) int commandType, int priority) {}
            }
            """.trimIndent() + "\n",
        )

        val commandOwner = "javax/microedition/lcdui/Command"
        val commandFields = listOf("SCREEN", "BACK", "CANCEL", "OK", "HELP", "STOP", "EXIT", "ITEM")
            .map { FieldSig(commandOwner, it, "I") }
        val shortConstructor = MethodSig(commandOwner, "<init>", "(Ljava/lang/String;II)V")
        val longConstructor = MethodSig(commandOwner, "<init>", "(Ljava/lang/String;Ljava/lang/String;II)V")
        val getCommandType = MethodSig(commandOwner, "getCommandType", "()I")
        val commandSymbols = ClassSymbols(
            fields = commandFields,
            methods = listOf(shortConstructor, longConstructor, getCommandType),
            fieldAccess = commandFields.associateWith { Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL },
            fieldConstantValues = commandFields.zip((1..8).map(Int::toString)).toMap(),
        )

        val mappings = loadJavaLikeMappings(mapsDir, setOf("a"), setOf(commandOwner))
        val commandDomain = "javax.microedition.lcdui.CommandType"
        val projectMethod = MethodSig("a", "m", "(I)V")
        mappings.semantic.domains.keys shouldContain commandDomain
        mappings.semantic.realValues.values.toSet() shouldBe setOf(commandDomain)
        mappings.semantic.scalarDomains[SemanticTarget.Parameter(MethodParameterSig(shortConstructor, 1))] shouldBe commandDomain
        mappings.semantic.scalarDomains[SemanticTarget.Parameter(MethodParameterSig(longConstructor, 2))] shouldBe commandDomain
        mappings.semantic.scalarDomains[SemanticTarget.Return(getCommandType)] shouldBe commandDomain
        mappings.semantic.scalarDomains[SemanticTarget.Parameter(MethodParameterSig(projectMethod, 0))] shouldBe commandDomain

        validateSemanticMap(
            mappings.semantic,
            mappings.canonical,
            mapOf("a" to ClassSymbols(emptyList(), listOf(projectMethod))),
            mapOf(commandOwner to commandSymbols),
        )
    }

    test("built-in API packs stay inactive when their API is absent") {
        val mapsDir = Files.createTempDirectory("semantic-no-builtins").resolve("mappings").createDirectories()

        loadJavaLikeMappings(mapsDir, emptySet()).semantic shouldBe j2me.model.SemanticMap()
    }

    test("ordinary double signatures do not enter array semantic handling") {
        val mapsDir = Files.createTempDirectory("semantic-double-signatures").resolve("mappings").createDirectories()
        mapsDir.resolve("NumericHelper.map").writeText(
            """
            class NumericHelper /* was a */ {
                static double curve(float value) /* was a */;
                static double magnitude(double value) /* was b */;
            }
            """.trimIndent() + "\n",
        )

        val mappings = loadJavaLikeMappings(mapsDir, setOf("a"))

        mappings.canonical.methods shouldContainExactly mapOf(
            MethodSig("a", "a", "(F)D") to "curve",
            MethodSig("a", "b", "(D)D") to "magnitude",
        )
    }

    test("method returns can inherit the call-site domain of one parameter") {
        val mapsDir = Files.createTempDirectory("semantic-domain-flow").resolve("mappings").createDirectories()
        mapsDir.resolve("NumericHelper.map").writeText(
            """
            class NumericHelper /* was a */ {
                @DomainFromParameter(0)
                static int absolute(int value) /* was a */;
                static int sign(int value) /* was b */;
            }
            """.trimIndent() + "\n",
        )
        val absolute = MethodSig("a", "a", "(I)I")
        val sign = MethodSig("a", "b", "(I)I")

        val mappings = loadJavaLikeMappings(mapsDir, setOf("a"))

        mappings.semantic.returnDomainSources shouldContainExactly mapOf(absolute to 0)
        semanticStats(mappings.semantic).returnDomainSources shouldBe 1
        validateSemanticMap(
            mappings.semantic,
            mappings.canonical,
            mapOf("a" to ClassSymbols(emptyList(), listOf(absolute, sign))),
        )

        val sidecarPath = mapsDir.parent.resolve("semantic-map.json")
        buildSemanticMappings(
            mappings.semantic,
            mappings.canonical,
            mapOf("a" to ClassSymbols(emptyList(), listOf(absolute, sign))),
        ).write(sidecarPath)
        val sidecar = Json.parseToJsonElement(sidecarPath.readText()).jsonObject
        val source = sidecar["return_domain_sources"]?.jsonArray?.single()?.jsonObject
        source?.get("source_parameter")?.jsonPrimitive?.content shouldBe "0"
        source?.get("target")?.jsonObject?.get("name")?.jsonPrimitive?.content shouldBe "absolute"
    }

    test("return domain sources reject ambiguous and invalid contracts") {
        val conflictingDir = Files.createTempDirectory("semantic-domain-flow-conflict").resolve("mappings").createDirectories()
        conflictingDir.resolve("NumericHelper.map").writeText(
            """
            @ValueDomain interface ValueKind { int ONE = 1; }
            class NumericHelper /* was a */ {
                @Domain(ValueKind.class) @DomainFromParameter(0)
                static int convert(int value) /* was a */;
            }
            """.trimIndent() + "\n",
        )
        shouldThrow<IllegalArgumentException> {
            loadJavaLikeMappings(conflictingDir, setOf("a"))
        }.message.orEmpty() shouldContain "cannot be combined"

        val invalidTypeDir = Files.createTempDirectory("semantic-domain-flow-type").resolve("mappings").createDirectories()
        invalidTypeDir.resolve("NumericHelper.map").writeText(
            """
            class NumericHelper /* was a */ {
                @DomainFromParameter(0)
                static float absolute(float value) /* was a */;
            }
            """.trimIndent() + "\n",
        )
        val method = MethodSig("a", "a", "(F)F")
        val mappings = loadJavaLikeMappings(invalidTypeDir, setOf("a"))
        shouldThrow<IllegalArgumentException> {
            validateSemanticMap(
                mappings.semantic,
                mappings.canonical,
                mapOf("a" to ClassSymbols(emptyList(), listOf(method))),
            )
        }.message.orEmpty() shouldContain "integral return value"
    }

    test("semantic mappings can be disabled without disabling ordinary mappings") {
        val mapsDir = Files.createTempDirectory("semantic-disabled").resolve("mappings").createDirectories()
        mapsDir.resolve("Entity.map").writeText(
            """
            @ValueDomain
            class InvalidDirectionDomain {}

            class Entity /* was a */ {
                @Domain(MissingDomain.class) int direction /* was b */;
            }
            """.trimIndent() + "\n",
        )

        val mappings = loadJavaLikeMappings(
            mapsDir,
            knownProjectClasses = setOf("a"),
            includeSemanticMappings = false,
        )

        mappings.canonical.classes shouldContainExactly mapOf("a" to "Entity")
        mappings.canonical.fields shouldContainExactly mapOf(FieldSig("a", "b", "I") to "direction")
        mappings.semantic shouldBe j2me.model.SemanticMap()
    }

    test("slot annotations on scalar types produce a mapping error") {
        val mapsDir = Files.createTempDirectory("semantic-scalar-slots").resolve("mappings").createDirectories()
        mapsDir.resolve("NumericHelper.map").writeText(
            """
            @SlotDomain interface ValueSlot { int VALUE = 0; }
            class NumericHelper /* was a */ {
                @Slots(ValueSlot.class) double magnitude(double value) /* was a */;
            }
            """.trimIndent() + "\n",
        )

        val failure = shouldThrow<IllegalArgumentException> {
            loadJavaLikeMappings(mapsDir, setOf("a"))
        }

        failure.message.orEmpty() shouldContain "@Slots requires an array declaration"
    }

    test("built-in API packs adapt to the members exposed by an API profile") {
        val mapsDir = Files.createTempDirectory("semantic-partial-api").resolve("mappings").createDirectories()
        val owner = "javax/microedition/lcdui/Command"
        val screen = FieldSig(owner, "SCREEN", "I")
        val constructor = MethodSig(owner, "<init>", "(Ljava/lang/String;II)V")
        val symbols = ClassSymbols(
            fields = listOf(screen),
            methods = listOf(constructor),
            fieldAccess = mapOf(screen to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL)),
            fieldConstantValues = mapOf(screen to "1"),
        )

        val mappings = loadJavaLikeMappings(
            mapsDir,
            knownProjectClasses = emptySet(),
            knownClasspathClasses = setOf(owner),
            classpathSymbolsByClass = mapOf(owner to symbols),
        )

        mappings.semantic.realValues.keys shouldContainExactly listOf(screen)
        mappings.semantic.scalarDomains.keys shouldContainExactly listOf(
            SemanticTarget.Parameter(MethodParameterSig(constructor, 1)),
        )
        validateSemanticMap(mappings.semantic, mappings.canonical, emptyMap(), mapOf(owner to symbols))
    }

    test("semantic declarations become validated named sidecar data") {
        val mapsDir = Files.createTempDirectory("semantic-mappings").resolve("mappings").createDirectories()
        mapsDir.resolve("Entity.map").writeText(
            """
            package game;

            @ValueDomain
            interface Direction {
                int LEFT = -1;
                char UNKNOWN = '?';
            }

            @SlotDomain
            interface Properties {
                @SlotValue(Direction.class) int FACING = 2;
            }

            @FlagDomain
            interface Access {
                int READ = 1;
                int WRITE = 2;
            }

            class Entity /* was a */ {
                @DomainValue(Direction.class) public static final int RIGHT /* was r */;
                @Domain(Direction.class) int direction /* was d */;
                @Flags(Access.class) int access /* was f */;
                @Slots(Properties.class) byte[] properties /* was p */;
                @Slots(value = Properties.class, dimension = 1) byte[][] records /* was u */;
                @Domain(Direction.class) int[][] directionGrid /* was v */;
                @IndexDomain(value = Direction.class, dimension = 0)
                @IndexDomain(value = Direction.class, dimension = 1)
                byte[][] table /* was t */;
                @Domain(Direction.class) int getDirection() /* was g */;
                void setDirection(@Domain(Direction.class) int direction) /* was s */;
                @Domain(Direction.class)
                @IndexDomain(value = Direction.class, dimension = 0) byte[] getDirections() /* was h */;
                int readDirection(@Domain(Direction.class) @IndexDomain(value = Direction.class, dimension = 0) byte[] values) /* was i */;
                @Slots(Properties.class) byte[] getProperties() /* was j */;
                int readProperty(@Slots(Properties.class) byte[] record) /* was k */;
            }
            """.trimIndent() + "\n",
        )

        val mappings = loadJavaLikeMappings(mapsDir, setOf("a"))
        val right = FieldSig("a", "r", "I")
        val direction = FieldSig("a", "d", "I")
        val properties = FieldSig("a", "p", "[B")
        val records = FieldSig("a", "u", "[[B")
        val directionGrid = FieldSig("a", "v", "[[I")
        val table = FieldSig("a", "t", "[[B")
        val access = FieldSig("a", "f", "I")
        val getter = MethodSig("a", "g", "()I")
        val setter = MethodSig("a", "s", "(I)V")
        val arrayGetter = MethodSig("a", "h", "()[B")
        val arrayReader = MethodSig("a", "i", "([B)I")
        val slotGetter = MethodSig("a", "j", "()[B")
        val slotReader = MethodSig("a", "k", "([B)I")

        mappings.canonical.classes shouldContainExactly mapOf("a" to "game/Entity")
        mappings.semantic.domains.mapValues { it.value.kind } shouldContainExactly mapOf(
            "game.Direction" to SemanticDomainKind.VALUE,
            "game.Properties" to SemanticDomainKind.SLOTS,
            "game.Access" to SemanticDomainKind.FLAGS,
        )
        mappings.semantic.realValues[right] shouldBe "game.Direction"
        mappings.semantic.scalarDomains shouldContainExactly mapOf(
            SemanticTarget.Field(direction) to "game.Direction",
            SemanticTarget.Field(access) to "game.Access",
            SemanticTarget.Return(getter) to "game.Direction",
            SemanticTarget.Parameter(MethodParameterSig(setter, 0)) to "game.Direction",
        )
        mappings.semantic.arraySemantics shouldContainExactly mapOf(
            SemanticTarget.Field(properties) to SemanticArraySemantics(slotDomains = mapOf(0 to "game.Properties")),
            SemanticTarget.Field(records) to SemanticArraySemantics(slotDomains = mapOf(1 to "game.Properties")),
            SemanticTarget.Field(directionGrid) to SemanticArraySemantics(elementDomain = "game.Direction"),
            SemanticTarget.Field(table) to SemanticArraySemantics(indexDomains = mapOf(0 to "game.Direction", 1 to "game.Direction")),
            SemanticTarget.Return(arrayGetter) to SemanticArraySemantics(
                indexDomains = mapOf(0 to "game.Direction"),
                elementDomain = "game.Direction",
            ),
            SemanticTarget.Parameter(MethodParameterSig(arrayReader, 0)) to SemanticArraySemantics(
                indexDomains = mapOf(0 to "game.Direction"),
                elementDomain = "game.Direction",
            ),
            SemanticTarget.Return(slotGetter) to SemanticArraySemantics(slotDomains = mapOf(0 to "game.Properties")),
            SemanticTarget.Parameter(MethodParameterSig(slotReader, 0)) to
                SemanticArraySemantics(slotDomains = mapOf(0 to "game.Properties")),
        )

        val stats = semanticStats(mappings.semantic)
        stats.domainTotal shouldBe 3
        stats.valueTotal shouldBe 6
        stats.syntheticValueTotal shouldBe 5
        stats.realValueTotal shouldBe 1
        stats.fieldBindings shouldBe 2
        stats.returnBindings shouldBe 1
        stats.parameterBindings shouldBe 1
        stats.fieldArrayBindings shouldBe 4
        stats.returnArrayBindings shouldBe 2
        stats.parameterArrayBindings shouldBe 2
        stats.indexBindings shouldBe 4
        stats.slotBindings shouldBe 4
        stats.elementBindings shouldBe 3
        stats.slotValueLinks shouldBe 1
        stats.returnDomainSources shouldBe 0

        val symbols = mapOf(
            "a" to ClassSymbols(
                fields = listOf(right, direction, access, properties, records, directionGrid, table),
                methods = listOf(getter, setter, arrayGetter, arrayReader, slotGetter, slotReader),
                fieldAccess = mapOf(right to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL)),
                fieldConstantValues = mapOf(right to "1"),
            ),
        )
        validateSemanticMap(mappings.semantic, mappings.canonical, symbols)

        val sidecarPath = mapsDir.parent.resolve("semantic-map.json")
        buildSemanticMappings(mappings.semantic, mappings.canonical, symbols).write(sidecarPath)
        val sidecar = Json.parseToJsonElement(sidecarPath.readText()).jsonObject
        sidecar["scalar_bindings"]?.jsonArray?.size shouldBe 4
        sidecar["array_bindings"]?.jsonArray?.size shouldBe 8
        sidecar["values"]?.jsonArray?.map {
            val value = it.jsonObject
            Triple(value["owner"]?.jsonPrimitive?.content, value["name"]?.jsonPrimitive?.content, value["value"]?.jsonPrimitive?.content)
        } shouldContainExactly listOf(
            Triple("game/Access", "READ", "1"),
            Triple("game/Access", "WRITE", "2"),
            Triple("game/Direction", "LEFT", "-1"),
            Triple("game/Direction", "UNKNOWN", "63"),
            Triple("game/Properties", "FACING", "2"),
            Triple("game/Entity", "RIGHT", "1"),
        )
    }

    test("semantic bindings reject non-integral JVM types") {
        val method = MethodSig("a", "m", "(F)V")
        val mapsDir = Files.createTempDirectory("semantic-float").resolve("mappings").createDirectories()
        mapsDir.resolve("Entity.map").writeText(
            """
            @ValueDomain interface Mode { int ACTIVE = 1; }
            class Entity /* was a */ {
                void setMode(@Domain(Mode.class) float mode) /* was m */;
            }
            """.trimIndent() + "\n",
        )
        val mappings = loadJavaLikeMappings(mapsDir, setOf("a"))
        val symbols = mapOf("a" to ClassSymbols(fields = emptyList(), methods = listOf(method)))

        shouldThrow<IllegalArgumentException> {
            validateSemanticMap(mappings.semantic, mappings.canonical, symbols)
        }
    }

    test("external API declarations produce usable semantic bindings without renames") {
        val mapsDir = Files.createTempDirectory("semantic-external").resolve("mappings").createDirectories()
        mapsDir.resolve("GraphicsAnchor.map").writeText(
            """
            package game;

            @FlagDomain interface GraphicsAnchor {}
            """.trimIndent() + "\n",
        )
        mapsDir.resolve("Graphics.map").writeText(
            """
            package ext.api;

            import game.GraphicsAnchor;

            @External
            class Graphics {
                @DomainValue(GraphicsAnchor.class) public static final int LEFT;
                @DomainValue(GraphicsAnchor.class) public static final int TOP;
                void drawString(String text, int x, int y, @Flags(GraphicsAnchor.class) int anchor);
            }
            """.trimIndent() + "\n",
        )

        val left = FieldSig("ext/api/Graphics", "LEFT", "I")
        val top = FieldSig("ext/api/Graphics", "TOP", "I")
        val drawString = MethodSig("ext/api/Graphics", "drawString", "(Ljava/lang/String;III)V")
        val externalSymbols = mapOf(
            "ext/api/Graphics" to ClassSymbols(
                fields = listOf(left, top),
                methods = listOf(drawString),
                fieldAccess = mapOf(
                    left to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL),
                    top to (Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL),
                ),
                fieldConstantValues = mapOf(left to "4", top to "16"),
            ),
        )
        val mappings = loadJavaLikeMappings(mapsDir, emptySet(), externalSymbols.keys)
        mappings.canonical shouldBe j2me.model.CanonicalMap()

        val sidecarPath = mapsDir.parent.resolve("semantic-map.json")
        validateSemanticMap(mappings.semantic, mappings.canonical, emptyMap(), externalSymbols)
        buildSemanticMappings(mappings.semantic, mappings.canonical, emptyMap(), externalSymbols).write(sidecarPath)
        val sidecar = Json.parseToJsonElement(sidecarPath.readText()).jsonObject
        sidecar["values"]?.jsonArray?.map { value ->
            val entry = value.jsonObject
            Triple(
                entry["owner"]?.jsonPrimitive?.content,
                entry["name"]?.jsonPrimitive?.content,
                entry["value"]?.jsonPrimitive?.content,
            )
        } shouldContainExactly listOf(
            Triple("ext/api/Graphics", "LEFT", "4"),
            Triple("ext/api/Graphics", "TOP", "16"),
        )
        val binding = sidecar["scalar_bindings"]?.jsonArray?.single()?.jsonObject
        val target = binding?.get("target")?.jsonObject
        target?.get("owner")?.jsonPrimitive?.content shouldBe "ext/api/Graphics"
        target?.get("name")?.jsonPrimitive?.content shouldBe "drawString"
        target?.get("index")?.jsonPrimitive?.content shouldBe "3"
        binding?.get("domain")?.jsonPrimitive?.content shouldBe "game/GraphicsAnchor"
    }

    test("multidimensional slot layouts require an explicit dimension") {
        val mapsDir = Files.createTempDirectory("semantic-slots").resolve("mappings").createDirectories()
        mapsDir.resolve("Entity.map").writeText(
            """
            @SlotDomain interface RecordSlot { int TYPE = 0; }
            class Entity /* was a */ {
                @Slots(RecordSlot.class) int[][] records /* was r */;
            }
            """.trimIndent() + "\n",
        )

        shouldThrow<IllegalArgumentException> {
            loadJavaLikeMappings(mapsDir, setOf("a"))
        }
    }
})
