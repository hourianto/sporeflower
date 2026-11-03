package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class PreparedMethodNamingRegressionTest extends DecompileRegressionTestBase {
  @Test
  void overrideContractCannotBeSplit() throws Exception {
    overrideFixture();
    NamingPlan plan = builder().build().prepareNames();
    reject(rename(plan, "q/Child", "value", "()I", "separate"), "override contract");
  }

  @Test
  void inheritedSiblingContractsCannotBeSplitWithoutAnImplementation() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/FirstFace.java", "package p; public interface FirstFace { int value(); }"),
      writeSource("p/SecondFace.java", "package p; public interface SecondFace { int value(); }"),
      writeSource("p/Combined.java", "package p; public interface Combined extends FirstFace, SecondFace {}")
    ), outRoot());
    NamingPlan plan = builder().build().prepareNames();
    reject(rename(plan, "p/FirstFace", "value", "()I", "separate"), "override contract");
  }

  @Test
  void libraryContractCannotBeRenamed() throws Exception {
    overrideFixture();
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    Files.move(outRoot().resolve("p/Base.class"), library.resolve("p/Base.class"));
    NamingPlan plan = builder().libraries(library.toFile()).build().prepareNames();
    IllegalArgumentException failure = assertThrows(IllegalArgumentException.class, () -> builder()
      .libraries(library.toFile()).preparedNames(rename(plan, "q/Child", "value", "()I", "separate")).build().decompile());
    assertTrue(failure.getMessage().contains("override contract"), failure.getMessage());
  }

  @Test
  void nativeContractCannotBeRenamedEvenWhenTheWholeFamilyAgrees() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Contract.java", "package p; public interface Contract { int value(); }"),
      writeSource("p/NativeBody.java", "package p; public class NativeBody implements Contract { public native int value(); }")
    ), outRoot());
    NamingPlan plan = builder().build().prepareNames();
    plan = rename(rename(plan, "p/Contract", "value", "()I", "other"), "p/NativeBody", "value", "()I", "other");
    reject(plan, "native contract");
  }

  @Test
  void preparedNamesCannotCreateAnAccidentalOverride() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Base.java", "package p; public class Base { public int first() { return 11; } }"),
      writeSource("q/Child.java", "package q; public class Child extends p.Base { public int second() { return 22; } }")
    ), outRoot());
    NamingPlan plan = builder().build().prepareNames();
    reject(rename(plan, "q/Child", "second", "()I", "first"), "method names collide");
  }

  @Test
  void relocationCannotSeparateAnExistingPackageOverride() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Base.java", "package p; public class Base { int value() { return 11; } }"),
      writeSource("p/Child.java", "package p; public class Child extends Base { public int value() { return 22; } }")
    ), outRoot());
    NamingPlan plan = builder().build().prepareNames();
    Map<String, String> classes = new HashMap<>(plan.classes());
    classes.put("p/Child", "q/Child");
    reject(new NamingPlan(classes, plan.fields(), plan.methods(), plan.parameters(), plan.innerNames()), "splits original package");
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void automaticRelocationKeepsPackageOverrideDispatch(boolean libraryBase) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Base.java", """
        package p;
        public class Base {
          int value() { return 11; }
          public int baseCall() { return value(); }
        }
        """),
      writeSource("p/Child.java", """
        package p;
        public class Child extends Base {
          public int value() { return 22; }
          public static int run() { Child child = new Child(); return child.baseCall() + child.value(); }
        }
        """)
    ), outRoot());
    assertEquals(44, invoke(outRoot(), "p.Child", "run"));
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    if (libraryBase) Files.move(outRoot().resolve("p/Base.class"), library.resolve("p/Base.class"));
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\toriginal\tnamed\nc\tp/Child\tq/Readable\n");
    Decompiler.Builder automatic = builder().libraries(library.toFile())
      .option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    NamingPlan plan = automatic.build().prepareNames();
    String emitted = libraryBase ? "p/Readable" : "q/Readable";
    assertEquals(emitted, plan.classes().get("p/Child"));
    if (libraryBase) {
      NamingPlan relocated = new NamingPlan(Map.of("p/Child", "q/Readable"), plan.fields(), plan.methods(), plan.parameters(), plan.innerNames());
      IllegalArgumentException failure = assertThrows(IllegalArgumentException.class,
        () -> builder().libraries(library.toFile()).preparedNames(relocated).build().prepareNames());
      assertTrue(failure.getMessage().contains("changes a library package boundary"), failure.getMessage());
    }
    automatic.build().decompile();
    String source = readDecompiled(emitted + ".java");
    builder().libraries(library.toFile()).preparedNames(plan).build().decompile();
    assertEquals(source, readDecompiled(emitted + ".java"));
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8ReleaseWithClasspath(listJavaSources(fixture.getTargetDir()), rebuilt, library);
    try (URLClassLoader loader = loader(rebuilt, library)) {
      assertEquals(44, loader.loadClass(emitted.replace('/', '.')).getMethod("run").invoke(null));
    }
  }

  @Test
  void illegalLibraryPackageReportsThatOverrideDispatchCannotBeRepresented() throws Exception {
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("for"));
    Files.createDirectories(outRoot().resolve("for"));
    ClassWriter base = new ClassWriter(0);
    base.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC | Opcodes.ACC_ABSTRACT, "for/Base", null, "java/lang/Object", null);
    base.visitMethod(Opcodes.ACC_ABSTRACT, "value", "()I", null, null).visitEnd();
    base.visitEnd();
    Files.write(library.resolve("for/Base.class"), base.toByteArray());
    ClassWriter child = new ClassWriter(0);
    child.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC | Opcodes.ACC_ABSTRACT, "for/Child", null, "for/Base", null);
    child.visitMethod(Opcodes.ACC_PUBLIC | Opcodes.ACC_ABSTRACT, "value", "()I", null, null).visitEnd();
    child.visitEnd();
    Files.write(outRoot().resolve("for/Child.class"), child.toByteArray());
    // A keyword package is legal to the JVM, even though Java cannot declare
    // an overriding class in that fixed library package.
    try (URLClassLoader loader = loader(outRoot(), library)) {
      assertEquals("value", loader.loadClass("for.Child").getDeclaredMethod("value").getName());
    }
    List<String> warnings = new CopyOnWriteArrayList<>();
    Decompiler.Builder automatic = builder().libraries(library.toFile()).logger(warnings(warnings));
    NamingPlan plan = automatic.build().prepareNames();
    assertEquals("value", plan.methods().get(new Member("for/Child", "value", "()I")));
    assertTrue(warnings.stream().anyMatch(message -> message.contains("Cannot preserve fixed library package")), warnings.toString());
    automatic.build().decompile();
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve(plan.classes().get("for/Child") + ".java")));
    // Unlike retaining an impossible source spelling, changing this package
    // actually loses the library override. A transport plan must not permit it.
    IllegalArgumentException failure = assertThrows(IllegalArgumentException.class,
      () -> builder().libraries(library.toFile()).preparedNames(plan).build().prepareNames());
    assertTrue(failure.getMessage().contains("changes a library package boundary"), failure.getMessage());
  }

  @ParameterizedTest
  @ValueSource(strings = {"private", ""})
  void inaccessibleAncestorDoesNotReserveAChildName(String access) throws Exception {
    independentFixture(access);
    NamingPlan plan = rename(builder().build().prepareNames(), "q/Child", "second", "()I", "first");
    rebuild(plan, "q.Child", "run", 33);
  }

  @Test
  void interfaceStaticDoesNotReserveAnImplementorName() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Contract.java", "package p; public interface Contract { static int first() { return 11; } }"),
      writeSource("q/Child.java", """
        package q;
        public class Child implements p.Contract {
          public static int second() { return 22; }
          public static int run() { return p.Contract.first() + second(); }
        }
        """)
    ), outRoot());
    NamingPlan plan = rename(builder().build().prepareNames(), "q/Child", "second", "()I", "first");
    rebuild(plan, "q.Child", "run", 33);
  }

  @Test
  void alternativeConsistentFamilyNamePreservesDispatch() throws Exception {
    overrideFixture();
    NamingPlan plan = builder().build().prepareNames();
    plan = rename(rename(plan, "p/Base", "value", "()I", "otherValue"), "q/Child", "value", "()I", "otherValue");
    rebuild(plan, "q.Child", "run", 44);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void visibleCovariantBridgeKeepsADistinctSourceDeclaration(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Provider.java", "package p; public interface Provider { Object value(); }"),
      writeSource("p/BridgeBody.java", """
        package p;
        public class BridgeBody implements Provider {
          public String value() { return "ok"; }
          public static String run() { return ((Provider)new BridgeBody()).value() + new BridgeBody().value(); }
        }
        """)
    ), outRoot());
    Decompiler.Builder builder = builder().option(IFernflowerPreferences.REMOVE_BRIDGE, false)
      .option(IFernflowerPreferences.REMOVE_SYNTHETIC, false);
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals(plan.methods().get(new Member("p/BridgeBody", "value", "()Ljava/lang/Object;")),
      plan.methods().get(new Member("p/BridgeBody", "value", "()Ljava/lang/String;")));
    if (prepared) builder.preparedNames(plan);
    builder.build().decompile();
    recompile();
    assertEquals("okok", invoke(outRoot(), "p.BridgeBody", "run"));
    assertEquals("okok", invoke(recompiledRoot(), "p.BridgeBody", "run"));
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void visibleLibraryPinnedBridgeReportsAnUnrepresentableContract(boolean prepared) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Provider.java", "package p; public interface Provider { Object value(); }"),
      writeSource("p/LibraryBase.java", """
        package p;
        public class LibraryBase implements Provider { public String value() { return "base"; } }
        """),
      writeSource("p/Child.java", "package p; public class Child extends LibraryBase { public String value() { return \"child\"; } public int other() { return 1; } }"),
      writeSource("p/Unrelated.java", "package p; public class Unrelated { public static int run() { return 33; } }")
    ), outRoot());
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("p"));
    for (String name : List.of("Provider", "LibraryBase")) {
      Files.move(outRoot().resolve("p/" + name + ".class"), library.resolve("p/" + name + ".class"));
    }
    List<String> warnings = new CopyOnWriteArrayList<>();
    Decompiler.Builder builder = builder().libraries(library.toFile()).logger(warnings(warnings))
      .option(IFernflowerPreferences.REMOVE_BRIDGE, false).option(IFernflowerPreferences.REMOVE_SYNTHETIC, false);
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("value", plan.methods().get(new Member("p/Child", "value", "()Ljava/lang/Object;")));
    assertEquals("value", plan.methods().get(new Member("p/Child", "value", "()Ljava/lang/String;")));
    assertTrue(warnings.stream().anyMatch(message -> message.contains("method names collide in Java")), warnings.toString());
    warnings.clear();
    if (prepared) builder.preparedNames(plan);
    builder.build().decompile();
    assertTrue(warnings.stream().anyMatch(message -> message.contains("method names collide in Java")), warnings.toString());
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8(List.of(fixture.getTargetDir().resolve("p/Unrelated.java")), rebuilt);
    assertEquals(33, invoke(rebuilt, "p.Unrelated", "run"));
    IllegalArgumentException failure = assertThrows(IllegalArgumentException.class,
      () -> builder.preparedNames(rename(plan, "p/Child", "other", "()I", "value")).build().prepareNames());
    assertTrue(failure.getMessage().contains("method names collide in Java"), failure.getMessage());
  }

  @Test
  void automaticParameterHintsIgnoreStaleSlotsButPreparedPlansRejectThem() throws Exception {
    compileJava8NoDebug(writeSource("p/Subject.java", """
      package p;
      public class Subject { public int value(int input) { return input + 1; } }
      """), outRoot());
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, """
      tiny\t2\t0\toriginal\tnamed
      c\tp/Subject\tp/Subject
      \tm\t(I)I\tvalue\tvalue
      \t\tp\t1\t\tinput
      \t\tp\t5\t\tstale
      """);
    List<String> warnings = new CopyOnWriteArrayList<>();
    Decompiler.Builder automatic = builder().option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString()).logger(warnings(warnings));
    NamingPlan plan = automatic.build().prepareNames();
    Member method = new Member("p/Subject", "value", "(I)I");
    assertEquals(Map.of(1, "input"), plan.parameters().get(method));
    assertTrue(warnings.stream().anyMatch(message -> message.contains("Ignoring mapped parameter slot 5")), warnings.toString());
    automatic.build().decompile();
    String source = readDecompiled("p/Subject.java");
    builder().preparedNames(plan).build().decompile();
    assertEquals(source, readDecompiled("p/Subject.java"));
    recompile();
    reject(new NamingPlan(plan.classes(), plan.fields(), plan.methods(), Map.of(method, Map.of(5, "stale")), plan.innerNames()),
      "Invalid prepared parameter slot");
  }

  @Test
  void invalidPreparedMethodAndParameterNamesAreRejected() throws Exception {
    compileJava8NoDebug(writeSource("p/Subject.java", """
      package p;
      public class Subject { public int value(int first, int second) { return first + second; } }
      """), outRoot());
    NamingPlan plan = builder().build().prepareNames();
    reject(rename(plan, "p/Subject", "value", "(II)I", "class"), "Invalid prepared method name");
    Member method = new Member("p/Subject", "value", "(II)I");
    reject(new NamingPlan(plan.classes(), plan.fields(), plan.methods(), Map.of(method, Map.of(1, "same", 2, "same")),
      plan.innerNames()), "parameter name");
    reject(new NamingPlan(plan.classes(), plan.fields(), plan.methods(), Map.of(method, Map.of(1, "class")),
      plan.innerNames()), "parameter name");
  }

  @Test
  void referencedSyntheticMethodStillReservesItsSourceSignature() throws Exception {
    compileJava8NoDebug(writeSource("p/Subject.java", """
      package p;
      public class Subject {
        public static String helper() { return "first"; }
        public static Object value() { return "second"; }
        public static String run() { return helper() + value(); }
      }
      """), outRoot());
    Path input = outRoot().resolve("p/Subject.class");
    ClassWriter writer = new ClassWriter(0);
    new ClassReader(Files.readAllBytes(input)).accept(new ClassVisitor(Opcodes.ASM9, writer) {
      @Override
      public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        if (name.equals("helper")) access |= Opcodes.ACC_SYNTHETIC;
        return super.visitMethod(access, name, descriptor, signature, exceptions);
      }
    }, 0);
    Files.write(input, writer.toByteArray());
    assertEquals("firstsecond", invoke(outRoot(), "p.Subject", "run"));
    NamingPlan plan = rename(builder().build().prepareNames(), "p/Subject", "helper", "()Ljava/lang/String;", "value");
    reject(plan, "method names collide");
  }

  private void overrideFixture() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Base.java", """
        package p;
        public class Base {
          public int value() { return 11; }
          public int baseCall() { return value(); }
        }
        """),
      writeSource("q/Child.java", """
        package q;
        public class Child extends p.Base {
          public int value() { return 22; }
          public static int run() { Child child = new Child(); return child.baseCall() + child.value(); }
        }
        """)
    ), outRoot());
  }

  private void independentFixture(String access) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Base.java", "package p; public class Base { " + access
        + " int first() { return 11; } public int baseCall() { return first(); } }"),
      writeSource("q/Child.java", """
        package q;
        public class Child extends p.Base {
          public int second() { return 22; }
          public static int run() { Child child = new Child(); return child.baseCall() + child.second(); }
        }
        """)
    ), outRoot());
  }

  private Decompiler.Builder builder() {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private static IFernflowerLogger warnings(List<String> messages) {
    return new IFernflowerLogger() {
      @Override public void writeMessage(String message, Severity severity) {
        if (severity == Severity.WARN) messages.add(message);
      }
      @Override public void writeMessage(String message, Severity severity, Throwable failure) {
        throw new AssertionError(message, failure);
      }
    };
  }

  private static NamingPlan rename(NamingPlan plan, String owner, String name, String descriptor, String emitted) {
    Map<Member, String> methods = new HashMap<>(plan.methods());
    Member key = new Member(owner, name, descriptor);
    assertTrue(methods.containsKey(key), key.toString());
    methods.put(key, emitted);
    return new NamingPlan(plan.classes(), plan.fields(), methods, plan.parameters(), plan.innerNames());
  }

  private void reject(NamingPlan plan, String message) {
    IllegalArgumentException failure = assertThrows(IllegalArgumentException.class, () -> builder().preparedNames(plan).build().decompile());
    assertTrue(failure.getMessage().contains(message), failure.getMessage());
  }

  private void rebuild(NamingPlan plan, String owner, String method, Object expected) throws Exception {
    assertEquals(expected, invoke(outRoot(), owner, method));
    builder().preparedNames(plan).build().decompile();
    recompile();
    assertEquals(expected, invoke(recompiledRoot(), owner, method));
  }

  private static Object invoke(Path classes, String owner, String method) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      return loader.loadClass(owner).getMethod(method).invoke(null);
    }
  }
}
