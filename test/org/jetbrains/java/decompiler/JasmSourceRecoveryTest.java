package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.MethodSource;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Stream;

public class JasmSourceRecoveryTest extends DecompileRegressionTestBase {
  private static final Object[] RENAMED = {
    IFernflowerPreferences.RENAME_ENTITIES, "1"
  };
  private static final Object[] STRICT_SLOTS = {
    IFernflowerPreferences.RENAME_ENTITIES, "1",
    IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, "1"
  };

  // The first fixture supplies the source assertions; any others are decompiled
  // alongside it. Extra sources are only for recompilation: these regressions
  // deliberately exercise missing declarations during decompilation.
  private record JasmCase(
    String name,
    List<String> fixtures,
    Object[] options,
    List<String> required,
    List<String> forbidden,
    Map<String, String> extraSources
  ) {
    JasmCase has(String... fragments) {
      return new JasmCase(name, fixtures, options, append(required, fragments), forbidden, extraSources);
    }

    JasmCase lacks(String... fragments) {
      return new JasmCase(name, fixtures, options, required, append(forbidden, fragments), extraSources);
    }

    JasmCase withOptions(Object... options) {
      return new JasmCase(name, fixtures, options, required, forbidden, extraSources);
    }

    JasmCase withSource(String path, String source) {
      Map<String, String> sources = new HashMap<>(extraSources);
      sources.put(path, source);
      return new JasmCase(name, fixtures, options, required, forbidden, Map.copyOf(sources));
    }

    @Override
    public String toString() {
      return name;
    }

    private static List<String> append(List<String> existing, String[] fragments) {
      return Stream.concat(existing.stream(), Stream.of(fragments)).toList();
    }
  }

  private static JasmCase jasm(String name, String... fixtures) {
    return new JasmCase(name, List.of(fixtures), new Object[0], List.of(), List.of(), Map.of());
  }

  @ParameterizedTest(name = "{0}")
  @MethodSource("cases")
  public void recoveredSourceRemainsCompilable(JasmCase testCase) throws IOException {
    String primary = testCase.fixtures().get(0);
    String source = testCase.fixtures().size() == 1
      ? decompileFixture(primary, testCase.options())
      : decompileDirectory(copyJasmClasses(testCase.fixtures().toArray(String[]::new)), "pkg/" + primary + ".java", testCase.options());
    assertContains(source, testCase.required().toArray(String[]::new));
    assertNotContains(source, testCase.forbidden().toArray(String[]::new));

    List<Path> extraSources = new ArrayList<>();
    for (Map.Entry<String, String> extraSource : testCase.extraSources().entrySet()) {
      extraSources.add(writeSource(extraSource.getKey(), extraSource.getValue()));
    }
    recompile(extraSources);
  }

