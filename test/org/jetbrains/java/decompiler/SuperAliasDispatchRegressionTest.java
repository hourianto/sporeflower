package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.EnumSource;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.objectweb.asm.Opcodes.*;

public class SuperAliasDispatchRegressionTest extends DecompileRegressionTestBase {
  private static final String BASE = "pkg/DispatchBase";
  private static final String CHILD = "pkg/DispatchChild";

  private enum Shape {
    STORED_ALIAS, STACK_SAVED_RECEIVER, MIXED_LATER_LIFETIME
  }

  @ParameterizedTest(name = "super dispatch through {0}")
  @EnumSource(Shape.class)
  void preservesOriginalBehavior(Shape shape) throws Exception {
    Path base = writeSource(BASE + ".java", """
      package pkg;
      public class DispatchBase {
        public int marker = 7;
        public static int baseCalls, childCalls, argumentEffects, overwriteEffects, trace;
        public static void mark() { trace = trace * 10 + 1; }
        public static void observeInt(int value) { overwriteEffects += value + 1; }
        public static void after(Object value) {
          overwriteEffects += value == null ? 10 : 20;
          trace = trace * 10 + 4;
        }
        public static int argument(int value) {
          argumentEffects++;
          trace = trace * 10 + 2;
          return value;
        }
        public int update(int value) {
          baseCalls++;
          trace = trace * 10 + 3;
          return value * 10 + 7;
        }
      }
      """);
    compileJava8NoDebug(base, outRoot());
    Files.write(outRoot().resolve(CHILD + ".class"), child(shape));

    int[] inputs = {0, 1};
    for (int input : inputs) {
      assertEquals(expected(shape, input), observe(outRoot(), input), "original " + shape + " / " + input);
    }

    assertInvocation(outRoot(), CHILD, "update", "update", "(I)I", INVOKESPECIAL);
    decompileDirectory(outRoot(), CHILD + ".java");
    recompile();
    Path rebuilt = fixture.getTempDir().resolve("recompiled-out");
    for (int input : inputs) {
      String before = observe(outRoot(), input);
      String after = observe(rebuilt, input);
      assertEquals(before, after, shape + " / " + input);
    }
    assertInvocation(rebuilt, CHILD, "update", "update", "(I)I", INVOKESPECIAL);
  }

  private static String expected(Shape shape, int input) {
    if (shape == Shape.MIXED_LATER_LIFETIME) {
      return "result=" + (input * 10 + 7) + ",base=1,child=1,args=1,overwrites=" + (input == 0 ? 10 : 20) + ",trace=1234";
    }
    // Observe both the overwritten int and restored marker.
    int overwrites = shape == Shape.STACK_SAVED_RECEIVER ? input + 9 : 0;
    return "result=" + (input * 10 + 7) + ",base=1,child=1,args=1,overwrites=" + overwrites + ",trace=123";
  }

