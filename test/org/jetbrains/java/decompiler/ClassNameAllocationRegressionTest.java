package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.renamer.ConverterHelper;
import org.jetbrains.java.decompiler.util.PortablePathKey;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.text.Normalizer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.jar.JarEntry;
import java.util.jar.JarOutputStream;

import static org.junit.jupiter.api.Assertions.*;

class ClassNameAllocationRegressionTest extends DecompileRegressionTestBase implements Opcodes {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void packagePrefixesReserveTheirJavaNamespace(boolean prepared) throws Exception {
    Path input = jar(Map.of("p/Branch", valueClass("p/Branch", 17),
      "p/Branch/Leaf", valueClass("p/Branch/Leaf", 29)));
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/Branch", plan.classes().get("p/Branch"));
    assertEquals("p/Branch/Leaf", plan.classes().get("p/Branch/Leaf"));
    rebuild(builder, plan, prepared);
    assertValue(plan, "p/Branch", 17);
    assertValue(plan, "p/Branch/Leaf", 29);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void illegalPackagePrefixesMoveAsUnits(boolean mapped) throws Exception {
    ClassWriter helper = type("for/Helper", 0);
    valueMethod(helper, "secret", ACC_STATIC, 31);
    ClassWriter probe = type("for/Example", ACC_PUBLIC);
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitMethodInsn(INVOKESTATIC, "for/Helper", "secret", "()I", false);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    Path input = jar(Map.of("for/Helper", bytes(helper), "for/Example", bytes(probe),
      "for/deeper/Child", valueClass("for/deeper/Child", 7),
      "package_0/Existing", valueClass("package_0/Existing", 11)));
    Path library = Files.createDirectories(fixture.getTempDir().resolve("library/package_0_1"));
    Files.write(library.resolve("Library.class"), valueClass("package_0_1/Library", 13));
    Decompiler.Builder builder = builder(input, mapped).libraries(library.getParent().toFile());
    NamingPlan plan = builder.build().prepareNames();
    String pkg = packageName(plan.classes().get("for/Example"));
    assertNotEquals("for", pkg);
    assertNotEquals("package_0", pkg);
    assertNotEquals("package_0_1", pkg);
    assertEquals(pkg, packageName(plan.classes().get("for/Helper")));
    assertEquals(pkg + "/deeper", packageName(plan.classes().get("for/deeper/Child")));
    assertEquals("package_0/Existing", plan.classes().get("package_0/Existing"));
    rebuild(builder, plan, true);
    assertValue(plan, "for/Example", 31);
    assertValue(plan, "for/deeper/Child", 7);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void partialPackageRequestsPreserveOrdinaryPackageAccess(boolean libraryHelper) throws Exception {
    Path helperSource = writeSource("p/Helper.java", "package p; class Helper { static int value = 11; static int read() { return 22; } }");
    compileJava8NoDebug(List.of(helperSource,
      writeSource("p/Probe.java", "package p; public class Probe { public static int run() { return Helper.value + Helper.read(); } }")), outRoot());
    Decompiler.Builder builder = builder(outRoot(), false);
    if (libraryHelper) {
      Path library = Files.createDirectories(fixture.getTempDir().resolve("library/p"));
      Files.move(outRoot().resolve("p/Helper.class"), library.resolve("Helper.class"));
      builder.libraries(library.getParent().toFile());
    }
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/Probe\tq/Readable\n");
    builder.option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertEquals(libraryHelper ? "p/Readable" : "q/Readable", plan.classes().get("p/Probe"));
    if (!libraryHelper) assertEquals("q/Helper", plan.classes().get("p/Helper"));
    builder.option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan).build().decompile();
    List<Path> sources = new ArrayList<>(listJavaSources(fixture.getTargetDir()));
    if (libraryHelper) sources.add(helperSource);
    compileJava8(sources, fixture.getTempDir().resolve("rebuilt"));
    assertValue(plan, "p/Probe", 33);
  }

