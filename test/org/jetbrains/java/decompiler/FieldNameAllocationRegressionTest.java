package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class FieldNameAllocationRegressionTest extends DecompileRegressionTestBase implements Opcodes {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void libraryQualifierRepairsOwnFieldsAndWarnsOnlyForFixedConflicts(boolean fixedPackageField) throws Exception {
    Path targetSource = writeSource("p/Target.java", "package p; public class Target { public static int value = 33; }");
    Path namesSource = writeSource("p/Names.java", "package p; public interface Names { int Target = 1; "
      + (fixedPackageField ? "int p = 2;" : "") + " }");
    compileJava8NoDebug(List.of(targetSource, namesSource), outRoot());
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_8, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", new String[]{"p/Names"});
    if (!fixedPackageField) probe.visitField(ACC_PUBLIC | ACC_STATIC, "p", "I", null, 2).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "p/Target", "value", "I");
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
    assertEquals(33, invoke(outRoot()));
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    for (String name : List.of("Target", "Names")) {
      Files.move(outRoot().resolve("p/" + name + ".class"), library.resolve("p/" + name + ".class"));
    }
    List<String> warnings = new CopyOnWriteArrayList<>();
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\toriginal\tnamed\n");
    Decompiler.Builder automatic = builder().libraries(library.toFile())
      .option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString()).logger(new IFernflowerLogger() {
      @Override public void writeMessage(String message, Severity severity) {
        if (severity == Severity.WARN) warnings.add(message);
      }
      @Override public void writeMessage(String message, Severity severity, Throwable failure) {
        throw new AssertionError(message, failure);
      }
    });
    NamingPlan plan = automatic.build().prepareNames();
    assertEquals(fixedPackageField, warnings.stream().anyMatch(message -> message.contains("Cannot qualify external class p/Target")), warnings.toString());
    automatic.build().decompile();
    String source = Files.readString(fixture.getTargetDir().resolve("p/Probe.java"));
    warnings.clear();
    automatic.option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan).build().decompile();
    assertEquals(source, Files.readString(fixture.getTargetDir().resolve("p/Probe.java")));
    assertEquals(fixedPackageField, warnings.stream().anyMatch(message -> message.contains("Cannot qualify external class p/Target")), warnings.toString());
    if (!fixedPackageField) {
      assertNotEquals("p", fieldName(plan, "p/Probe", "p"));
      var fields = new LinkedHashMap<>(plan.fields());
      fields.put(new NamingPlan.Member("p/Probe", "p", "I"), "p");
      NamingPlan invalid = new NamingPlan(plan.classes(), fields, plan.methods(), plan.parameters(), plan.innerNames());
      IllegalArgumentException error = assertThrows(IllegalArgumentException.class,
        () -> automatic.preparedNames(invalid).build().prepareNames());
      assertTrue(error.getMessage().contains("obscure class"), error.getMessage());
      Path rebuilt = fixture.getTempDir().resolve("rebuilt");
      compileJava8(List.of(fixture.getTargetDir().resolve("p/Probe.java"), targetSource, namesSource), rebuilt);
      assertEquals(33, invoke(rebuilt));
    }
  }

  @Test
  void referencedSyntheticFieldsRemainVisibleToQualifierChecks() throws Exception {
    compileJava8NoDebug(writeSource("p/Target.java",
      "package p; public class Target { public static int value = 33; }"), outRoot());
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_5, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", null);
    probe.visitField(ACC_PUBLIC | ACC_STATIC | ACC_SYNTHETIC, "Target", "I", null, null).visitEnd();
    // Distinct descriptors keep a prepared same-name pair valid at the JVM
    // identity level, exercising the retained field's Java namespace check.
    probe.visitField(ACC_PUBLIC | ACC_STATIC, "p", "J", null, null).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "p/Probe", "Target", "I");
    run.visitFieldInsn(GETSTATIC, "p/Target", "value", "I");
    run.visitInsn(IADD);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
    Decompiler.Builder builder = builder().option(IFernflowerPreferences.REMOVE_SYNTHETIC, "1");
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("Target", fieldName(plan, "p/Probe", "Target"));
    for (String invalid : List.of("class", "p")) {
      var invalidFields = new LinkedHashMap<>(plan.fields());
      invalidFields.put(new NamingPlan.Member("p/Probe", "Target", "I"), invalid);
      NamingPlan invalidPlan = new NamingPlan(plan.classes(), invalidFields, plan.methods(), plan.parameters(), plan.innerNames());
      IllegalArgumentException failure = assertThrows(IllegalArgumentException.class,
        () -> builder.preparedNames(invalidPlan).build().prepareNames());
      assertTrue(failure.getMessage().contains(invalid.equals("class") ? "Invalid prepared field name" : "field names collide"),
        failure.getMessage());
    }
    var fields = new LinkedHashMap<>(plan.fields());
    fields.replaceAll((member, name) -> member.name());
    NamingPlan obscured = new NamingPlan(plan.classes(), fields, plan.methods(), plan.parameters(), plan.innerNames());
    IllegalArgumentException error = assertThrows(IllegalArgumentException.class,
      () -> builder.preparedNames(obscured).build().prepareNames());
    assertTrue(error.getMessage().contains("obscure"), error.getMessage());
    rebuildAndCompare(builder, plan, true, 33);
  }

  @Test
  void primitiveClassLiteralsDoNotReserveWrapperQualifiers() throws Exception {
    writeWrapperProbe();
    Decompiler.Builder builder = builder().libraries(writeIntegerLibrary().toFile());
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("Integer", fieldName(plan, "p/Probe", "Integer"));
    rebuildAndCompare(builder, plan, true, 3);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void localStaticQualifierUsesItsSourceName(boolean prepared) throws Exception {
    writeLocalQualifierProbe(false);
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("Local", fieldName(plan, "p/Probe", "Local"));
    rebuildAndCompare(builder, plan, prepared, 33);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void fixedFieldMovesTheSourceNameOfALocalQualifier(boolean prepared) throws Exception {
    Path librarySource = writeSource("p/Names.java", "package p; public interface Names { int Local = 11; }");
    compileJava8NoDebug(librarySource, outRoot());
    writeLocalQualifierProbe(true);
    assertEquals(33, invoke(outRoot()));
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    Files.move(outRoot().resolve("p/Names.class"), library.resolve("p/Names.class"));
    Decompiler.Builder builder = builder().libraries(library.toFile());
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("Local", plan.innerNames().get("p/Probe$1Local"));
    assertTrue(plan.classes().get("p/Probe$1Local").startsWith("p/Probe$1"));
    if (prepared) builder.preparedNames(plan);
    builder.build().decompile();
    recompile(List.of(librarySource));
    assertEquals(33, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  private void writeLocalQualifierProbe(boolean inherited) throws Exception {
    String localName = "p/Probe$1Local";
    ClassWriter local = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    local.visit(V1_5, ACC_SUPER, localName, null, "java/lang/Object", null);
    local.visitOuterClass("p/Probe", "run", "()I");
    local.visitInnerClass(localName, null, "Local", 0);
    local.visitField(ACC_STATIC | ACC_FINAL, "value", "I", null, 33).visitEnd();
    var init = local.visitMethod(0, "<init>", "()V", null, null);
    init.visitCode();
    init.visitVarInsn(ALOAD, 0);
    init.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
    init.visitInsn(RETURN);
    init.visitMaxs(0, 0);
    init.visitEnd();
    local.visitEnd();
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_5, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", inherited ? new String[]{"p/Names"} : null);
    probe.visitInnerClass(localName, null, "Local", 0);
    if (!inherited) probe.visitField(ACC_PUBLIC | ACC_STATIC, "Local", "I", null, null).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitTypeInsn(NEW, localName);
    run.visitInsn(DUP);
    run.visitMethodInsn(INVOKESPECIAL, localName, "<init>", "()V", false);
    run.visitInsn(POP);
    run.visitFieldInsn(GETSTATIC, localName, "value", "I");
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.createDirectories(outRoot().resolve("p"));
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
    Files.write(outRoot().resolve(localName + ".class"), local.toByteArray());
  }

  private void writeWrapperProbe() throws Exception {
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_8, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", null);
    probe.visitField(ACC_PUBLIC | ACC_STATIC, "Integer", "I", null, null).visitEnd();
    probe.visitField(ACC_PUBLIC | ACC_STATIC, "java", "I", null, null).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "java/lang/Integer", "TYPE", "Ljava/lang/Class;");
    run.visitMethodInsn(INVOKEVIRTUAL, "java/lang/Class", "getName", "()Ljava/lang/String;", false);
    run.visitMethodInsn(INVOKEVIRTUAL, "java/lang/String", "length", "()I", false);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.createDirectories(outRoot().resolve("p"));
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
  }

  private Path writeIntegerLibrary() throws Exception {
    ClassWriter wrapper = new ClassWriter(0);
    wrapper.visit(V1_8, ACC_PUBLIC | ACC_FINAL, "java/lang/Integer", null, "java/lang/Number", null);
    wrapper.visitField(ACC_PUBLIC | ACC_STATIC | ACC_FINAL, "TYPE", "Ljava/lang/Class;", null, null).visitEnd();
    wrapper.visitEnd();
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("java/lang"));
    Files.write(library.resolve("java/lang/Integer.class"), wrapper.toByteArray());
    return library;
  }

  @Test
  void preparedNamesRejectAnObscuredStaticQualifier() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Names.java", "package p; public interface Names { int Target = Integer.parseInt(\"11\"); }"),
      writeSource("p/Target.java", "package p; public class Target { public static int value = 33; }")
    ), outRoot());
    writeQualifierProbe("p/Names", "p/Target");
    Decompiler.Builder builder = builder();
    NamingPlan allocated = builder.build().prepareNames();
    var fields = new LinkedHashMap<>(allocated.fields());
    fields.put(new NamingPlan.Member("p/Names", "Target", "I"), "Target");
    NamingPlan obscured = new NamingPlan(allocated.classes(), fields, allocated.methods(),
      allocated.parameters(), allocated.innerNames());
    IllegalArgumentException error = assertThrows(IllegalArgumentException.class,
      () -> builder.preparedNames(obscured).build().prepareNames());
    assertTrue(error.getMessage().contains("obscure"), error.getMessage());
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void collisionRepairReservesOtherInheritedFields(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Names.java", "package p; public interface Names { int shared = Integer.parseInt(\"11\"); }"),
      writeSource("p/Other.java", "package p; public interface Other { int field_0 = Integer.parseInt(\"22\"); }"),
      writeSource("p/Parent.java", "package p; public class Parent { public static int shared = 33; }"),
      writeSource("p/Probe.java", """
        package p;
        public class Probe extends Parent implements Names, Other {
          public static int run() { return Names.shared + Other.field_0 + Parent.shared; }
        }
        """)
    ), outRoot());
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("field_0", fieldName(plan, "p/Other", "field_0"));
    assertNotEquals(fieldName(plan, "p/Names", "shared"), fieldName(plan, "p/Other", "field_0"));
    rebuildAndCompare(builder, plan, prepared, 66);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void qualifierRepairReservesFieldsInOtherSubtypes(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Names.java", "package p; public interface Names { int Target = Integer.parseInt(\"11\"); }"),
      writeSource("p/Other.java", "package p; public interface Other { int field_0 = Integer.parseInt(\"22\"); }"),
      writeSource("p/Sibling.java", "package p; public class Sibling implements Names, Other {}"),
      writeSource("p/Target.java", "package p; public class Target { public static int value = 33; }")
    ), outRoot());
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", new String[]{"p/Names"});
    probe.visitField(ACC_PUBLIC | ACC_STATIC, "p", "I", null, null).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "p/Target", "value", "I");
    run.visitFieldInsn(GETSTATIC, "p/Names", "Target", "I");
    run.visitInsn(IADD);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("field_0", fieldName(plan, "p/Other", "field_0"));
    assertNotEquals(fieldName(plan, "p/Names", "Target"), fieldName(plan, "p/Other", "field_0"));
    rebuildAndCompare(builder, plan, prepared, 44);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void generatedNameDoesNotStealALaterValidDeclaration(boolean prepared) throws Exception {
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", null);
    probe.visitField(ACC_PUBLIC | ACC_STATIC | ACC_FINAL, "if", "I", null, 11).visitEnd();
    probe.visitField(ACC_PUBLIC | ACC_STATIC | ACC_FINAL, "field_0", "I", null, 22).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "p/Probe", "if", "I");
    run.visitFieldInsn(GETSTATIC, "p/Probe", "field_0", "I");
    run.visitInsn(IADD);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.createDirectories(outRoot().resolve("p"));
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("field_0", fieldName(plan, "p/Probe", "field_0"));
    assertNotEquals("field_0", fieldName(plan, "p/Probe", "if"));
    rebuildAndCompare(builder, plan, prepared, 33);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void qualifierRepairCannotObscureAnEarlierTypeReference(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Names.java", "package p; public interface Names { int Target = Integer.parseInt(\"11\"); }"),
      writeSource("p/field_0.java", "package p; public class field_0 { public static int value = 17; }"),
      writeSource("p/Target.java", "package p; public class Target { public static int value = 33; }")
    ), outRoot());
    writeQualifierProbe("p/Names", "p/field_0", "p/Target");
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("field_0", fieldName(plan, "p/Names", "Target"));
    rebuildAndCompare(builder, plan, prepared, 50);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void mappedFieldsRemainDistinctFromLibraryDeclarations(boolean prepared) throws Exception {
    Path librarySource = writeSource("p/Library.java", "package p; public interface Library { int shared = Integer.parseInt(\"11\"); }");
    compileJava8NoDebug(List.of(librarySource,
      writeSource("p/Parent.java", "package p; public class Parent { public static int value = 22; }"),
      writeSource("p/Probe.java", """
        package p;
        public class Probe extends Parent implements Library {
          public static int run() { return Parent.value + Library.shared; }
        }
        """)
    ), outRoot());
    assertEquals(33, invoke(outRoot()));
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    Files.move(outRoot().resolve("p/Library.class"), library.resolve("p/Library.class"));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/Parent\tp/Parent\n\tf\tI\tvalue\tshared\n");
    Decompiler.Builder builder = builder().libraries(library.toFile())
      .option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("shared", fieldName(plan, "p/Parent", "value"));
    if (prepared) builder.option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan);
    builder.build().decompile();
    recompile(List.of(librarySource));
    assertEquals(33, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void inaccessibleFieldsDoNotReserveNamesAcrossAPackageBoundary(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Parent.java", """
        package p;
        public class Parent {
          private static int hidden = 11;
          static int lost = 22;
          public static int parentValue() { return hidden + lost; }
        }
        """),
      writeSource("q/Middle.java", "package q; public class Middle extends p.Parent {}"),
      writeSource("p/Probe.java", """
        package p;
        public class Probe extends q.Middle {
          public static int hidden = 33;
          public static int lost = 44;
          public static int run() { return hidden + lost + parentValue(); }
        }
        """)
    ), outRoot());
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    for (String owner : List.of("p/Parent", "p/Probe")) {
      assertEquals("hidden", fieldName(plan, owner, "hidden"));
      assertEquals("lost", fieldName(plan, owner, "lost"));
    }
    rebuildAndCompare(builder, plan, prepared, 110);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void hiddenInstanceFieldsRemainSeparateStorage(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Parent.java", """
        package p;
        public class Parent {
          public int value = 11;
          public int parentValue() { return value; }
        }
        """),
      writeSource("p/Probe.java", """
        package p;
        public class Probe extends Parent {
          public long value = 22L;
          public static int run() {
            Probe instance = new Probe();
            instance.value += 3;
            ((Parent)instance).value += 4;
            return (int)instance.value + instance.parentValue();
          }
        }
        """)
    ), outRoot());
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals(fieldName(plan, "p/Parent", "value"), plan.fields().get(new NamingPlan.Member("p/Probe", "value", "J")));
    rebuildAndCompare(builder, plan, prepared, 40);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void movingAQualifierCannotObscureItInAnotherClass(boolean prepared) throws Exception {
    Path librarySource = writeSource("p/Library.java", "package p; public interface Library { int Target = 11; }");
    compileJava8NoDebug(List.of(librarySource,
      writeSource("p/Target.java", "package p; public class Target { public static int value = 33; }")
    ), outRoot());
    ClassWriter earlier = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    earlier.visit(V1_3, ACC_PUBLIC, "p/Earlier", null, "java/lang/Object", null);
    earlier.visitField(ACC_PUBLIC | ACC_STATIC, "class_0", "I", null, null).visitEnd();
    earlier.visitField(ACC_PUBLIC | ACC_STATIC, "p", "I", null, null).visitEnd();
    var read = earlier.visitMethod(ACC_PUBLIC | ACC_STATIC, "read", "()I", null, null);
    read.visitCode();
    read.visitFieldInsn(GETSTATIC, "p/Target", "value", "I");
    read.visitInsn(IRETURN);
    read.visitMaxs(0, 0);
    read.visitEnd();
    earlier.visitEnd();
    Files.write(outRoot().resolve("p/Earlier.class"), earlier.toByteArray());
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", new String[]{"p/Library"});
    probe.visitField(ACC_PUBLIC | ACC_STATIC, "p", "I", null, null).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "p/Target", "value", "I");
    run.visitMethodInsn(INVOKESTATIC, "p/Earlier", "read", "()I", false);
    run.visitInsn(IADD);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
    assertEquals(66, invoke(outRoot()));
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    Files.move(outRoot().resolve("p/Library.class"), library.resolve("p/Library.class"));
    Decompiler.Builder builder = builder().libraries(library.toFile());
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals("p/class_0", plan.classes().get("p/Target"));
    if (prepared) builder.preparedNames(plan);
    builder.build().decompile();
    recompile(List.of(librarySource));
    assertEquals(66, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  private void writeQualifierProbe(String iface, String... targets) throws Exception {
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", new String[]{iface});
    probe.visitField(ACC_PUBLIC | ACC_STATIC, "p", "I", null, null).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    for (int i = 0; i < targets.length; i++) {
      run.visitFieldInsn(GETSTATIC, targets[i], "value", "I");
      if (i > 0) run.visitInsn(IADD);
    }
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    probe.visitEnd();
    Files.write(outRoot().resolve("p/Probe.class"), probe.toByteArray());
  }

  private Decompiler.Builder builder() {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private static String fieldName(NamingPlan plan, String owner, String name) {
    return plan.fields().get(new NamingPlan.Member(owner, name, "I"));
  }

  private void rebuildAndCompare(Decompiler.Builder builder, NamingPlan plan, boolean prepared, int expected) throws Exception {
    assertEquals(expected, invoke(outRoot()));
    if (prepared) builder.preparedNames(plan);
    builder.build().decompile();
    recompile();
    assertEquals(expected, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  private static Object invoke(Path classes) throws Exception {
    return invoke(classes, "p.Probe");
  }

  private static Object invoke(Path classes, String type) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return loader.loadClass(type).getMethod("run").invoke(null);
    }
  }
}