  private static byte[] child(Shape shape) {
    // Version 46 mirrors old J2ME bytecode and lets the JVM verify slot-zero
    // reassignment without fabricated StackMapTable information.
    ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    writer.visit(V1_2, ACC_PUBLIC | ACC_SUPER, CHILD, null, BASE, null);
    MethodVisitor init = writer.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
    init.visitCode();
    init.visitVarInsn(ALOAD, 0);
    init.visitMethodInsn(INVOKESPECIAL, BASE, "<init>", "()V", false);
    init.visitInsn(RETURN);
    init.visitMaxs(0, 0);
    init.visitEnd();

    MethodVisitor method = writer.visitMethod(ACC_PUBLIC, "update", "(I)I", null, null);
    method.visitCode();
    // A bounded re-entry guard makes virtual redispatch a deterministic failure
    // without depending on the VM's thread stack limit.
    method.visitFieldInsn(GETSTATIC, BASE, "childCalls", "I");
    method.visitInsn(ICONST_1);
    method.visitInsn(IADD);
    method.visitFieldInsn(PUTSTATIC, BASE, "childCalls", "I");
    method.visitFieldInsn(GETSTATIC, BASE, "childCalls", "I");
    method.visitInsn(ICONST_1);
    Label firstCall = new Label();
    method.visitJumpInsn(IF_ICMPEQ, firstCall);
    method.visitTypeInsn(NEW, "java/lang/IllegalStateException");
    method.visitInsn(DUP);
    method.visitMethodInsn(INVOKESPECIAL, "java/lang/IllegalStateException", "<init>", "()V", false);
    method.visitInsn(ATHROW);
    method.visitLabel(firstCall);

    if (shape == Shape.STORED_ALIAS) {
      // Move the int argument before reusing its parameter slot for this.
      method.visitVarInsn(ALOAD, 0);
      method.visitVarInsn(ILOAD, 1);
      method.visitVarInsn(ISTORE, 2);
      method.visitVarInsn(ASTORE, 1);
    } else if (shape != Shape.STACK_SAVED_RECEIVER) {
      method.visitVarInsn(ALOAD, 0);
      method.visitVarInsn(ASTORE, 2);
    }
    method.visitMethodInsn(INVOKESTATIC, BASE, "mark", "()V", false);
    if (shape == Shape.STACK_SAVED_RECEIVER) {
      // Keep this on the operand stack while slot zero holds an int, then
      // restore it with DUP/ASTORE_0. The retained stack copy feeds a field
      // read, while the later special call loads the restored slot zero.
      method.visitVarInsn(ALOAD, 0);
      method.visitVarInsn(ILOAD, 1);
      method.visitVarInsn(ISTORE, 0);
      method.visitVarInsn(ILOAD, 0);
      method.visitMethodInsn(INVOKESTATIC, BASE, "observeInt", "(I)V", false);
      method.visitInsn(DUP);
      method.visitVarInsn(ASTORE, 0);
      method.visitFieldInsn(GETFIELD, BASE, "marker", "I");
      method.visitMethodInsn(INVOKESTATIC, BASE, "observeInt", "(I)V", false);
    }
    int receiverSlot = shape == Shape.STORED_ALIAS ? 1 : shape == Shape.STACK_SAVED_RECEIVER ? 0 : 2;
    method.visitVarInsn(ALOAD, receiverSlot);
    method.visitVarInsn(ILOAD, shape == Shape.STORED_ALIAS ? 2 : 1);
    method.visitMethodInsn(INVOKESTATIC, BASE, "argument", "(I)I", false);
    method.visitMethodInsn(INVOKESPECIAL, BASE, "update", "(I)I", false);
    if (shape == Shape.MIXED_LATER_LIFETIME) {
      // The call receiver is this at this instruction, but a later phi joins
      // its definition with null. Per-local proof after phi collapse loses
      // the valid identity proof for the earlier call.
      method.visitVarInsn(ISTORE, 3);
      method.visitVarInsn(ILOAD, 1);
      Label retained = new Label();
      method.visitJumpInsn(IFNE, retained);
      method.visitInsn(ACONST_NULL);
      method.visitVarInsn(ASTORE, 2);
      method.visitLabel(retained);
      method.visitVarInsn(ALOAD, 2);
      method.visitMethodInsn(INVOKESTATIC, BASE, "after", "(Ljava/lang/Object;)V", false);
      method.visitVarInsn(ILOAD, 3);
    }
    method.visitInsn(IRETURN);
    method.visitMaxs(0, 0);
    method.visitEnd();
    writer.visitEnd();
    return writer.toByteArray();
  }