  @Test
  void conflictingPackageRequestsPreserveOriginalPackageGroups() throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17), "p/Second", valueClass("p/Second", 29),
      "r/Third", valueClass("r/Third", 41)));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/First\tq/First\nc\tp/Second\ts/Second\nc\tr/Third\tq/Third\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("q/First", plan.classes().get("p/First"));
    assertEquals("q/Second", plan.classes().get("p/Second"));
    assertNotEquals("q", packageName(plan.classes().get("r/Third")));
    NamingPlan split = new NamingPlan(Map.of("p/First", "q/First", "p/Second", "s/Second", "r/Third", "r/Third"), plan.fields(), plan.methods());
    NamingPlan merged = new NamingPlan(Map.of("p/First", "q/First", "p/Second", "q/Second", "r/Third", "q/Third"), plan.fields(), plan.methods());
    assertThrows(IllegalArgumentException.class, () -> builder(input, false).preparedNames(split).build().prepareNames());
    assertThrows(IllegalArgumentException.class, () -> builder(input, false).preparedNames(merged).build().prepareNames());
    rebuild(builder, plan, true);
    assertValue(plan, "p/First", 17);
    assertValue(plan, "p/Second", 29);
    assertValue(plan, "r/Third", 41);
  }

  @Test
  void packageRequestDoesNotDisplaceAnExistingPackage() throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17), "q/Second", valueClass("q/Second", 29)));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/First\tq/First\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("q_1/First", plan.classes().get("p/First"));
    assertEquals("q/Second", plan.classes().get("q/Second"));
    rebuild(builder, plan, true);
    assertValue(plan, "p/First", 17);
    assertValue(plan, "q/Second", 29);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void unchangedCaseOnlyClassesHaveDistinctPortableOutputs(boolean prepared) throws Exception {
    // A directory fixture would already lose an entry on a case-insensitive
    // host, before the decompiler ever had a chance to see both identities.
    Path input = jar(Map.of("p/Readable", valueClass("p/Readable", 17),
      "p/READABLE", valueClass("p/READABLE", 29)));
    Decompiler.Builder builder = builder(input, false);
    NamingPlan plan = builder.build().prepareNames();
    assertDistinctPaths(plan);
    rebuild(builder, plan, prepared);
    assertEquals(2, listJavaSources(fixture.getTargetDir()).size());
    assertValue(plan, "p/Readable", 17);
    assertValue(plan, "p/READABLE", 29);
  }

  @Test
  void reservedHelperOutputsAlsoReserveTheirFoldedPaths() throws Exception {
    Path input = jar(Map.of("p/READABLE", valueClass("p/READABLE", 17)));
    Decompiler.Builder builder = builder(input, true).option(IFernflowerPreferences.RESERVED_CLASS_NAMES, "p/Readable");
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/readable", plan.classes().get("p/READABLE").toLowerCase(Locale.ROOT));
    NamingPlan invalid = new NamingPlan(Map.of("p/READABLE", "p/READABLE"), plan.fields(), plan.methods());
    assertThrows(IllegalArgumentException.class, () -> builder(input, false)
      .option(IFernflowerPreferences.RESERVED_CLASS_NAMES, "p/Readable").preparedNames(invalid).build().prepareNames());
    rebuild(builder, plan, true);
    assertValue(plan, "p/READABLE", 17);
  }

  @Test
  void explicitlyMappedCaseOnlyClassesHaveDistinctPortableOutputs() throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17),
      "p/Second", valueClass("p/Second", 29)));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/First\tq/Readable\nc\tp/Second\tq/READABLE\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertDistinctPaths(plan);
    rebuild(builder, plan, true);
    assertValue(plan, "p/First", 17);
    assertValue(plan, "p/Second", 29);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void classRequestsCannotTakeAnotherOriginalIdentity(boolean prepared) throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17), "p/Second", valueClass("p/Second", 29)));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/First\tp/Second\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/Second", plan.classes().get("p/First"));
    assertEquals("p/Second", plan.classes().get("p/Second"));
    rebuild(builder, plan, prepared);
    assertValue(plan, "p/First", 17);
    assertValue(plan, "p/Second", 29);
  }

  @Test
  void invalidMappedSimpleNamesKeepTheirRequestedPackages() throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17), "p/Second", valueClass("p/Second", 29)));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/First\tq/if\nc\tp/Second\t/for//Second\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("q", packageName(plan.classes().get("p/First")));
    assertFalse(plan.classes().get("p/Second").startsWith("/"));
    assertFalse(plan.classes().get("p/Second").contains("//"));
    rebuild(builder, plan, true);
    assertValue(plan, "p/First", 17);
    assertValue(plan, "p/Second", 29);
  }

  @Test
  void configuredDefaultPackageSharesPartialMappingDestination() throws Exception {
    Path input = jar(Map.of("First", valueClass("First", 17), "Second", valueClass("Second", 29)));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tFirst\treadable/First\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString())
      .option(IFernflowerPreferences.DEFAULT_PACKAGE, "readable");
    NamingPlan plan = builder.build().prepareNames();
    assertEquals(packageName(plan.classes().get("First")), packageName(plan.classes().get("Second")));
    rebuild(builder, plan, true);
    assertValue(plan, "First", 17);
    assertValue(plan, "Second", 29);
  }

  @Test
  void canonicallyEquivalentUnicodePathsRemainDistinct() throws Exception {
    String first = "p/Caf\u00e9";
    String second = "p/Cafe\u0301";
    Path input = jar(Map.of(first, valueClass(first, 17), second, valueClass(second, 29)));
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals(Normalizer.normalize(plan.classes().get(first), Normalizer.Form.NFC),
      Normalizer.normalize(plan.classes().get(second), Normalizer.Form.NFC));
    rebuild(builder, plan, true);
    assertValue(plan, first, 17);
    assertValue(plan, second, 29);
  }

  @ParameterizedTest
  @CsvSource({"ſ,s", "ς,σ", "ẞ,ß"})
  void unicodeCaseEquivalentClassPathsRemainDistinct(String firstSuffix, String secondSuffix) throws Exception {
    String first = "p/Read" + firstSuffix;
    String second = "p/Read" + secondSuffix;
    assertEquals(PortablePathKey.of(first), PortablePathKey.of(second));
    assertEquals(PortablePathKey.of(first), PortablePathKey.of(PortablePathKey.of(first)));
    Path input = jar(Map.of(first, valueClass(first, 17), second, valueClass(second, 29)));
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    assertDistinctPaths(plan);
    NamingPlan invalid = new NamingPlan(Map.of(first, first, second, second), plan.fields(), plan.methods());
    assertThrows(IllegalArgumentException.class, () -> builder(input, false).preparedNames(invalid).build().prepareNames());
    rebuild(builder, plan, true);
    assertValue(plan, first, 17);
    assertValue(plan, second, 29);
  }

  @ParameterizedTest
  @CsvSource({"ſ,s", "ς,σ", "ẞ,ß"})
  void unicodeCaseEquivalentPackagesRemainDistinct(String firstPackage, String secondPackage) throws Exception {
    String first = firstPackage + "/First", second = secondPackage + "/Second";
    Path input = jar(Map.of(first, valueClass(first, 17), second, valueClass(second, 29)));
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals(PortablePathKey.of(packageName(plan.classes().get(first))), PortablePathKey.of(packageName(plan.classes().get(second))));
    rebuild(builder, plan, true);
    assertValue(plan, first, 17);
    assertValue(plan, second, 29);
  }

  @Test
  void occupiedCustomClassRequestUsesTheBuiltInFallback() throws Exception {
    Path input = jar(Map.of("p/ZProbe", valueClass("p/ZProbe", 17),
      "p/Taken", valueClass("p/Taken", 29), "p/Retry", valueClass("p/Retry", 41)));
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.USER_RENAMER_CLASS, SingleRequestClassRenamer.class.getName());
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("p/Taken", plan.classes().get("p/Taken"));
    assertEquals("p/Retry", plan.classes().get("p/Retry"));
    assertTrue(plan.classes().get("p/ZProbe").startsWith("p/class_"));
    builder.option(IFernflowerPreferences.USER_RENAMER_CLASS, "");
    rebuild(builder, plan, true);
    assertValue(plan, "p/ZProbe", 17);
  }

  @Test
  void illegalCustomClassRequestUsesTheBuiltInFallback() throws Exception {
    Path input = jar(Map.of("p/Probe", valueClass("p/Probe", 17)));
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.USER_RENAMER_CLASS, InvalidClassRenamer.class.getName());
    NamingPlan plan = builder.build().prepareNames();
    assertTrue(plan.classes().get("p/Probe").startsWith("p/class_"));
    builder.option(IFernflowerPreferences.USER_RENAMER_CLASS, "");
    rebuild(builder, plan, true);
    assertValue(plan, "p/Probe", 17);
  }

  @Test
  void caseOnlyPackageDirectoriesPreserveTheirSeparateIdentities() throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17),
      "P/Second", valueClass("P/Second", 29), "p/deep/Third", valueClass("p/deep/Third", 41),
      "P/deep/Fourth", valueClass("P/deep/Fourth", 53)));
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    String lower = packageName(plan.classes().get("p/First"));
    String upper = packageName(plan.classes().get("P/Second"));
    assertNotEquals(lower.toLowerCase(Locale.ROOT), upper.toLowerCase(Locale.ROOT));
    assertEquals(lower + "/deep", packageName(plan.classes().get("p/deep/Third")));
    assertEquals(upper + "/deep", packageName(plan.classes().get("P/deep/Fourth")));
    rebuild(builder, plan, true);
    assertValue(plan, "p/First", 17);
    assertValue(plan, "P/Second", 29);
    assertValue(plan, "p/deep/Third", 41);
    assertValue(plan, "P/deep/Fourth", 53);
  }

  @Test
  void caseOnlyMemberNamesPreserveBinaryAndSourceOwnership() throws Exception {
    ClassWriter outer = type("p/Outer", ACC_PUBLIC);
    Map<String, byte[]> classes = new LinkedHashMap<>();
    int value = 17;
    for (String simple : List.of("Readable", "READABLE")) {
      String name = "p/Outer$" + simple;
      outer.visitInnerClass(name, "p/Outer", simple, ACC_PUBLIC | ACC_STATIC);
      ClassWriter inner = type(name, ACC_PUBLIC);
      inner.visitInnerClass(name, "p/Outer", simple, ACC_PUBLIC | ACC_STATIC);
      valueMethod(inner, "run", ACC_PUBLIC | ACC_STATIC, value);
      value += 12;
      classes.put(name, bytes(inner));
    }
    valueMethod(outer, "run", ACC_PUBLIC | ACC_STATIC, 41);
    classes.put("p/Outer", bytes(outer));
    Path input = jar(classes);
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    assertDistinctPaths(plan);
    for (String original : classes.keySet()) {
      if (original.contains("$")) assertEquals(plan.classes().get("p/Outer") + "$" + plan.innerNames().get(original), plan.classes().get(original));
    }
    rebuild(builder, plan, true);
    assertEquals(1, listJavaSources(fixture.getTargetDir()).size());
    assertValue(plan, "p/Outer$Readable", 17);
    assertValue(plan, "p/Outer$READABLE", 29);
  }

  @Test
  void preparedNamesRejectIllegalAndConflictingClassNamespaces() throws Exception {
    Path input = jar(Map.of("p/First", valueClass("p/First", 17), "q/Second", valueClass("q/Second", 29)));
    NamingPlan plan = builder(input, true).build().prepareNames();
    for (Map<String, String> classes : List.of(
      Map.of("p/First", "p/Readable", "q/Second", "p/READABLE"),
      Map.of("p/First", "p/Branch", "q/Second", "p/Branch/Leaf"),
      Map.of("p/First", "for/First", "q/Second", "q/Second"),
      Map.of("p/First", "p/First", "q/Second", "P/Second")
    )) {
      NamingPlan invalid = new NamingPlan(classes, plan.fields(), plan.methods(), plan.parameters(), plan.innerNames());
      assertThrows(IllegalArgumentException.class, () -> builder(input, false).preparedNames(invalid).build().prepareNames());
    }
    Map<String, String> alternative = new HashMap<>(plan.classes());
    alternative.put("p/First", "legal/Alternative");
    NamingPlan valid = new NamingPlan(alternative, plan.fields(), plan.methods(), plan.parameters(), plan.innerNames());
    assertEquals(valid, builder(input, false).preparedNames(valid).build().prepareNames());
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void requestedClassRenameChainsAreAdjusted(boolean prepared) throws Exception {
    overlappingClassNames(prepared, false, false);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void requestedClassNameSwapsAreAdjusted(boolean prepared) throws Exception {
    overlappingClassNames(prepared, true, false);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void misplacedSourceEntriesStillRenameThroughTheirDeclaredIdentities(boolean prepared) throws Exception {
    overlappingClassNames(prepared, false, true);
  }

  private void overlappingClassNames(boolean prepared, boolean swap, boolean misplaced) throws Exception {
    ClassWriter first = type("p/First", ACC_PUBLIC);
    valueMethod(first, "run", ACC_PUBLIC | ACC_STATIC, 17);
    var call = first.visitMethod(ACC_PUBLIC | ACC_STATIC, "call", "()I", null, null);
    call.visitCode();
    call.visitMethodInsn(INVOKESTATIC, "p/Second", "run", "()I", false);
    call.visitInsn(IRETURN);
    call.visitMaxs(0, 0);
    call.visitEnd();
    Path input = jar(Map.of(misplaced ? "Alias" : "p/First", bytes(first), "p/Second", valueClass("p/Second", 29)));
    if (misplaced && prepared) {
      Path directory = Files.createDirectories(fixture.getTempDir().resolve("nested-input"));
      Files.move(input, directory.resolve("input.jar"));
      input = directory;
    }
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    String second = swap ? "p/First" : "p/Third";
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/First\tp/Second\nc\tp/Second\t" + second + "\n");
    Decompiler.Builder builder = builder(input, false).option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/Second", plan.classes().get("p/First"));
    assertNotEquals("p/First", plan.classes().get("p/Second"));
    NamingPlan overlap = new NamingPlan(Map.of("p/First", "p/Second", "p/Second", second), plan.fields(), plan.methods());
    Path originalInput = input;
    IllegalArgumentException rejection = assertThrows(IllegalArgumentException.class,
      () -> builder(originalInput, false).preparedNames(overlap).build().prepareNames());
    assertTrue(rejection.getMessage().contains("reuses another class's original name"), rejection.getMessage());
    rebuild(builder, plan, prepared);
    assertEquals(2, listJavaSources(fixture.getTargetDir()).size());
    assertValue(plan, "p/First", 17);
    assertValue(plan, "p/Second", 29);
    try (URLClassLoader loader = new URLClassLoader(new URL[]{fixture.getTempDir().resolve("rebuilt").toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      assertEquals(29, loader.loadClass(plan.classes().get("p/First").replace('/', '.')).getMethod("call").invoke(null));
    }
  }

  @Test
  void memberBinaryNamesCannotCollideWithPackagePrefixes() throws Exception {
    ClassWriter outer = type("p/Outer", ACC_PUBLIC);
    outer.visitInnerClass("p/Outer$Nested", "p/Outer", "Nested", ACC_PUBLIC | ACC_STATIC);
    valueMethod(outer, "run", ACC_PUBLIC | ACC_STATIC, 17);
    ClassWriter inner = type("p/Outer$Nested", ACC_PUBLIC);
    inner.visitInnerClass("p/Outer$Nested", "p/Outer", "Nested", ACC_PUBLIC | ACC_STATIC);
    valueMethod(inner, "run", ACC_PUBLIC | ACC_STATIC, 29);
    Path input = jar(Map.of("p/Outer", bytes(outer), "p/Outer$Nested", bytes(inner),
      "p/Outer$Nested/Leaf", valueClass("p/Outer$Nested/Leaf", 41)));
    Decompiler.Builder builder = builder(input, true);
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/Outer$Nested", plan.classes().get("p/Outer$Nested"));
    assertEquals("p/Outer$Nested/Leaf", plan.classes().get("p/Outer$Nested/Leaf"));
    rebuild(builder, plan, true);
    assertValue(plan, "p/Outer$Nested", 29);
    assertValue(plan, "p/Outer$Nested/Leaf", 41);
  }

  @Test
  void movingAnEnclosingQualifierPreservesGeneratedDescendantNames() throws Exception {
    Path librarySource = writeSource("p/Names.java", "package p; public interface Names { int Outer = 1; int p = 2; }");
    compileJava8NoDebug(librarySource, outRoot());
    Map<String, byte[]> classes = new LinkedHashMap<>();
    String[] owners = {"p/Outer", "p/Outer$if", "p/Outer$if$while"};
    for (int i = 0; i < owners.length; i++) {
      ClassWriter owner = type(owners[i], ACC_PUBLIC);
      owner.visitInnerClass(owners[1], owners[0], "if", ACC_PUBLIC | ACC_STATIC);
      owner.visitInnerClass(owners[2], owners[1], "while", ACC_PUBLIC | ACC_STATIC);
      valueMethod(owner, "run", ACC_PUBLIC | ACC_STATIC, 17 + i * 12);
      classes.put(owners[i], bytes(owner));
    }
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC | ACC_SUPER, "p/Probe", null, "java/lang/Object", new String[]{"p/Names"});
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitMethodInsn(INVOKESTATIC, "p/Outer", "run", "()I", false);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    classes.put("p/Probe", bytes(probe));
    Path input = jar(classes);
    NamingPlan beforeQualifierRepair = builder(input, true).build().prepareNames();
    Decompiler.Builder builder = builder(input, true).libraries(outRoot().toFile());
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/Outer", plan.classes().get("p/Outer"));
    for (int i = 1; i < owners.length; i++) {
      assertEquals(beforeQualifierRepair.innerNames().get(owners[i]), plan.innerNames().get(owners[i]));
      assertEquals(plan.classes().get(owners[i - 1]) + "$" + plan.innerNames().get(owners[i]), plan.classes().get(owners[i]));
    }
    builder(input, false).libraries(outRoot().toFile()).preparedNames(plan).build().decompile();
    List<Path> sources = new ArrayList<>(listJavaSources(fixture.getTargetDir()));
    sources.add(librarySource);
    compileJava8(sources, fixture.getTempDir().resolve("rebuilt"));
    assertValue(plan, "p/Probe", 17);
    for (int i = 0; i < owners.length; i++) assertValue(plan, owners[i], 17 + i * 12);
  }

  private Decompiler.Builder builder(Path input, boolean tiny) throws Exception {
    Decompiler.Builder builder = Decompiler.builder().inputs(input.toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, true)
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
    if (tiny) {
      Path mapping = fixture.getTempDir().resolve("identity.tiny");
      Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n");
      builder.option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    }
    return builder;
  }

  private void rebuild(Decompiler.Builder builder, NamingPlan plan, boolean prepared) throws Exception {
    if (prepared) {
      Path stored = fixture.getTempDir().resolve("prepared.tiny");
      plan.write(stored);
      assertEquals(plan, NamingPlan.read(stored));
      builder.option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(NamingPlan.read(stored));
    }
    builder.build().decompile();
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
  }

  private void assertValue(NamingPlan plan, String original, int expected) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{fixture.getTempDir().resolve("rebuilt").toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      String method = plan.methods().get(new NamingPlan.Member(original, "run", "()I"));
      assertEquals(expected, loader.loadClass(plan.classes().get(original).replace('/', '.')).getMethod(method).invoke(null));
    }
  }

  private static void assertDistinctPaths(NamingPlan plan) {
    assertEquals(plan.classes().size(), plan.classes().values().stream().map(PortablePathKey::of).distinct().count());
  }

  private Path jar(Map<String, byte[]> classes) throws Exception {
    Path jar = fixture.getTempDir().resolve("input.jar");
    try (JarOutputStream output = new JarOutputStream(Files.newOutputStream(jar))) {
      for (var entry : new java.util.TreeMap<>(classes).entrySet()) {
        output.putNextEntry(new JarEntry(entry.getKey() + ".class"));
        output.write(entry.getValue());
        output.closeEntry();
      }
    }
    return jar;
  }

  private static ClassWriter type(String name, int access) {
    ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    writer.visit(V1_3, access | ACC_SUPER, name, null, "java/lang/Object", null);
    return writer;
  }

  private static byte[] valueClass(String name, int value) {
    ClassWriter writer = type(name, ACC_PUBLIC);
    valueMethod(writer, "run", ACC_PUBLIC | ACC_STATIC, value);
    return bytes(writer);
  }

  private static void valueMethod(ClassWriter writer, String name, int access, int value) {
    var method = writer.visitMethod(access, name, "()I", null, null);
    method.visitCode();
    method.visitLdcInsn(value);
    method.visitInsn(IRETURN);
    method.visitMaxs(0, 0);
    method.visitEnd();
  }

  private static byte[] bytes(ClassWriter writer) { writer.visitEnd(); return writer.toByteArray(); }
  private static String packageName(String name) { return name.substring(0, Math.max(0, name.lastIndexOf('/'))); }

  public static class SingleRequestClassRenamer extends ConverterHelper {
    private boolean requested;

    @Override
    public boolean toBeRenamed(Type type, String owner, String name, String descriptor) {
      return type == Type.ELEMENT_CLASS && (owner.equals("Probe") || owner.equals("ZProbe"));
    }

    @Override
    public String getNextClassName(String owner, String simple) {
      if (requested) throw new AssertionError("Custom class naming should receive only one request");
      requested = true;
      return request();
    }

    protected String request() { return "p/Taken"; }
  }

  public static class InvalidClassRenamer extends SingleRequestClassRenamer {
    @Override
    protected String request() { return "bad-name"; }
  }
}
