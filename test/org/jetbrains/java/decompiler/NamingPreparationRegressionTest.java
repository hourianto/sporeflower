package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.util.token.TextRange;
import org.jetbrains.java.decompiler.main.extern.TextTokenVisitor;
import javax.tools.ToolProvider;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.List;
import java.util.ArrayList;
import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;

import java.nio.file.Files;
import java.nio.file.Path;
import java.net.URLClassLoader;
import java.net.URL;

import static org.junit.jupiter.api.Assertions.*;

class NamingPreparationRegressionTest extends DecompileRegressionTestBase {
  private Decompiler.Builder builder(Path mapping) {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString())
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  @Test
  void preparedMemberClassNamesMatchCompiledBinaryNames() throws Exception {
    compileJava8(writeSource("p/Outer.java", """
      package p;
      public class Outer {
        public static class Inner { public int value() { return 17; } }
        public static int call() { return new Inner().value(); }
      }
      """), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\nc\tp/Outer\treadable/Container\nc\tp/Outer$Inner\treadable/Entry\n");
    NamingPlan names = builder(request).build().prepareNames();
    assertEquals("readable/Container$Entry", names.classes().get("p/Outer$Inner"));
    Path prepared = fixture.getTempDir().resolve("prepared.tiny");
    names.write(prepared);
    assertEquals(names, NamingPlan.read(prepared));
    builder(prepared).option(IFernflowerPreferences.MAPPINGS_PATH, "")
      .option(IFernflowerPreferences.PREPARED_NAMES_PATH, prepared.toString()).build().decompile();
    String source = Files.readString(fixture.getTargetDir().resolve("readable/Container.java"));
    assertFalse(source.contains("renamed from: value"), source);
    assertFalse(source.contains("renamed from: call"), source);
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
    assertTrue(Files.exists(fixture.getTempDir().resolve("rebuilt/readable/Container$Entry.class")));
  }

  @Test
  void dollarInNestedSimpleNameSurvivesPlacementAndPreparedNames() throws Exception {
    compileJava8(writeSource("p/Outer.java", """
      package p;
      public class Outer {
        public static class A$B { public int value() { return 3; } }
        public static class B { public int value() { return 4; } }
        public static int run() { return new A$B().value() + new B().value(); }
      }
      """), outRoot());
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n");
    NamingPlan unchanged = builder(mapping).build().prepareNames();
    assertEquals("p/Outer$A$B", unchanged.classes().get("p/Outer$A$B"));
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tp/Outer\tq/Readable\n");
    NamingPlan names = builder(mapping).build().prepareNames();
    assertEquals("q/Readable$A$B", names.classes().get("p/Outer$A$B"));
    assertEquals("A$B", names.innerNames().get("p/Outer$A$B"));
    builder(mapping).option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(names).build().decompile();
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8(listJavaSources(fixture.getTargetDir()), rebuilt);
    try (URLClassLoader loader = new URLClassLoader(new URL[]{rebuilt.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      assertEquals(7, loader.loadClass("q.Readable").getMethod("run").invoke(null));
      assertNotNull(loader.loadClass("q.Readable$A$B"));
    }
  }

  @Test
  void nestedQualifierRepairRenamesItsEnclosingClass() throws Exception {
    lateClassRepairFixture(true);
  }

  @Test
  void lateClassRepairKeepsStandaloneMemberRenameComments() throws Exception {
    lateClassRepairFixture(false);
  }

  private void lateClassRepairFixture(boolean nested) throws Exception {
    compileJava8(List.of(
      writeSource("p/Outer.java", """
        package p;
        public class Outer {
          public static int value = 17;
          public static Outer oldField;
          public static Outer oldMethod() { return oldField; }
          public static class Inner { public static int value = 17; }
        }
        """),
      writeSource("q/Names.java", "package q; public interface Names {}"),
      writeSource("q/Use.java", "package q; public class Use implements Names { public static int read() { return p.Outer."
        + (nested ? "Inner." : "") + "value; } }")
    ), outRoot());
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve("q"));
    Files.move(outRoot().resolve("q/Names.class"), library.resolve("q/Names.class"));
    // These inherited fields obscure Java qualifiers but do not affect the original field reference.
    ClassWriter sdk = new ClassWriter(0);
    sdk.visit(Opcodes.V1_3, Opcodes.ACC_PUBLIC | Opcodes.ACC_INTERFACE | Opcodes.ACC_ABSTRACT,
      "q/Names", null, "java/lang/Object", null);
    sdk.visitField(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC | Opcodes.ACC_FINAL, "Outer", "I", null, 7).visitEnd();
    sdk.visitField(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC | Opcodes.ACC_FINAL, "p", "I", null, 9).visitEnd();
    sdk.visitEnd();
    Files.write(library.resolve("q/Names.class"), sdk.toByteArray());
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Path exported = fixture.getTempDir().resolve("names.tiny");
    Files.writeString(mapping, """
      tiny\t2\t0\tofficial\tnamed
      c\tp/Outer\tp/Outer
      \tf\tLp/Outer;\toldField\tcurrentField
      \tm\t()Lp/Outer;\toldMethod\tcurrentMethod
      """);
    builder(mapping).libraries(library.toFile()).option(IFernflowerPreferences.NAMING_OUTPUT, exported.toString()).build().decompile();
    NamingPlan names = NamingPlan.read(exported);
    String owner = names.classes().get("p/Outer");
    assertNotEquals("p/Outer", owner);
    assertEquals(owner + "$Inner", names.classes().get("p/Outer$Inner"));
    String source = Files.readString(fixture.getTargetDir().resolve(owner + ".java"));
    assertTrue(source.contains("renamed from: oldField"), source);
    assertTrue(source.contains("renamed from: oldMethod"), source);
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    var arguments = new ArrayList<>(List.of("--release", "8", "-classpath", library.toString(), "-d", rebuilt.toString()));
    listJavaSources(fixture.getTargetDir()).forEach(path -> arguments.add(path.toString()));
    assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null, arguments.toArray(String[]::new)));
    for (Path classes : new Path[]{outRoot(), rebuilt}) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL(), library.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        assertEquals(17, loader.loadClass("q.Use").getMethod("read").invoke(null));
      }
    }
  }

  @Test
  void keywordClassUsesTheSameDefaultPackageAsItsSiblings() throws Exception {
    Files.createDirectories(outRoot());
    for (String name : new String[]{"do", "Stable"}) {
      ClassWriter writer = new ClassWriter(0);
      writer.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, name, null, "java/lang/Object", null);
      writer.visitField(Opcodes.ACC_STATIC, "value", "I", null, null).visitEnd();
      writer.visitEnd();
      Files.write(outRoot().resolve(name + ".class"), writer.toByteArray());
    }
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\nc\tdo\tdo\nc\tStable\tStable\n");
    NamingPlan names = builder(request).option(IFernflowerPreferences.DEFAULT_PACKAGE, "defpackage").build().prepareNames();
    assertTrue(names.classes().get("do").startsWith("defpackage/class_"));
    assertEquals("defpackage/Stable", names.classes().get("Stable"));
    assertEquals(2, names.fields().size());
    Path prepared = fixture.getTempDir().resolve("prepared.tiny");
    names.write(prepared);
    builder(prepared).option(IFernflowerPreferences.MAPPINGS_PATH, "")
      .option(IFernflowerPreferences.PREPARED_NAMES_PATH, prepared.toString()).build().decompile();
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
  }

  @Test
  void renamedDebugDescriptorsAndGenericSignaturesCompile() throws Exception {
    compileJava8WithDebug(writeSource("p/Holder.java", """
      package p;
      import java.util.List;
      import java.util.ArrayList;
      public class Holder<T> {
        public class Nested<U> { }
        public List<Dir[]> entries;
        public Holder<String>.Nested<Dir[]> nested;
        public static Dir opposite(Dir dir) { dir = dir.opposite(); return dir; }
        public static Dir viaList(Dir dir) {
          List<Dir> values = new ArrayList<Dir>();
          values.add(dir);
          return values.get(0);
        }
      }
      class Dir { Dir opposite() { return this; } }
      """), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\nc\tp/Dir\tp/Direction\nc\tp/Holder$Nested\tp/Entry\n");
    builder(request).build().decompile();
    String source = Files.readString(fixture.getTargetDir().resolve("p/Holder.java"));
    assertTrue(source.contains("Direction"), source);
    assertFalse(source.contains("(Dir)"), source);
    assertFalse(source.contains("List<Dir["), source);
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
  }

  @Test
  void returnOnlyMethodsKeepDistinctBodiesAndExactInterfaceImplementation() throws Exception {
    ClassWriter contract = new ClassWriter(0);
    contract.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC | Opcodes.ACC_INTERFACE | Opcodes.ACC_ABSTRACT,
      "p/Contract", null, "java/lang/Object", null);
    contract.visitMethod(Opcodes.ACC_PUBLIC | Opcodes.ACC_ABSTRACT, "a", "()Ljava/lang/Object;", null, null).visitEnd();
    saveClass("p/Contract", contract);
    ClassWriter impl = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    impl.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "p/Impl", null, "java/lang/Object", new String[]{"p/Contract"});
    var ctor = impl.visitMethod(Opcodes.ACC_PUBLIC, "<init>", "()V", null, null);
    ctor.visitCode(); ctor.visitVarInsn(Opcodes.ALOAD, 0);
    ctor.visitMethodInsn(Opcodes.INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
    ctor.visitInsn(Opcodes.RETURN); ctor.visitMaxs(0, 0); ctor.visitEnd();
    for (String type : new String[]{"String", "Object"}) {
      var method = impl.visitMethod(Opcodes.ACC_PUBLIC, "a", "()Ljava/lang/" + type + ";", null, null);
      method.visitCode(); method.visitLdcInsn(type); method.visitInsn(Opcodes.ARETURN); method.visitMaxs(0, 0); method.visitEnd();
      var call = impl.visitMethod(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "call" + type, "()Ljava/lang/Object;", null, null);
      call.visitCode(); call.visitTypeInsn(Opcodes.NEW, "p/Impl"); call.visitInsn(Opcodes.DUP);
      call.visitMethodInsn(Opcodes.INVOKESPECIAL, "p/Impl", "<init>", "()V", false);
      boolean throughInterface = type.equals("Object");
      call.visitMethodInsn(throughInterface ? Opcodes.INVOKEINTERFACE : Opcodes.INVOKEVIRTUAL,
        throughInterface ? "p/Contract" : "p/Impl", "a", "()Ljava/lang/" + type + ";", throughInterface);
      call.visitInsn(Opcodes.ARETURN); call.visitMaxs(0, 0); call.visitEnd();
    }
    saveClass("p/Impl", impl);
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n");
    builder(mapping).build().decompile();
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8(listJavaSources(fixture.getTargetDir()), rebuilt);
    for (Path classes : new Path[]{outRoot(), rebuilt}) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        Class<?> type = loader.loadClass("p.Impl");
        assertEquals("String", type.getMethod("callString").invoke(null));
        assertEquals("Object", type.getMethod("callObject").invoke(null));
      }
    }
  }

  @Test
  void interfaceAndPackageFieldsCannotObscureAClassQualifier() throws Exception {
    qualifierFixture("p/", true, false);
  }

  @Test
  void unnamedClassQualifierGetsAConsistentFieldRepair() throws Exception {
    qualifierFixture("", false, false);
  }

  @Test
  void localCannotObscureTheRequiredPackageQualifier() throws Exception {
    qualifierFixture("p/", false, true);
  }

  @Test
  void visibleExternalFieldConflictRenamesTheOwnedClass() throws Exception {
    qualifierFixture("p/", true, false, true);
  }

  private void qualifierFixture(String pkg, boolean packageField, boolean local) throws Exception {
    qualifierFixture(pkg, packageField, local, false);
  }

  private void qualifierFixture(String pkg, boolean packageField, boolean local, boolean external) throws Exception {
    ClassWriter target = new ClassWriter(0);
    target.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, pkg + "Target", null, "java/lang/Object", null);
    target.visitField(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "value", "I", null, null).visitEnd();
    saveClass(pkg + "Target", target);
    ClassWriter names = new ClassWriter(0);
    names.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC | Opcodes.ACC_INTERFACE | Opcodes.ACC_ABSTRACT, pkg + "Names", null, "java/lang/Object", null);
    names.visitField(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC | Opcodes.ACC_FINAL, "Target", "I", null, 7).visitEnd();
    saveClass(pkg + "Names", names);
    ClassWriter use = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    use.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, pkg + "Use", null, "java/lang/Object", new String[]{pkg + "Names"});
    if (packageField) use.visitField(Opcodes.ACC_STATIC, "p", "Ljava/lang/Object;", null, null).visitEnd();
    var method = use.visitMethod(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "read", local ? "(Ljava/lang/Object;)I" : "()I", null, null);
    var start = new org.objectweb.asm.Label();
    var end = new org.objectweb.asm.Label();
    method.visitCode(); method.visitLabel(start); method.visitFieldInsn(Opcodes.GETSTATIC, pkg + "Target", "value", "I");
    method.visitInsn(Opcodes.IRETURN); method.visitLabel(end);
    if (local) method.visitLocalVariable("p", "Ljava/lang/Object;", null, start, end, 0);
    method.visitMaxs(0, 0); method.visitEnd();
    saveClass(pkg + "Use", use);
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library.resolve(pkg));
    if (external) Files.move(outRoot().resolve(pkg + "Names.class"), library.resolve(pkg + "Names.class"));
    NamingPlan plan = builder(request).libraries(library.toFile()).build().prepareNames();
    if (external) assertNotEquals(pkg + "Target", plan.classes().get(pkg + "Target"));
    else if (!local) assertNotEquals("Target", plan.fields().get(new NamingPlan.Member(pkg + "Names", "Target", "I")));
    Path prepared = fixture.getTempDir().resolve("prepared.tiny");
    plan.write(prepared);
    builder(prepared).libraries(library.toFile()).option(IFernflowerPreferences.MAPPINGS_PATH, "")
      .option(IFernflowerPreferences.PREPARED_NAMES_PATH, prepared.toString()).build().decompile();
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    Files.createDirectories(rebuilt);
    var arguments = new ArrayList<>(List.of("--release", "8", "-classpath", library.toString(), "-d", rebuilt.toString()));
    listJavaSources(fixture.getTargetDir()).forEach(path -> arguments.add(path.toString()));
    assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null, arguments.toArray(String[]::new)));
  }

  private void saveClass(String name, ClassWriter writer) throws Exception {
    writer.visitEnd();
    Path file = outRoot().resolve(name + ".class");
    Files.createDirectories(file.getParent());
    Files.write(file, writer.toByteArray());
  }

  @Test
  void malformedOptionalDebugTypeDoesNotPreventRenaming() throws Exception {
    ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    writer.visit(Opcodes.V1_8, Opcodes.ACC_PUBLIC, "BadDebug", null, "java/lang/Object", null);
    var method = writer.visitMethod(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "read", "()I", null, null);
    var start = new org.objectweb.asm.Label();
    var end = new org.objectweb.asm.Label();
    method.visitCode(); method.visitLabel(start); method.visitInsn(Opcodes.ICONST_1);
    method.visitInsn(Opcodes.IRETURN); method.visitLabel(end);
    method.visitLocalVariable("unused", "L;", null, start, end, 0);
    method.visitMaxs(0, 0); method.visitEnd();
    saveClass("BadDebug", writer);
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\nc\tBadDebug\tReadable\n");
    builder(mapping).build().decompile();
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
  }

  @Test
  void unrelatedMethodDoesNotRetainGenericBridge() throws Exception {
    compileJava8(writeSource("p/Subject.java", """
      package p;
      public class Subject implements Contract<String> {
        public String apply(String value) { return value; }
        public int unrelated(Object value) { return 7; }
        public static String run() { return ((Contract<String>)new Subject()).apply("ok"); }
      }
      interface Contract<T> { T apply(T value); }
      """), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    NamingPlan plan = builder(request).build().prepareNames();
    builder(request).option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan).build().decompile();
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8(listJavaSources(fixture.getTargetDir()), rebuilt);
    try (URLClassLoader loader = new URLClassLoader(new URL[]{rebuilt.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      assertEquals("ok", loader.loadClass("p.Subject").getMethod("run").invoke(null));
    }
  }

  @Test
  void unchangedAnonymousAndLocalClassesKeepTheirBinaryNames() throws Exception {
    compileJava8(writeSource("p/Outer.java", """
      package p;
      public class Outer {
        Object first() { class Local { } return new Local(); }
        Object second() { class Local { } return new Local(); }
        Object a = new Object() { };
        Object b = new Object() { };
      }
      """), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    NamingPlan plan = builder(request).build().prepareNames();
    plan.classes().forEach((original, emitted) -> assertEquals(original, emitted));
    builder(request).option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan).build().decompile();
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
  }

  @Test
  void parameterSlotsSurvivePreparedNamesTransport() throws Exception {
    compileJava8NoDebug(writeSource("p/Parameters.java", """
      package p;
      public class Parameters {
        public static long sum(long a, int b) { return a + b; }
        public long add(long a, int b) { return a + b; }
      }
      """), outRoot());
    Path request = fixture.getTempDir().resolve("parameters.tiny");
    Files.writeString(request, """
      tiny\t2\t0\tofficial\tnamed
      c\tp/Parameters\tp/Parameters
      \tm\t(JI)J\tsum\tsum
      \t\tp\t0\t\ttotal
      \t\tp\t2\t\tamount
      \tm\t(JI)J\tadd\tadd
      \t\tp\t1\t\ttotal
      \t\tp\t3\t\tamount
      """);
    NamingPlan plan = builder(request).build().prepareNames();
    Path prepared = fixture.getTempDir().resolve("prepared.tiny");
    plan.write(prepared);
    assertEquals(plan, NamingPlan.read(prepared));
    builder(prepared).option(IFernflowerPreferences.MAPPINGS_PATH, "")
      .option(IFernflowerPreferences.PREPARED_NAMES_PATH, prepared.toString()).build().decompile();
    String source = Files.readString(fixture.getTargetDir().resolve("p/Parameters.java"));
    assertTrue(source.contains("sum(long total, int amount)"), source);
    assertTrue(source.contains("add(long total, int amount)"), source);
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
  }

  @Test
  void privateExternalAncestorFieldsDoNotHideClassQualifiers() throws Exception {
    compileJava8(writeSource("p/Subject.java", """
      package p;
      public class Subject extends Base {
        public static int read() { return p.Target.value; }
      }
      class Target { static int value = 19; }
      class Base { private int Target; private int p; }
      """), outRoot());
    Path library = fixture.getTempDir().resolve("library/p");
    Files.createDirectories(library);
    Files.move(outRoot().resolve("p/Base.class"), library.resolve("Base.class"));
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    builder(request).libraries(library.getParent().toFile()).build().decompile();
    var sources = listJavaSources(fixture.getTargetDir());
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    Files.createDirectories(rebuilt);
    var arguments = new ArrayList<>(List.of("--release", "8", "-classpath", library.getParent().toString(), "-d", rebuilt.toString()));
    sources.forEach(path -> arguments.add(path.toString()));
    assertEquals(0, ToolProvider.getSystemJavaCompiler().run(null, null, null, arguments.toArray(String[]::new)));
    try (URLClassLoader loader = new URLClassLoader(new URL[]{rebuilt.toUri().toURL(), library.getParent().toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      assertEquals(19, loader.loadClass("p.Subject").getMethod("read").invoke(null));
    }
  }

  @Test
  void duplicateReservedNamesAreAccepted() throws Exception {
    compileJava8(writeSource("Subject.java", "public class Subject { }"), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    NamingPlan plan = builder(request).option(IFernflowerPreferences.RESERVED_CLASS_NAMES, " Subject,Subject,, ").build().prepareNames();
    assertNotEquals("Subject", plan.classes().get("Subject"));
  }

  @Test
  void ordinaryTokenConsumersDoNotSeeAnonymousBoundaryMarkers() throws Exception {
    compileJava8(writeSource("Subject.java", "public class Subject { Object value() { return new Object() {}; } }"), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    Decompiler decompiler = builder(request).build();
    List<Integer> lengths = new CopyOnWriteArrayList<>();
    TextTokenVisitor.addVisitor(next -> new TextTokenVisitor(next) {
      @Override public void visitClass(TextRange range, boolean declaration, String name) {
        lengths.add(range.length);
        super.visitClass(range, declaration, name);
      }
    });
    decompiler.decompile();
    assertFalse(lengths.isEmpty());
    assertTrue(lengths.stream().allMatch(length -> length > 0), lengths.toString());
  }

  @Test
  void preparedPlanValidatesMembersWithoutRejectingOverloads() throws Exception {
    compileJava8(writeSource("Subject.java", """
      public class Subject {
        int first; long second;
        int value(int x) { return x; }
        long value(long x) { return x; }
      }
      """), outRoot());
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
    NamingPlan plan = builder(request).build().prepareNames();
    builder(request).option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan).build().decompile();
    compileJava8(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("rebuilt"));
    var fields = new java.util.HashMap<>(plan.fields());
    fields.replaceAll((member, name) -> "same");
    NamingPlan invalid = new NamingPlan(plan.classes(), fields, plan.methods(), plan.parameters(), plan.innerNames());
    IllegalArgumentException failure = assertThrows(IllegalArgumentException.class, () ->
      builder(request).option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(invalid).build().decompile());
    assertTrue(failure.getMessage().contains("field names collide"), failure.getMessage());
  }

  @Test
  void forwardingBridgeWithExtraCastIsNotRegeneratedAsPlainCovariance() throws Exception {
    compileJava8(writeSource("p/CheckedBridge.java", """
      package p;
      public class CheckedBridge implements Provider {
        public String value(Object input) { return "ok"; }
        public static Object run(Object input) { return ((Provider)new CheckedBridge()).value(input); }
      }
      interface Provider { Object value(Object input); }
      """), outRoot());
    Path input = outRoot().resolve("p/CheckedBridge.class");
    var writer = new ClassWriter(0);
    new org.objectweb.asm.ClassReader(Files.readAllBytes(input)).accept(new org.objectweb.asm.ClassVisitor(Opcodes.ASM9, writer) {
      @Override public org.objectweb.asm.MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        var delegate = super.visitMethod(access, name, descriptor, signature, exceptions);
        if ((access & Opcodes.ACC_BRIDGE) == 0) return delegate;
        return new org.objectweb.asm.MethodVisitor(Opcodes.ASM9, delegate) {
          @Override public void visitMethodInsn(int opcode, String owner, String method, String desc, boolean isInterface) {
            super.visitTypeInsn(Opcodes.CHECKCAST, "java/lang/String");
            super.visitMethodInsn(opcode, owner, method, desc, isInterface);
          }
        };
      }
    }, 0);
    Files.write(input, writer.toByteArray());
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n");
    builder(mapping).build().decompile();
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8(listJavaSources(fixture.getTargetDir()), rebuilt);
    for (Path classes : new Path[]{outRoot(), rebuilt}) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        var method = loader.loadClass("p.CheckedBridge").getMethod("run", Object.class);
        assertEquals("ok", method.invoke(null, "text"));
        var failure = assertThrows(java.lang.reflect.InvocationTargetException.class, () -> method.invoke(null, new Object()));
        assertEquals(ClassCastException.class, failure.getCause().getClass());
      }
    }
  }

  @Test
  void diamondInheritanceDoesNotRenameTheSameFieldTwice() throws Exception {
    compileJava8(List.of(
      writeSource("q/Root.java", "package q; public interface Root { int shared = 7; }"),
      writeSource("p/Left.java", "package p; public interface Left extends q.Root { }"),
      writeSource("q/Right.java", "package q; public interface Right extends Root { }"),
      writeSource("q/Subject.java", "package q; public class Subject implements p.Left, Right { public int read() { return shared; } }")
    ), outRoot());
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n");
    NamingPlan plan = builder(mapping).build().prepareNames();
    assertEquals("shared", plan.fields().get(new NamingPlan.Member("q/Root", "shared", "I")));
  }

  @Test
  void genuineCovariantBridgeKeepsReadableModernSource() throws Exception {
    bridgeFixture();
  }

  private void bridgeFixture() throws Exception {
    compileJava8(writeSource("p/Bridge.java", """
      package p;
      public class Bridge implements Provider {
        public String value() { return "ok"; }
        public static Object through() { return ((Provider)new Bridge()).value(); }
        public static Object direct() { return new Bridge().value(); }
      }
      interface Provider { Object value(); }
      """), outRoot());
    Path mapping = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n");
    builder(mapping).build().decompile();
    String source = Files.readString(fixture.getTargetDir().resolve("p/Bridge.java"));
    assertTrue(source.contains("public String value()"), source);
    assertFalse(source.contains("public Object value()"), source);
    Path rebuilt = fixture.getTempDir().resolve("rebuilt");
    compileJava8(listJavaSources(fixture.getTargetDir()), rebuilt);
    for (Path classes : new Path[]{outRoot(), rebuilt}) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        Class<?> type = loader.loadClass("p.Bridge");
        assertEquals("ok", type.getMethod("through").invoke(null));
        assertEquals("ok", type.getMethod("direct").invoke(null));
      }
    }
  }
}