  private static Stream<JasmCase> cases() {
    return Stream.of(
      // Checked exceptions and catch reachability.
      jasm("Checked catch with constructor throws", "TestCheckedCatchConstructorThrows"),
      jasm("Checked catch without declared throws", "TestCheckedCatchWithoutThrows"),
      jasm("Override infers checked throws allowed by superclass", "TestInheritedInputStreamThrows"),
      jasm("Missing checked throws are inferred", "TestMissingMethodThrows"),
      jasm("Private internal catch does not gain throws from another callsite", "TestPrivateInternalCatchNoThrows"),
      jasm("Private method keeps body-inferred checked throws without a callsite catch", "TestPrivateMissingThrowsNoCatch"),
      jasm("Broad exception and throwable catches are not narrowed", "TestBroadCatchWithoutDeclaredThrows")
        .has("catch (Exception", "catch (Throwable").lacks("catch (RuntimeException", "substituted checked catch types"),
      jasm("Handlers continuing a protected loop remain inside catch", "TestCatchHandlersLoopBack")
        .has("catch (InterruptedException", "catch (Exception"),
      // IOException needs a reachability marker; ClassNotFoundException remains
      // naturally reachable and must not be discarded from the same handler.
      jasm("Unthrowable checked type does not discard valid multi-catch types", "TestPartialMultiCatchRewrite")
        .has("IOException", "ClassNotFoundException", "if (false)").lacks("catch (RuntimeException", "Thread.currentThread()"),
      jasm("Private helper does not infer broad throws from a catch-all callsite", "TestBroadCatchUnknownInvocation")
        .lacks("helper() throws Exception")
        .withSource("pkg/MissingApi.java", """
          package pkg;
          public class MissingApi {
            public static void ping() {
            }
          }
          """),
      jasm("Same-class method infers checked throws from external API invocation", "TestExternalCheckedThrowsCallsite")
        .withSource("pkg/ExternalApi.java", """
          package pkg;
          import java.io.IOException;
          public class ExternalApi {
            public static Object create(String value) throws IOException {
              return value;
            }
          }
          """),
      jasm("Constructor infers throws from another class with inferred throws",
        "TestCrossClassMissingThrowsCaller", "TestCrossClassMissingThrowsProducer"),
      jasm("Undeclared checked exception is wrapped at a no-throws override boundary",
        "TestUndeclaredCheckedExceptionOverrideBoundary", "TestUndeclaredCheckedExceptionOverrideBoundaryBase")
        .has("helper() throws IOException", "public void run() {", "catch (IOException", "new RuntimeException")
        .lacks("public void run() throws IOException"),
      // The first analyzed member of this recursion cycle throws. Caching an
      // incomplete answer for the other member used to omit its throws clause.
      jasm("Mutual recursion propagates checked throws across the cycle", "TestMutualRecursionCheckedThrows").withOptions(RENAMED),

      // Local slots and reference constraints.
      jasm("Object-to-int slot reuse in arithmetic", "TestObjectIntSlotReuseMath"),
      jasm("Parameter slot reuse for array-element member access", "TestParamSlotReuseArrayElementMemberAccess"),
      jasm("Parameter self-store initializes the split local", "TestParameterSelfStoreSplit"),
      jasm("Baload switch selector from a reused slot", "TestBaloadSlotReuseSwitch")
        .has("case 119:", "case 120:").lacks("switch (((Object[])"),
      jasm("No-op receiver and parameter self-assignments are omitted", "TestThisSelfAssignment").lacks("this = this;", "var1 = var1;"),
      jasm("Boolean parameter slot reused as an integer argument", "TestBooleanParamSlotReusedAsIntArgument").withOptions(RENAMED),
      jasm("Null-seeded slot keeps incompatible reference uses separate", "TestNullSeededSlotConflictingReferenceTypes")
        .withOptions(STRICT_SLOTS),
      jasm("Null-seeded string concatenation slot retains string type", "TestStringConcatNullSeededSlot").withOptions(STRICT_SLOTS),
      jasm("Boolean array slot reused across a switch", "TestBooleanArraySlotHugeSwitch")
        .withOptions(STRICT_SLOTS).has("boolean[]").lacks("(byte[])"),
      jasm("Null slot does not erase a later object-use constraint", "TestNullSlotMergeUpperBound").withOptions(STRICT_SLOTS).has("String"),

      // Numeric conversions and overload resolution.
      jasm("Boolean and integer stack coercions render as Java conversions", "TestBooleanIntStackCoercion")
        .has("return (short)(flag() ? 1 : 0);", "return (this.p != 0);", "this.p = (short)(var1 ? 1 : 0);",
          "return (var1 != 0 ? (this.p != 0) : flag());")
        .lacks("(short)flag()", "(boolean)this.p", "(short)var1", "var0 ? 1 : 0"),
      jasm("Compound multiplier keeps original integer-store semantics", "TestCompoundIntMultiplierNoByteNarrow")
        .has("int var3 = 1;", "var3 *= 10").lacks("byte var3", "(byte)var3"),
      jasm("Dup-stored integer comparison does not invent byte narrowing", "TestIntNormalizeNoByteNarrow")
        .has("int var1;", "int var2 = (var1 =").lacks("byte ", "(byte)"),
      jasm("Byte comparison does not render a boolean literal", "TestNumericCompareBooleanLiteral").lacks("!= true", "== true"),
      jasm("InputStream long overload keeps its disambiguating cast", "TestOverloadIntLongAmbiguity")
        .withOptions(RENAMED).has("a((InputStream)var0, (long)(var1 - 2));"),
      jasm("Byte-int overload keeps its disambiguating cast", "TestPrimitiveOverloadAmbiguity")
        .withOptions(RENAMED).has("e((byte)0, (int)var0, value());"),

      // Constructors, initializers, and referenced members.
      jasm("This-constructor call after a branch precedes statements", "TestConstructorThisCallAfterBranch"),
      jasm("Remote constructor across a switch resugars to a new expression",
        "TestRemoteConstructorAcrossSwitch", "TestRemoteConstructorAcrossSwitchBox")
        .has("new TestRemoteConstructorAcrossSwitchBox(", "makeDupStore", "makeDupStoreInterleaved", "makeAliasStore")
        .lacks("Unable to resugar constructor", "new TestRemoteConstructorAcrossSwitchBox;"),
      jasm("Static initializer break through finally stays inside the initializer", "TestStaticInitializerTryFinallyBreak")
        .withOptions(RENAMED).lacks("return;"),
      jasm("Referenced synthetic fields remain in the root class", "TestSyntheticReferencedField"),

      // Language-version compatibility, including the explicit opt-out.
      jasm("Pre-Java-7 shared handler renders separate catches by default", "TestLegacyMultiCatchRendering")
        .has("catch (IOException", "catch (ClassNotFoundException").lacks(" | "),
      jasm("Pre-Java-7 shared handler renders multi-catch when opted out", "TestLegacyMultiCatchRendering")
        .withOptions(IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "0").has("catch (IOException | ClassNotFoundException"),
      jasm("Legacy source declares the broad catch rethrow type", "TestLegacyBroadCatchRethrow").has("throws IOException, Exception"),
      jasm("Pre-Java-5 array loop avoids enhanced for by default", "TestLegacyArrayForeachRendering").has("for (int ").lacks(" : ")
    );
  }
}