  @Test
  void virtualAliasRemainsVirtual() throws Exception {
    Path source = writeSource("pkg/VirtualAlias.java", """
      package pkg;
      class VirtualAliasBase { public int target() { return 10; } }
      public class VirtualAlias extends VirtualAliasBase {
        public int target() { return 20; }
        public int run(boolean branch) {
          VirtualAlias alias = this;
          if (branch) alias = this;
          return alias.target();
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    assertInvocation(outRoot(), "pkg/VirtualAlias", "run", "target", "()I", INVOKEVIRTUAL);
    assertEquals("20,20", observeVirtual(outRoot()));
    decompileDirectory(outRoot(), "pkg/VirtualAlias.java");
    recompile();
    assertInvocation(fixture.getTempDir().resolve("recompiled-out"), "pkg/VirtualAlias", "run", "target", "()I", INVOKEVIRTUAL);
    assertEquals(observeVirtual(outRoot()), observeVirtual(fixture.getTempDir().resolve("recompiled-out")));
  }

  @ParameterizedTest(name = "inherited same-owner call through alias: {0}")
  @ValueSource(booleans = {false, true})
  void inheritedSameOwnerSpecialBypassesSubclassOverride(boolean copiedReceiver) throws Exception {
    Path parent = writeSource("pkg/InheritedParent.java", """
      package pkg;
      public class InheritedParent { public int value() { return 1; } }
      """);
    Path child = writeSource("pkg/InheritedChild.java", """
      package pkg;
      public class InheritedChild extends InheritedParent {
        public int value(String ignored) { return 3; }
        public int run() { %s }
      }
      """.formatted(copiedReceiver
        ? "InheritedChild alias = this; return alias.value();"
        : "return this.value();"));
    Path grandchild = writeSource("pkg/InheritedGrandchild.java", """
      package pkg;
      public class InheritedGrandchild extends InheritedChild {
        public int value() { return 2; }
      }
      """);
    compileJava8NoDebug(List.of(parent, child, grandchild), outRoot());
    try (URLClassLoader loader = loader(outRoot())) {
      assertEquals("pkg.InheritedParent", loader.loadClass("pkg.InheritedChild").getMethod("value").getDeclaringClass().getName());
    }
    // The symbolic owner is Child, but only Parent declares this descriptor.
    // Child's same-named overload must not hide that inherited declaration.
    makeCallSpecial(outRoot().resolve("pkg/InheritedChild.class"), "run", "value", "pkg/InheritedChild");
    assertInvocation(outRoot(), "pkg/InheritedChild", "run", "value", "()I", INVOKESPECIAL);
    assertEquals(1, observeRun(outRoot(), "pkg.InheritedGrandchild"));
    decompileDirectory(outRoot(), "pkg/InheritedChild.java");
    recompile();
    Path rebuilt = fixture.getTempDir().resolve("recompiled-out");
    assertEquals(observeRun(outRoot(), "pkg.InheritedGrandchild"), observeRun(rebuilt, "pkg.InheritedGrandchild"));
    assertInvocation(rebuilt, "pkg/InheritedChild", "run", "value", "()I", INVOKESPECIAL);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void unoverriddenSameOwnerSpecialNeedsNoHelper(boolean finalMethod) throws Exception {
    Path source = writeSource("pkg/DeclaredChild.java", """
      package pkg;
      class DeclaredParent { public int value() { return 1; } }
      public class DeclaredChild extends DeclaredParent {
        public %s int value() { return 2; }
        public int run() {
          DeclaredChild alias = this;
          return alias.value();
        }
      }
      """.formatted(finalMethod ? "final" : ""));
    compileJava8NoDebug(source, outRoot());
    makeCallSpecial(outRoot().resolve("pkg/DeclaredChild.class"), "run", "value", "pkg/DeclaredChild");
    assertInvocation(outRoot(), "pkg/DeclaredChild", "run", "value", "()I", INVOKESPECIAL);
    assertEquals(2, observeRun(outRoot(), "pkg.DeclaredChild"));
    String sourceText = decompileDirectory(outRoot(), "pkg/DeclaredChild.java");
    assertFalse(sourceText.contains("$vf$special$"));
    recompile();
    // With no override, an ordinary call retains the current implementation.
    assertEquals(observeRun(outRoot(), "pkg.DeclaredChild"),
      observeRun(fixture.getTempDir().resolve("recompiled-out"), "pkg.DeclaredChild"));
  }

  @Test
  void superclassSpecialOnMixedReceiverDoesNotBecomeCurrentReceiver() throws Exception {
    Path source = writeSource("pkg/MixedReceiver.java", """
      package pkg;
      class MixedParent {
        private final int marker;
        MixedParent(int marker) { this.marker = marker; }
        public int target(int ignored) { return marker; }
      }
      public class MixedReceiver extends MixedParent {
        public static int effects;
        public MixedReceiver(int marker) { super(marker); }
        private static int argument() { effects++; return 0; }
        public int run(boolean useThis, MixedReceiver other) {
          MixedReceiver alias = useThis ? this : other;
          return alias.target(argument());
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    makeCallSpecial(outRoot().resolve("pkg/MixedReceiver.class"), "run", "target", "pkg/MixedParent");
    // A superclass special receiver need not be this. With no target override
    // in these exact instances, the ordinary call fallback is equivalent;
    // this negative specifically guards against accepting a contaminated phi.
    assertEquals("10/1,20/1,NullPointerException/1", observeMixed(outRoot()));
    assertInvocation(outRoot(), "pkg/MixedReceiver", "run", "target", "(I)I", INVOKESPECIAL);
    decompileDirectory(outRoot(), "pkg/MixedReceiver.java");
    recompile();
    assertEquals(observeMixed(outRoot()), observeMixed(fixture.getTempDir().resolve("recompiled-out")));
  }

  @Test
  void ancestorSymbolicOwnerUsesDirectSuperImplementationAndOriginalOverload() throws Exception {
    Path source = writeSource("pkg/DispatchParent.java", """
      package pkg;
      class DispatchAncestor {
        public int choose(Object value) { return 10; }
        public int choose(String value) { return 20; }
      }
      public class DispatchParent extends DispatchAncestor {
        public int choose(Object value) { return 30; }
        public int choose(String value) { return 40; }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_MAXS);
    writer.visit(V1_2, ACC_PUBLIC | ACC_SUPER, CHILD, null, "pkg/DispatchParent", null);
    MethodVisitor init = writer.visitMethod(ACC_PUBLIC, "<init>", "()V", null, null);
    init.visitCode();
    init.visitVarInsn(ALOAD, 0);
    init.visitMethodInsn(INVOKESPECIAL, "pkg/DispatchParent", "<init>", "()V", false);
    init.visitInsn(RETURN);
    init.visitMaxs(0, 0);
    init.visitEnd();
    MethodVisitor override = writer.visitMethod(ACC_PUBLIC, "choose", "(Ljava/lang/Object;)I", null, null);
    override.visitCode();
    override.visitIntInsn(BIPUSH, 90);
    override.visitInsn(IRETURN);
    override.visitMaxs(0, 0);
    override.visitEnd();
    MethodVisitor run = writer.visitMethod(ACC_PUBLIC, "run", "()I", null, null);
    run.visitCode();
    run.visitVarInsn(ALOAD, 0);
    run.visitVarInsn(ASTORE, 1);
    run.visitVarInsn(ALOAD, 1);
    run.visitInsn(ACONST_NULL);
    // ACC_SUPER selects Parent.choose(Object), despite the ancestor owner.
    // Java must also retain Object overload selection for this null argument.
    run.visitMethodInsn(INVOKESPECIAL, "pkg/DispatchAncestor", "choose", "(Ljava/lang/Object;)I", false);
    run.visitInsn(IRETURN);
    run.visitMaxs(0, 0);
    run.visitEnd();
    writer.visitEnd();
    Files.write(outRoot().resolve(CHILD + ".class"), writer.toByteArray());
    assertEquals(30, observeHierarchy(outRoot()));
    assertInvocation(outRoot(), CHILD, "run", "choose", "(Ljava/lang/Object;)I", INVOKESPECIAL);
    decompileDirectory(outRoot(), CHILD + ".java");
    recompile();
    Path rebuilt = fixture.getTempDir().resolve("recompiled-out");
    assertInvocation(rebuilt, CHILD, "run", "choose", "(Ljava/lang/Object;)I", INVOKESPECIAL);
    assertEquals(observeHierarchy(outRoot()), observeHierarchy(rebuilt));
  }

  private static int observeHierarchy(Path classes) throws Exception {
    return observeRun(classes, CHILD.replace('/', '.'));
  }

  private static int observeRun(Path classes, String className) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = loader.loadClass(className);
      return (int)type.getMethod("run").invoke(type.getConstructor().newInstance());
    }
  }

  private static String observeMixed(Path classes) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = loader.loadClass("pkg.MixedReceiver");
      Object caller = type.getConstructor(int.class).newInstance(10);
      Object other = type.getConstructor(int.class).newInstance(20);
      var run = type.getMethod("run", boolean.class, type);
      StringBuilder result = new StringBuilder();
      for (int input = 0; input < 3; input++) {
        type.getField("effects").setInt(null, 0);
        if (input != 0) result.append(',');
        try {
          result.append(run.invoke(caller, input == 0, input == 2 ? null : other));
        } catch (InvocationTargetException failure) {
          result.append(failure.getCause().getClass().getSimpleName());
        }
        result.append('/').append(type.getField("effects").getInt(null));
      }
      return result.toString();
    }
  }

  private static void makeCallSpecial(Path classFile, String caller, String target, String targetOwner) throws Exception {
    ClassReader reader = new ClassReader(Files.readAllBytes(classFile));
    ClassWriter writer = new ClassWriter(0);
    int[] patched = {0};
    reader.accept(new ClassVisitor(ASM9, writer) {
      @Override
      public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        MethodVisitor delegate = super.visitMethod(access, name, descriptor, signature, exceptions);
        if (!name.equals(caller)) return delegate;
        return new MethodVisitor(ASM9, delegate) {
          @Override
          public void visitMethodInsn(int opcode, String owner, String name, String descriptor, boolean isInterface) {
            if (name.equals(target)) {
              patched[0]++;
              super.visitMethodInsn(INVOKESPECIAL, targetOwner, name, descriptor, false);
            } else {
              super.visitMethodInsn(opcode, owner, name, descriptor, isInterface);
            }
          }
        };
      }
    }, 0);
    assertEquals(1, patched[0]);
    Files.write(classFile, writer.toByteArray());
  }

  private static void assertInvocation(Path classes, String owner, String method, String target, String descriptor, int opcode) throws Exception {
    int[] calls = {0};
    new ClassReader(Files.readAllBytes(classes.resolve(owner + ".class"))).accept(new ClassVisitor(ASM9) {
      @Override
      public MethodVisitor visitMethod(int access, String name, String desc, String signature, String[] exceptions) {
        if (!name.equals(method)) return null;
        return new MethodVisitor(ASM9) {
          @Override
          public void visitMethodInsn(int actualOpcode, String owner, String name, String desc, boolean isInterface) {
            if (name.equals(target) && desc.equals(descriptor)) {
              calls[0]++;
              assertEquals(opcode, actualOpcode, method + " dispatch to " + owner + "." + target);
            }
          }
        };
      }
    }, ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
    assertEquals(1, calls[0], method + " must contain one call to " + target + descriptor);
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static String observeVirtual(Path classes) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> type = loader.loadClass("pkg.VirtualAlias");
      Object instance = type.getConstructor().newInstance();
      return type.getMethod("run", boolean.class).invoke(instance, false) + ","
        + type.getMethod("run", boolean.class).invoke(instance, true);
    }
  }

  private static String observe(Path classes, int input) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> child = loader.loadClass(CHILD.replace('/', '.'));
      Class<?> base = loader.loadClass(BASE.replace('/', '.'));
      Object instance = child.getConstructor().newInstance();
      String result;
      try {
        result = "result=" + child.getMethod("update", int.class).invoke(instance, input);
      } catch (InvocationTargetException failure) {
        result = "exception=" + failure.getCause().getClass().getSimpleName();
      }
      return result + ",base=" + base.getField("baseCalls").getInt(null)
        + ",child=" + base.getField("childCalls").getInt(null)
        + ",args=" + base.getField("argumentEffects").getInt(null)
        + ",overwrites=" + base.getField("overwriteEffects").getInt(null)
        + ",trace=" + base.getField("trace").getInt(null);
    }
  }
}
