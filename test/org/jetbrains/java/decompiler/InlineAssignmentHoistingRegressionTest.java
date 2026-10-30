package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.*;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;
import static org.objectweb.asm.Opcodes.*;

public class InlineAssignmentHoistingRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void conditionalAssignmentReusesReferenceSlot() throws Exception {
    Path source = writeSource("GuardedReference.java", """
      public class GuardedReference {
        public static GuardedReference next;
        public GuardedReference child;
        public String text;
        public static int reads;
        public String read() { reads++; return text; }
        public static boolean check() { return false; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    Path file = outRoot().resolve("GuardedReference.class");
    ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    new ClassReader(Files.readAllBytes(file)).accept(new ClassVisitor(ASM9, writer) {
      @Override
      public void visit(int version, int access, String name, String signature, String superName, String[] interfaces) {
        super.visit(V1_5, access, name, signature, superName, interfaces);
      }
      @Override
      public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        return name.equals("check") ? null : super.visitMethod(access, name, descriptor, signature, exceptions);
      }
    }, ClassReader.SKIP_FRAMES);
    MethodVisitor method = writer.visitMethod(ACC_PUBLIC | ACC_STATIC, "check", "()Z", null, null);
    method.visitCode();
    Label no = new Label(), yes = new Label(), end = new Label();
    // Preserve the original shape: DUP/ASTORE/IFNULL, redundant casts, and
    // reuse of slot zero for unrelated reference types inside nested guards.
    method.visitFieldInsn(GETSTATIC, "GuardedReference", "next", "LGuardedReference;");
    method.visitInsn(DUP);
    method.visitVarInsn(ASTORE, 0);
    method.visitJumpInsn(IFNULL, no);
    for (int i = 0; i < 2; i++) {
      method.visitVarInsn(ALOAD, 0);
      method.visitTypeInsn(CHECKCAST, "GuardedReference");
      method.visitTypeInsn(CHECKCAST, "GuardedReference");
      method.visitFieldInsn(GETFIELD, "GuardedReference", "child", "LGuardedReference;");
      if (i == 0) method.visitJumpInsn(IFNULL, no);
    }
    method.visitMethodInsn(INVOKEVIRTUAL, "GuardedReference", "read", "()Ljava/lang/String;", false);
    method.visitInsn(DUP);
    method.visitVarInsn(ASTORE, 0);
    method.visitJumpInsn(IFNULL, yes);
    method.visitVarInsn(ALOAD, 0);
    method.visitTypeInsn(CHECKCAST, "java/lang/String");
    method.visitTypeInsn(CHECKCAST, "java/lang/String");
    method.visitLdcInsn("blocked:");
    method.visitMethodInsn(INVOKEVIRTUAL, "java/lang/String", "startsWith", "(Ljava/lang/String;)Z", false);
    method.visitJumpInsn(IFNE, no);
    method.visitLabel(yes);
    method.visitInsn(ICONST_1);
    method.visitJumpInsn(GOTO, end);
    method.visitLabel(no);
    method.visitInsn(ICONST_0);
    method.visitLabel(end);
    method.visitInsn(IRETURN);
    method.visitMaxs(0, 0);
    method.visitEnd();
    Files.write(file, writer.toByteArray());
    checkGuard(outRoot());
    decompileDirectory(outRoot(), "GuardedReference.java");
    recompile();
    checkGuard(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkGuard(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("GuardedReference");
      for (int state = 0; state < 5; state++) {
        Object page = type.getConstructor().newInstance();
        Object child = type.getConstructor().newInstance();
        type.getField("next").set(null, state == 0 ? null : page);
        type.getField("child").set(page, state == 1 ? null : child);
        type.getField("text").set(child, state == 2 ? null : state == 3 ? "allowed" : "blocked:item");
        type.getField("reads").setInt(null, 0);
        assertEquals(state == 2 || state == 3, type.getMethod("check").invoke(null), "state " + state);
        assertEquals(state < 2 ? 0 : 1, type.getField("reads").getInt(null), "guarded read count");
      }
    }
  }

  @ParameterizedTest
  @ValueSource(strings = {
    "probe() & ((value = read()) != null)",
    "probe() | ((value = read()) != null)",
    "probe() && ((value = read()) != null)",
    "probe() || ((value = read()) != null)",
    "probe() ? ((value = read()) != null) : false",
    "probe() ? true : ((value = read()) != null)"
  })
  public void preservesOperandEvaluationOrder(String expression) throws Exception {
    Path source = writeSource("OrderedAssignment.java", """
      public class OrderedAssignment {
        public static String trace;
        public static boolean flag;
        static boolean probe() { trace += "p"; return flag; }
        static Object read() { trace += "r"; return "new"; }
        public static String check(boolean input) {
          trace = ""; flag = input;
          Object value = "old";
          if (%s) { trace += "t"; }
          return trace + ":" + value;
        }
      }
      """.formatted(expression));
    compileJava8NoDebug(source, outRoot());
    String[] expected = observe(outRoot());
    decompileDirectory(outRoot(), "OrderedAssignment.java");
    recompile();
    assertArrayEquals(expected, observe(fixture.getTempDir().resolve("recompiled-out")), expression);
  }

  @Test
  public void independentPrefixesKeepAssignmentsReadable() throws Exception {
    Path source = writeSource("ReadableAssignment.java", """
      public class ReadableAssignment {
        public static int output;
        public int field;
        public static int reads;
        static int[] load() { reads++; return new int[]{3, 7}; }
        public static int staticStore() {
          int[] values;
          output = (values = load())[0];
          return output + values[1];
        }
        public int instanceStore() {
          int[] values;
          field = (values = load())[0];
          return field + values[1];
        }
        public static int literalPrefix() {
          int[] values;
          int result = 55 + (values = load())[0];
          return result + values[1];
        }
        public static int arithmeticPrefix(int input) {
          int[] values;
          int result = input * input + (values = load())[0];
          return result + values[1];
        }
        public static int localPrefix(int input) {
          int[] values;
          int result = input + (values = load())[0];
          return result + values[1];
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    checkReadable(outRoot());
    String result = decompileDirectory(outRoot(), "ReadableAssignment.java");
    // All five independent prefixes should retain an initialized declaration,
    // instead of burying the assignment inside an array access.
    assertEquals(5, result.split(" = load\\(\\);", -1).length - 1, result);
    recompile();
    checkReadable(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkReadable(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("ReadableAssignment");
      Object receiver = type.getConstructor().newInstance();
      assertEquals(10, type.getMethod("staticStore").invoke(null));
      assertEquals(10, type.getMethod("instanceStore").invoke(receiver));
      assertEquals(65, type.getMethod("literalPrefix").invoke(null));
      assertEquals(15, type.getMethod("localPrefix", int.class).invoke(null, 5));
      assertEquals(35, type.getMethod("arithmeticPrefix", int.class).invoke(null, 5));
      assertEquals(5, type.getField("reads").getInt(null));
    }
  }

  @Test
  public void earlierValuesAndStoreTargetsSurviveLaterWrites() throws Exception {
    Path source = writeSource("CapturedTarget.java", """
      public class CapturedTarget {
        public static int[] nextArray;
        public CapturedTarget next;
        public boolean done;
        public static int length(int[] a) {
          int result = a.length + ((a = nextArray) == null ? 0 : 1);
          return result;
        }
        public static int index(int[] a) {
          int result = a[(a = nextArray) == null ? 0 : 1];
          return result;
        }
        public static void arrayStore(int[] a) {
          a[0] = (a = nextArray) == null ? 3 : 4;
        }
        public static void fieldStore(CapturedTarget p) {
          p.done = (p = p.next) == null;
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    checkTargets(outRoot());
    decompileDirectory(outRoot(), "CapturedTarget.java");
    recompile();
    checkTargets(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkTargets(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("CapturedTarget");
      for (boolean absent : new boolean[]{false, true}) {
        int[] replacement = absent ? null : new int[]{90, 91, 92};
        type.getField("nextArray").set(null, replacement);
        assertEquals(absent ? 2 : 3, type.getMethod("length", int[].class).invoke(null, (Object)new int[]{10, 11}));
        assertEquals(absent ? 10 : 11, type.getMethod("index", int[].class).invoke(null, (Object)new int[]{10, 11}));
        int[] original = {10, 11};
        type.getMethod("arrayStore", int[].class).invoke(null, (Object)original);
        assertEquals(absent ? 3 : 4, original[0]);
        if (!absent) assertArrayEquals(new int[]{90, 91, 92}, replacement);
        Object first = type.getConstructor().newInstance();
        Object next = type.getConstructor().newInstance();
        type.getField("next").set(first, absent ? null : next);
        type.getField("done").setBoolean(first, !absent);
        type.getField("done").setBoolean(next, true);
        type.getMethod("fieldStore", type).invoke(null, first);
        assertEquals(absent, type.getField("done").getBoolean(first));
        assertTrue(type.getField("done").getBoolean(next));
      }
    }
  }

  private static String[] observe(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("OrderedAssignment");
      String[] results = new String[2];
      for (int i = 0; i < 2; i++) {
        results[i] = type.getMethod("check", boolean.class).invoke(null, i != 0) + ":" + type.getField("trace").get(null);
      }
      return results;
    }
  }
}
