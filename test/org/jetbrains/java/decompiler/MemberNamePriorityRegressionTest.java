package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.EnumSource;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;

class MemberNamePriorityRegressionTest extends DecompileRegressionTestBase implements Opcodes {
  private enum Mode { AUTOMATIC, TINY, PREPARED }

  @ParameterizedTest
  @EnumSource(Mode.class)
  void generatedMethodCannotTakeAnOriginalName(Mode mode) throws Exception {
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", null);
    constantMethod(probe, "if", 11);
    constantMethod(probe, "method_0", 22);
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitMethodInsn(INVOKESTATIC, "p/Probe", "if", "()I", false);
    run.visitMethodInsn(INVOKESTATIC, "p/Probe", "method_0", "()I", false);
    run.visitInsn(IADD);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    save(probe);
    Decompiler.Builder builder = builder(mode);
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("method_0", plan.methods().get(new NamingPlan.Member("p/Probe", "method_0", "()I")));
    assertNotEquals("method_0", plan.methods().get(new NamingPlan.Member("p/Probe", "if", "()I")));
    rebuild(builder, plan, mode, 33);
  }

  @ParameterizedTest
  @EnumSource(Mode.class)
  void generatedClassFieldCannotTakeAnInterfaceName(Mode mode) throws Exception {
    compileJava8NoDebug(writeSource("p/Names.java", "package p; public interface Names { int field_0 = Integer.parseInt(\"22\"); }"), outRoot());
    ClassWriter probe = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    probe.visit(V1_3, ACC_PUBLIC, "p/Probe", null, "java/lang/Object", new String[]{"p/Names"});
    probe.visitField(ACC_PUBLIC | ACC_STATIC | ACC_FINAL, "if", "I", null, 11).visitEnd();
    var run = probe.visitMethod(ACC_PUBLIC | ACC_STATIC, "run", "()I", null, null);
    run.visitCode();
    run.visitFieldInsn(GETSTATIC, "p/Probe", "if", "I");
    run.visitFieldInsn(GETSTATIC, "p/Names", "field_0", "I");
    run.visitInsn(IADD);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    save(probe);
    Decompiler.Builder builder = builder(mode);
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("field_0", plan.fields().get(new NamingPlan.Member("p/Names", "field_0", "I")));
    assertNotEquals("field_0", plan.fields().get(new NamingPlan.Member("p/Probe", "if", "I")));
    rebuild(builder, plan, mode, 33);
  }

  @ParameterizedTest
  @EnumSource(value = Mode.class, names = {"TINY", "PREPARED"})
  void explicitMappedNameHasTheSamePriorityAsAnOriginalName(Mode mode) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Probe.java", """
        package p;
        public class Probe {
          public static int first = 11;
          public static int second = 22;
          public static int run() { return first + second; }
        }
        """)
    ), outRoot());
    Decompiler.Builder builder = builder(Mode.TINY);
    Path request = fixture.getTempDir().resolve("requested.tiny");
    Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\nc\tp/Probe\tp/Probe\n\tf\tI\tfirst\tsecond\n");
    NamingPlan plan = builder.build().prepareNames();
    // Equal-priority requests use declaration order, not whether the spelling changed.
    assertEquals("second", plan.fields().get(new NamingPlan.Member("p/Probe", "first", "I")));
    assertNotEquals("second", plan.fields().get(new NamingPlan.Member("p/Probe", "second", "I")));
    rebuild(builder, plan, mode, 33);
  }

  private Decompiler.Builder builder(Mode mode) throws Exception {
    Decompiler.Builder builder = Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
    if (mode == Mode.TINY) {
      Path request = fixture.getTempDir().resolve("requested.tiny");
      Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\n");
      builder.option(IFernflowerPreferences.MAPPINGS_PATH, request.toString());
    }
    return builder;
  }

  private void rebuild(Decompiler.Builder builder, NamingPlan plan, Mode mode, int expected) throws Exception {
    assertEquals(expected, invoke(outRoot()));
    if (mode == Mode.PREPARED) builder.option(IFernflowerPreferences.MAPPINGS_PATH, "").preparedNames(plan);
    builder.build().decompile();
    recompile();
    assertEquals(expected, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  private void save(ClassWriter writer) throws Exception {
    writer.visitEnd();
    Files.createDirectories(outRoot().resolve("p"));
    Files.write(outRoot().resolve("p/Probe.class"), writer.toByteArray());
  }

  private static void constantMethod(ClassWriter owner, String name, int value) {
    var method = owner.visitMethod(ACC_PUBLIC | ACC_STATIC, name, "()I", null, null);
    method.visitCode();
    method.visitLdcInsn(value);
    method.visitInsn(IRETURN);
    method.visitMaxs(0, 0);
    method.visitEnd();
  }

  private static Object invoke(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return loader.loadClass("p.Probe").getMethod("run").invoke(null);
    }
  }
}
