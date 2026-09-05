package j2me.semantic

import io.kotest.assertions.throwables.shouldThrow
import io.kotest.core.spec.style.FunSpec
import io.kotest.engine.spec.tempdir
import io.kotest.matchers.shouldBe
import io.kotest.matchers.string.shouldContain
import j2me.map.loadJavaLikeMappings
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MethodSig
import j2me.validation.validateMap
import j2me.reports.writeSemanticReport
import org.jetbrains.java.decompiler.api.SemanticMappingData
import org.objectweb.asm.Opcodes
import java.nio.file.Files
import kotlin.io.path.writeText

class SemanticAdvancedMappingsTest : FunSpec({
    val tempRoot = tempdir("semantic-advanced").toPath()

    test("project constructors preserve parameter names and semantic contracts") {
        val dir = Files.createTempDirectory(tempRoot, "constructors")
        val source = dir.resolve("Subject.map")
        source.writeText("""
            @ValueDomain interface State { int READY = 2; }
            class Subject /* was a */ { Subject(@Domain(State.class) int state) {}
                Subject(long wide, @Domain(State.class) int state) {}
            }
        """.trimIndent())
        val constructors = listOf(MethodSig("a", "<init>", "(I)V"), MethodSig("a", "<init>", "(JI)V"))
        val symbols = mapOf("a" to ClassSymbols(emptyList(), constructors))
        val mappings = loadJavaLikeMappings(dir, symbols.keys)
        validateMap(symbols, mappings.canonical)
        validateSemanticMap(mappings.semantic, mappings.canonical, symbols)
        mappings.canonical.methods.values.toSet() shouldBe setOf("<init>")
        mappings.canonical.methodArgs[constructors[0]] shouldBe listOf("state")
        mappings.canonical.methodArgs[constructors[1]] shouldBe listOf("wide", "state")
        buildSemanticMappings(mappings.semantic, mappings.canonical, symbols).scalarBindings().size shouldBe 2
        shouldThrow<IllegalArgumentException> { validateMap(emptyMap(), mappings.canonical) }
        for ((declaration, error) in listOf(
            "Subject(int state) { state = 2; }" to "empty body",
            "Subject(int state) /* was a */ {}" to "was",
            "Wrong(int state) {}" to "constructor name",
        )) {
            source.writeText("class Subject /* was a */ { $declaration }")
            shouldThrow<IllegalArgumentException> { loadJavaLikeMappings(dir, symbols.keys) }.message.orEmpty() shouldContain error
        }
    }

    test("table column sources require an explicit innermost dimension on nested arrays") {
        val dir = Files.createTempDirectory(tempRoot, "nested-tables")
        val source = dir.resolve("Subject.map")
        val symbols = mapOf("a" to ClassSymbols(emptyList(), listOf(MethodSig("a", "m", "([[II)I"))))
        fun mapping(dimension: String) = """
            class Subject /* was a */ {
                @DomainFromSlot(parameter = 0, slot = 1$dimension)
                int lookup(int[][] table, @DomainFromSlot(parameter = 0, slot = 0$dimension) int key) /* was m */;
            }
        """.trimIndent()
        source.writeText(mapping(", dimension = 1"))
        val mappings = loadJavaLikeMappings(dir, symbols.keys)
        validateSemanticMap(mappings.semantic, mappings.canonical, symbols)
        val data = buildSemanticMappings(mappings.semantic, mappings.canonical, symbols)
        data.slotDomainSources().map { it.dimension() } shouldBe listOf(1, 1)
        val json = dir.resolve("semantics.json")
        data.write(json)
        SemanticMappingData.read(json) shouldBe data
        for (dimension in listOf("", ", dimension = 0", ", dimension = -1", ", dimension = 2")) {
            source.writeText(mapping(dimension))
            shouldThrow<IllegalArgumentException> {
                val invalid = loadJavaLikeMappings(dir, symbols.keys)
                validateSemanticMap(invalid.semantic, invalid.canonical, symbols)
            }.message.orEmpty() shouldContain "dimension"
        }
    }

    test("packed formats strings containers conditional domains and planes validate and round trip") {
        val dir = Files.createTempDirectory(tempRoot, "semantic-advanced")
        dir.resolve("Subject.map").writeText("""
            @ValueDomain interface State { int READY = 2; }
            @PackedDomain
            @BitField(value = State.class, shift = 3, bits = 5)
            interface Packed {}
            @NumericDomain(format = "fixed", fractionBits = 8) interface Coordinate {}
            @StringDomain interface Token { String READ = "read"; String WRITE = "write"; }
            @SlotDomain interface Planes { @SlotValue(State.class) int STATE = 0; int COUNT = 1; }
            class Subject /* was a */ {
                @Domain(Packed.class) int packed /* was p */;
                @Domain(Coordinate.class) int coordinate /* was c */;
                @Domain(Token.class) String token /* was t */;
                @Elements(State.class) java.util.Vector paths /* was v */;
                @Keys(Token.class) @Values(State.class) java.util.Hashtable table /* was h */;
                @Planes(value = Planes.class, stride = 101) int[] planes /* was r */;
                @DomainWhen(value = State.class, parameter = 0, equals = 1)
                @DomainWhen(value = State.class, parameter = 0, otherwise = true)
                int decode(int selector) /* was d */;
                boolean check(int selector, @DomainWhen(value = State.class, parameter = 0, notEquals = 1) int value) /* was b */;
                @DomainFromSlot(parameter = 0, slot = 1)
                int lookup(int[] table, @DomainFromSlot(parameter = 0, slot = 0) int key) /* was l */;
            }
        """.trimIndent())
        val fields = listOf("p" to "I", "c" to "I", "t" to "Ljava/lang/String;", "v" to "Ljava/util/Vector;",
            "h" to "Ljava/util/Hashtable;", "r" to "[I").map { (name, desc) -> FieldSig("a", name, desc) }
        val methods = listOf(MethodSig("a", "d", "(I)I"), MethodSig("a", "b", "(II)Z"), MethodSig("a", "l", "([II)I"))
        val symbols = mapOf("a" to ClassSymbols(fields, methods))
        val libraries = setOf("java/util/Vector", "java/util/Hashtable")
        val mappings = loadJavaLikeMappings(dir, symbols.keys, libraries)
        validateSemanticMap(mappings.semantic, mappings.canonical, symbols)
        val data = buildSemanticMappings(mappings.semantic, mappings.canonical, symbols)
        data.domains().single { it.id() == "defpackage/Packed" }.bitFields().single().bits() shouldBe 5
        data.domains().single { it.id() == "defpackage/Coordinate" }.format().fractionBits() shouldBe 8
        data.stringValues().map { it.value() }.toSet() shouldBe setOf("read", "write")
        data.containerBindings().size shouldBe 2
        data.conditionalBindings().size shouldBe 3
        data.conditionalBindings().count { it.otherwise() } shouldBe 1
        data.conditionalBindings().single { it.notEqualsValue() != null }.notEqualsValue() shouldBe 1L
        data.slotDomainSources().map { it.slot() }.toSet() shouldBe setOf(0, 1)
        data.arrayBindings().single().records().single().planes() shouldBe true
        val report = dir.resolve("semantic-summary.md")
        val stats = writeSemanticReport(report, mappings.semantic)
        stats.recordBindings shouldBe 0
        stats.planeBindings shouldBe 1
        stats.bitFieldLinks shouldBe 1
        val rows = Files.readAllLines(report).filter { it.startsWith("|") }.map { line -> line.split('|').map { it.trim() } }
        val state = rows.single { it[1] == "State" }
        for ((column, expected) in mapOf("Conditional" to "3", "Container roles" to "2", "Bit-field links" to "1")) {
            state[rows.first().indexOf(column)] shouldBe expected
        }
        val planes = rows.single { it[1] == "Planes" }
        planes[rows.first().indexOf("Planes")] shouldBe "1"
        planes[rows.first().indexOf("Records")] shouldBe "0"
        Files.readString(report) shouldContain "not source substitutions"
        val json = dir.resolve("semantics.json")
        data.write(json)
        SemanticMappingData.read(json) shouldBe data
    }

    test("string API constants retain String values and descriptors") {
        val dir = Files.createTempDirectory(tempRoot, "semantic-string-api")
        val owner = "javax/microedition/io/HttpConnection"
        val fields = listOf("GET", "POST", "HEAD").map { FieldSig(owner, it, "Ljava/lang/String;") }
        val methods = listOf(MethodSig(owner, "setRequestMethod", "(Ljava/lang/String;)V"), MethodSig(owner, "getRequestMethod", "()Ljava/lang/String;"))
        val api = mapOf(owner to ClassSymbols(fields, methods,
            fieldAccess = fields.associateWith { Opcodes.ACC_PUBLIC or Opcodes.ACC_STATIC or Opcodes.ACC_FINAL },
            fieldConstantValues = fields.associateWith { it.name }))
        val mappings = loadJavaLikeMappings(dir, emptySet(), api.keys, api)
        validateSemanticMap(mappings.semantic, mappings.canonical, emptyMap(), api)
        val data = buildSemanticMappings(mappings.semantic, mappings.canonical, emptyMap(), api)
        data.stringValues().map { it.value() }.toSet() shouldBe setOf("GET", "POST", "HEAD")
        data.scalarBindings().size shouldBe 2
    }

    test("invalid and overlapping packed fields and cyclic domains are rejected") {
        val dir = Files.createTempDirectory(tempRoot, "semantic-invalid-packed")
        val source = dir.resolve("Packed.map")
        val prefix = "@ValueDomain interface State { int READY = 2; }\n"
        val cases = listOf(
            "@PackedDomain @BitField(value = State.class, bits = 1) interface P {}" to "do not fit",
            "@PackedDomain @BitField(value = State.class, shift = 63, bits = 2) interface P {}" to "bit range",
            "@PackedDomain @BitField(value = State.class, bits = 3) @BitField(value = State.class, shift = 1, bits = 2) interface P {}" to "Overlapping",
            "@PackedDomain @BitField(value = P.class, bits = 3) interface P {}" to "Cyclic",
            "@PackedDomain @BitField(value = State.class, bits = 3, selectorMask = 7, selectorValue = 8) interface P {}" to "Selector value",
            "@NumericDomain(format = \"fixed\", fractionBits = 64) interface P {}" to "numeric format",
        )
        for ((text, error) in cases) {
            source.writeText(prefix + text)
            shouldThrow<IllegalArgumentException> {
                val mappings = loadJavaLikeMappings(dir, emptySet())
                validateSemanticMap(mappings.semantic, mappings.canonical, emptyMap())
            }.message.orEmpty() shouldContain error
        }
        source.writeText(prefix + """
            @PackedDomain
            @BitField(value = State.class, shift = 3, bits = 5, selectorMask = 7, selectorValue = 1)
            @BitField(value = State.class, shift = 3, bits = 5, selectorMask = 7, selectorValue = 2)
            interface P {}
        """.trimIndent())
        val mappings = loadJavaLikeMappings(dir, emptySet())
        validateSemanticMap(mappings.semantic, mappings.canonical, emptyMap())
    }

    test("conditional selectors and container roles require compatible declarations") {
        val dir = Files.createTempDirectory(tempRoot, "semantic-invalid-contexts")
        val source = dir.resolve("Subject.map")
        val method = MethodSig("a", "m", "(II)I")
        val field = FieldSig("a", "v", "Ljava/util/Vector;")
        val symbols = mapOf("a" to ClassSymbols(listOf(field), listOf(method)))
        for ((declaration, error) in listOf(
            "@DomainWhen(value = State.class, parameter = 2, equals = 1) int read(int a, int b) /* was m */;" to "selector",
            "@DomainWhen(value = State.class, parameter = 0, equals = 2147483648L) int read(int a, int b) /* was m */;" to "selector's range",
            "@DomainWhen(value = State.class, parameter = 0, equals = 1, notEquals = 2) int read(int a, int b) /* was m */;" to "exactly one",
            "@DomainWhen(value = State.class, parameter = 0, otherwise = true) int read(int a, int b) /* was m */;" to "explicit equals",
            "@DomainWhen(value = State.class, parameter = 0, equals = 2) @DomainWhen(value = State.class, parameter = 0, notEquals = 1) int read(int a, int b) /* was m */;" to "Overlapping",
            "@DomainFromSlot(parameter = 0, slot = 1) int read(int a, int b) /* was m */;" to "integral array",
            "@DomainFromSlot(parameter = 2, slot = 1) int read(int a, int b) /* was m */;" to "source",
            "@Keys(State.class) java.util.Vector vector /* was v */;" to "Use @Keys/@Values",
            "@Domain(Token.class) int read(int a, int b) /* was m */;" to "compatible",
        )) {
            source.writeText("@ValueDomain interface State { int READY = 2; }\n@StringDomain interface Token { String GET = \"GET\"; }\nclass Subject /* was a */ {\n$declaration\n}")
            shouldThrow<IllegalArgumentException> {
                val mappings = loadJavaLikeMappings(dir, symbols.keys, setOf("java/util/Vector"))
                validateSemanticMap(mappings.semantic, mappings.canonical, symbols)
            }.message.orEmpty() shouldContain error
        }
    }
})
