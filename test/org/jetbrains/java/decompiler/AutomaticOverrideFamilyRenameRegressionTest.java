package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.Arguments;
import org.junit.jupiter.params.provider.MethodSource;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class AutomaticOverrideFamilyRenameRegressionTest extends DecompileRegressionTestBase implements Opcodes {
  private enum Feature { REVERSED, INHERITED, OCCUPIED, FINAL_BLOCKER, MULTIPLE_SCOPES, DELAYED_COLLISION }

  private enum Shape {
    PLAIN(14),
    REVERSED(14, Feature.REVERSED),
    INHERITED(14, Feature.INHERITED),
    OCCUPIED(33, Feature.OCCUPIED),
    FINAL_BLOCKER(33, Feature.FINAL_BLOCKER),
    MULTIPLE_SCOPES(78, Feature.FINAL_BLOCKER, Feature.MULTIPLE_SCOPES),
    DELAYED_COLLISION(78, Feature.FINAL_BLOCKER, Feature.MULTIPLE_SCOPES, Feature.DELAYED_COLLISION);

    final int expected;
    final Set<Feature> features;

    Shape(int expected, Feature... features) {
      this.expected = expected;
      this.features = Set.of(features);
    }

    boolean has(Feature feature) {
      return features.contains(feature);
    }
  }

  private enum Mode { AUTOMATIC, MAPPED, PREPARED }

  static Stream<Arguments> families() {
    return Stream.of(Shape.values())
      .flatMap(shape -> Stream.of(Mode.values()).map(mode -> Arguments.of(shape, mode)));
  }

  @ParameterizedTest(name = "{0}: {1}")
  @MethodSource("families")
  void sharedImplementationPreservesDispatch(Shape shape, Mode mode) throws Exception {
    writeFamily(shape);
    assertEquals(shape.expected, invoke(outRoot()));
    Decompiler.Builder builder = builder();
    if (mode == Mode.MAPPED) {
      String owner = shape.has(Feature.INHERITED) ? "Parent" : "Implementation";
      builder.option(IFernflowerPreferences.MAPPINGS_PATH,
        mapping(owner, "if", shape.has(Feature.DELAYED_COLLISION) ? "shared" : "method_0").toString());
    }
    if (mode == Mode.PREPARED) builder.preparedNames(builder.build().prepareNames());
    builder.build().decompile();
    recompile();
    // Compilation alone missed the original bug: the second interface method
    // was reconstructed as a concrete stub throwing AbstractMethodError.
    assertEquals(shape.expected, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  private enum SharedType {
    INTERFACE("public interface Both extends FirstFace, SecondFace {}", "implements"),
    ABSTRACT_CLASS("public abstract class Both implements FirstFace, SecondFace {}", "extends");

    final String source;
    final String implementationRelation;

    SharedType(String source, String implementationRelation) {
      this.source = source;
      this.implementationRelation = implementationRelation;
    }
  }

  static Stream<Arguments> unimplementedFamilies() {
    return Stream.of(SharedType.values()).flatMap(type -> Stream.of(false, true)
      .flatMap(library -> Stream.of(false, true).map(mapped -> Arguments.of(type, library, mapped))));
  }

  @ParameterizedTest(name = "{0}: library={1}, mapped={2}")
  @MethodSource("unimplementedFamilies")
  void inheritedInterfaceContractsShareANameWithoutAnImplementation(SharedType type, boolean library, boolean mapped) throws Exception {
    Path second = writeSource("SecondFace.java", "public interface SecondFace { int value(); }");
    Path implementation = writeSource("Implementation.java", "public class Implementation " + type.implementationRelation
      + " Both { public int value() { return 7; } }");
    Path probe = writeSource("Probe.java", """
      public class Probe {
        public static int run() {
          Both instance = new Implementation();
          return ((FirstFace)instance).value() + ((SecondFace)instance).value();
        }
      }
      """);
    compileJava8NoDebug(List.of(
      writeSource("FirstFace.java", "public interface FirstFace { int value(); }"),
      second, writeSource("Both.java", type.source), implementation, probe
    ), outRoot());
    assertEquals(14, invoke(outRoot()));

    // The decompiler must join the contracts without seeing a concrete method.
    Path withheld = Files.createDirectory(fixture.getTempDir().resolve("withheld"));
    for (String name : List.of("Implementation", "Probe")) {
      Files.move(outRoot().resolve(name + ".class"), withheld.resolve(name + ".class"));
    }
    Decompiler.Builder builder = builder();
    if (library) {
      Path libraries = Files.createDirectory(fixture.getTempDir().resolve("library"));
      Files.move(outRoot().resolve("SecondFace.class"), libraries.resolve("SecondFace.class"));
      builder.libraries(libraries.toFile());
    }
    if (mapped) builder.option(IFernflowerPreferences.MAPPINGS_PATH, mapping("FirstFace", "value", "renamed").toString());
    builder.build().decompile();

    String expectedName = mapped && !library ? "renamed" : "value";
    Files.writeString(implementation, Files.readString(implementation).replace("value()", expectedName + "()"));
    Files.writeString(probe, Files.readString(probe).replace("value()", expectedName + "()"));
    recompile(library ? List.of(second, implementation, probe) : List.of(implementation, probe));
    Path rebuilt = fixture.getTempDir().resolve("recompiled-out");
    try (URLClassLoader loader = loader(rebuilt)) {
      for (String name : List.of("FirstFace", "SecondFace")) {
        assertEquals(int.class, loader.loadClass(name).getDeclaredMethod(expectedName).getReturnType());
      }
    }
    assertEquals(14, invoke(rebuilt));
  }

  @Test
  void unrelatedInterfaceContractsKeepIndependentNames() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("FirstFace.java", "public interface FirstFace { int value(); }"),
      writeSource("SecondFace.java", "public interface SecondFace { int value(); }")
    ), outRoot());
    builder().option(IFernflowerPreferences.MAPPINGS_PATH, mapping("FirstFace", "value", "renamed").toString())
      .build().decompile();
    recompile();
    try (URLClassLoader loader = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      assertEquals(int.class, loader.loadClass("FirstFace").getDeclaredMethod("renamed").getReturnType());
      assertEquals(int.class, loader.loadClass("SecondFace").getDeclaredMethod("value").getReturnType());
    }
  }

  @Test
  void inheritedReturnOnlyContractsRemainSeparate() throws Exception {
    for (String name : List.of("FirstFace", "SecondFace")) {
      ClassWriter face = clazz(name, ACC_PUBLIC | ACC_INTERFACE | ACC_ABSTRACT, "java/lang/Object");
      face.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, "value", name.equals("FirstFace") ? "()I" : "()J", null, null).visitEnd();
      save(name, face);
    }
    save("Both", clazz("Both", ACC_PUBLIC | ACC_ABSTRACT, "java/lang/Object", "FirstFace", "SecondFace"));
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertNotEquals(plan.methods().get(new NamingPlan.Member("FirstFace", "value", "()I")),
      plan.methods().get(new NamingPlan.Member("SecondFace", "value", "()J")));
    builder.preparedNames(plan).build().decompile();
    recompile();
  }

  @Test
  void libraryInterfacePinsTheWholeMappedFamily() throws Exception {
    Path contract = writeSource("Contract.java", "public interface Contract { int value(); }");
    compileJava8NoDebug(List.of(contract, writeSource("Implementation.java", """
      public class Implementation implements Contract {
        public int value() { return 7; }
      }
      """), writeSource("Probe.java", """
      public class Probe {
        public static int run() { return ((Contract)new Implementation()).value(); }
      }
      """)), outRoot());
    assertEquals(7, invoke(outRoot()));
    Path library = fixture.getTempDir().resolve("library");
    Files.createDirectories(library);
    Files.move(outRoot().resolve("Contract.class"), library.resolve("Contract.class"));
    builder().libraries(library.toFile())
      .option(IFernflowerPreferences.MAPPINGS_PATH, mapping("Implementation", "value", "renamed").toString())
      .build().decompile();
    recompile(List.of(contract));
    assertEquals(7, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  @Test
  void nativeImplementationPinsItsInterfaceName() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("Contract.java", "public interface Contract { int value(); }"),
      writeSource("Implementation.java", "public class Implementation implements Contract { public native int value(); }")
    ), outRoot());
    builder().option(IFernflowerPreferences.MAPPINGS_PATH, mapping("Contract", "value", "renamed").toString())
      .build().decompile();
    recompile();
    try (URLClassLoader loader = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      assertEquals(int.class, loader.loadClass("Contract").getDeclaredMethod("value").getReturnType());
      var implementation = loader.loadClass("Implementation");
      assertEquals(1, Stream.of(implementation.getDeclaredMethods()).filter(method -> !method.isSynthetic()).count());
      assertTrue(java.lang.reflect.Modifier.isNative(implementation.getDeclaredMethod("value").getModifiers()));
    }
  }

  private Decompiler.Builder builder() {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, true)
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private Path mapping(String owner, String from, String to) throws Exception {
    return Files.writeString(fixture.getTempDir().resolve("requested.tiny"),
      "tiny\t2\t0\tofficial\tnamed\nc\t" + owner + "\t" + owner + "\n\tm\t()I\t" + from + "\t" + to + "\n");
  }

  private void writeFamily(Shape shape) throws Exception {
    boolean inherited = shape.has(Feature.INHERITED);
    boolean occupied = shape.has(Feature.OCCUPIED);
    boolean multiple = shape.has(Feature.MULTIPLE_SCOPES);
    String otherName = shape.has(Feature.DELAYED_COLLISION) ? "shared" : "method_1";
    boolean blocker = shape.has(Feature.FINAL_BLOCKER);
    for (String name : List.of("FirstFace", "SecondFace")) {
      ClassWriter face = clazz(name, ACC_PUBLIC | ACC_INTERFACE | ACC_ABSTRACT, "java/lang/Object");
      face.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, "if", "()I", null, null).visitEnd();
      if (occupied && name.equals("SecondFace")) face.visitMethod(ACC_PUBLIC | ACC_ABSTRACT, "method_0", "()I", null, null).visitEnd();
      save(name, face);
    }
    if (inherited || blocker) {
      ClassWriter parent = clazz("Parent", ACC_PUBLIC, "java/lang/Object");
      if (inherited) constant(parent, ACC_PUBLIC, "if", 7);
      if (blocker) constant(parent, ACC_PUBLIC | ACC_FINAL, "method_0", 19);
      save("Parent", parent);
    }
    String[] faces = shape.has(Feature.REVERSED) ? new String[]{"SecondFace", "FirstFace"} : new String[]{"FirstFace", "SecondFace"};
    ClassWriter impl = clazz("Implementation", ACC_PUBLIC, inherited || blocker ? "Parent" : "java/lang/Object", faces);
    if (!inherited) constant(impl, ACC_PUBLIC, "if", 7);
    if (occupied) constant(impl, ACC_PUBLIC, "method_0", 19);
    save("Implementation", impl);
    if (multiple) {
      ClassWriter parent = clazz("OtherParent", ACC_PUBLIC, "java/lang/Object");
      // A repair forced in this scope must also respect the other parent's
      // method_0, even if that earlier scope had no collision initially.
      constant(parent, ACC_PUBLIC | ACC_FINAL, otherName, 23);
      save("OtherParent", parent);
      ClassWriter other = clazz("OtherImplementation", ACC_PUBLIC, "OtherParent", faces);
      constant(other, ACC_PUBLIC, "if", 11);
      save("OtherImplementation", other);
    }
    ClassWriter probe = clazz("Probe", ACC_PUBLIC, "java/lang/Object");
    MethodVisitor m = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    m.visitCode();
    interfaceCalls(m, "Implementation");
    if (occupied || blocker) {
      m.visitVarInsn(ALOAD, 0);
      m.visitMethodInsn(occupied ? INVOKEINTERFACE : INVOKEVIRTUAL, occupied ? "SecondFace" : "Parent", "method_0", "()I", occupied);
      m.visitInsn(IADD);
    }
    if (multiple) {
      interfaceCalls(m, "OtherImplementation");
      m.visitInsn(IADD);
      m.visitVarInsn(ALOAD, 0);
      m.visitMethodInsn(INVOKEVIRTUAL, "OtherParent", otherName, "()I", false);
      m.visitInsn(IADD);
    }
    m.visitInsn(IRETURN);
    m.visitMaxs(0, 0);
    m.visitEnd();
    save("Probe", probe);
  }

  private static void interfaceCalls(MethodVisitor m, String implementation) {
    m.visitTypeInsn(NEW, implementation);
    m.visitInsn(DUP);
    m.visitMethodInsn(INVOKESPECIAL, implementation, "<init>", "()V", false);
    m.visitVarInsn(ASTORE, 0);
    for (String face : List.of("FirstFace", "SecondFace")) {
      m.visitVarInsn(ALOAD, 0);
      m.visitMethodInsn(INVOKEINTERFACE, face, "if", "()I", true);
    }
    m.visitInsn(IADD);
  }

  private static ClassWriter clazz(String name, int flags, String parent, String... faces) {
    ClassWriter c = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    // Java 1.3 bytecode: keyword method names are valid JVM names, and no
    // bridges/default interface methods are needed to trigger this failure.
    c.visit(V1_3, flags, name, null, parent, faces);
    if ((flags & ACC_INTERFACE) == 0) {
      MethodVisitor m = c.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
      m.visitCode();
      m.visitVarInsn(ALOAD, 0);
      m.visitMethodInsn(INVOKESPECIAL, parent, "<init>", "()V", false);
      m.visitInsn(RETURN);
      m.visitMaxs(0, 0);
      m.visitEnd();
    }
    return c;
  }

  private static void constant(ClassWriter c, int flags, String name, int value) {
    MethodVisitor m = c.visitMethod(flags, name, "()I", null, null);
    m.visitCode();
    m.visitLdcInsn(value);
    m.visitInsn(IRETURN);
    m.visitMaxs(0, 0);
    m.visitEnd();
  }

  private void save(String name, ClassWriter writer) throws Exception {
    writer.visitEnd();
    Files.createDirectories(outRoot());
    Files.write(outRoot().resolve(name + ".class"), writer.toByteArray());
  }

  private static URLClassLoader loader(Path root) throws Exception {
    return new URLClassLoader(new URL[]{root.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static Object invoke(Path root) throws Exception {
    try (URLClassLoader loader = loader(root)) {
      return loader.loadClass("Probe").getMethod("run").invoke(null);
    }
  }
}
