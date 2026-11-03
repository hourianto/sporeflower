package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Modifier;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.objectweb.asm.Opcodes.*;

public class DeclaredSpecialDispatchRegressionTest extends DecompileRegressionTestBase {
  @Test
  void packagePrivateTargetInAbstractClassKeepsSpecialAndVirtualSelfCalls() throws Exception {
    // A private implementation helper must not weaken access to an inherited
    // method with the same name and descriptor.
    Path ancestor = writeSource("pkg/ActionAncestor.java", """
      package pkg;
      public class ActionAncestor {
        public void $vf$special$0(int depth) { throw new AssertionError("helper name collision"); }
      }
      """);
    Path base = writeSource("pkg/ActionBase.java", """
      package pkg;
      public abstract class ActionBase extends ActionAncestor {
        public int sum, baseCalls, overrideCalls;
        void apply(int depth) {
          baseCalls++;
          sum++;
          if (depth > 0) {
            apply(depth - 1);
            apply(depth - 1);
          }
        }
        public void run(int depth) { apply(depth); }
        public void virtualRun(int depth) { apply(depth); }
        public void unchanged() {}
        public void plainRun() { unchanged(); }
      }
      """);
    Path child = writeSource("pkg/ActionChild.java", """
      package pkg;
      public class ActionChild extends ActionBase {
        public void apply(int depth) {
          overrideCalls++;
          sum += 100 + depth;
        }
      }
      """);
    compileJava8NoDebug(List.of(ancestor, base, child), outRoot());
    Path classFile = outRoot().resolve("pkg/ActionBase.class");
    setCallOpcodes(classFile, "run", "apply", "(I)V", INVOKESPECIAL);
    setCallOpcodes(classFile, "plainRun", "unchanged", "()V", INVOKESPECIAL);
    // A copied implementation must retain the distinction inside its body:
    // the first recursive call dispatches to Child, the second stays in Base.
    setCallOpcodes(classFile, "apply", "apply", "(I)V", INVOKEVIRTUAL, INVOKESPECIAL);
    Map<String, ActionState> expected = Map.of(
      "run/-1", new ActionState(1, 1, 0),
      "run/2", new ActionState(204, 3, 2),
      "virtualRun/-1", new ActionState(99, 0, 1),
      "virtualRun/2", new ActionState(102, 0, 1));
    Map<String, ActionState> original = actionStates(outRoot());
    assertEquals(expected, original);
    String source = decompileDirectory(outRoot(), "pkg/ActionBase.java");
    // A class with an overridden method must still leave its other methods alone.
    assertEquals(1L, source.lines().filter(line -> line.contains("private void $vf$special$")).count());
    recompile();
    assertEquals(original, actionStates(recompiledRoot()));
  }

  @Test
  void synchronizedTargetKeepsReceiverMonitorArgumentsAndVirtualOverride() throws Exception {
    Path base = writeSource("pkg/RenderBase.java", """
      package pkg;
      public class RenderBase {
        public static int effects, trace;
        public final int marker;
        public String result;
        public int baseCalls, overrideCalls;
        public boolean locked;
        public RenderBase(int marker) { this.marker = marker; }
        public synchronized void render(String a, String b, String c, Object[] items,
                                        String d, boolean x, boolean y, boolean z) {
          locked = Thread.holdsLock(this);
          if (!locked) throw new AssertionError("receiver monitor");
          baseCalls++;
          trace = trace * 10 + 4;
          result = marker + ":" + a + ":" + b + ":" + c + ":" + items.length + ":" + d
            + ":" + x + ":" + y + ":" + z + ":" + (items[0] == this);
        }
        private static String text(int digit) {
          effects++;
          trace = trace * 10 + digit;
          return "abc".substring(0, digit);
        }
        private static RenderChild receiver(RenderChild other) {
          trace = trace * 10 + 5;
          return other;
        }
        public void run() {
          render(text(1), null, text(2), new Object[]{this}, text(3), true, false, true);
        }
        public void onOther(RenderChild other) {
          receiver(other).render(text(1), null, text(2), new Object[]{other}, text(3), true, false, true);
        }
        public void virtualOnOther(RenderChild other) {
          receiver(other).render(text(1), null, text(2), new Object[]{other}, text(3), true, false, true);
        }
      }
      """);
    Path child = writeSource("pkg/RenderChild.java", """
      package pkg;
      public class RenderChild extends RenderBase {
        public RenderChild(int marker) { super(marker); }
        public synchronized void render(String a, String b, String c, Object[] items,
                                        String d, boolean x, boolean y, boolean z) {
          locked = Thread.holdsLock(this);
          if (!locked) throw new AssertionError("override monitor");
          overrideCalls++;
          trace = trace * 10 + 9;
          result = "override:" + marker;
        }
      }
      """);
    compileJava8NoDebug(List.of(base, child), outRoot());
    Path classFile = outRoot().resolve("pkg/RenderBase.class");
    String descriptor = "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/Object;Ljava/lang/String;ZZZ)V";
    setCallOpcodes(classFile, "run", "render", descriptor, INVOKESPECIAL);
    setCallOpcodes(classFile, "onOther", "render", descriptor, INVOKESPECIAL);

    RenderState untouched = new RenderState(null, 0, 0, false);
    Map<Receiver, RenderObservation> expected = Map.of(
      Receiver.SELF, new RenderObservation(null,
        new RenderState("10:a:null:ab:1:abc:true:false:true:true", 1, 0, true), untouched, 3, 1234),
      Receiver.SUBTYPE, new RenderObservation(null, untouched,
        new RenderState("20:a:null:ab:1:abc:true:false:true:true", 1, 0, true), 3, 51234),
      Receiver.VIRTUAL, new RenderObservation(null, untouched,
        new RenderState("override:20", 0, 1, true), 3, 51239),
      Receiver.NULL, new RenderObservation("NullPointerException", untouched, untouched, 3, 5123));
    Map<Receiver, RenderObservation> original = renderObservations(outRoot());
    assertEquals(expected, original);
    decompileDirectory(outRoot(), "pkg/RenderBase.java");
    recompile();
    assertEquals(original, renderObservations(recompiledRoot()));
  }

  private record ActionState(int sum, int baseCalls, int overrideCalls) { }

  private static Map<String, ActionState> actionStates(Path classes) throws Exception {
    try (URLClassLoader loader = loader(classes)) {
      Class<?> base = loader.loadClass("pkg.ActionBase");
      Class<?> child = loader.loadClass("pkg.ActionChild");
      assertTrue(Modifier.isAbstract(base.getModifiers()));
      int modifiers = base.getDeclaredMethod("apply", int.class).getModifiers();
      assertEquals(0, modifiers & (Modifier.PUBLIC | Modifier.PROTECTED | Modifier.PRIVATE | Modifier.STATIC | Modifier.FINAL | Modifier.ABSTRACT));
      Map<String, ActionState> results = new LinkedHashMap<>();
      for (String method : List.of("run", "virtualRun")) {
        for (int depth : new int[]{-1, 2}) {
          Object instance = child.getConstructor().newInstance();
          child.getMethod(method, int.class).invoke(instance, depth);
          results.put(method + "/" + depth, new ActionState(
            base.getField("sum").getInt(instance), base.getField("baseCalls").getInt(instance),
            base.getField("overrideCalls").getInt(instance)));
        }
      }
      return results;
    }
  }

  private enum Receiver { SELF, SUBTYPE, VIRTUAL, NULL }
  private record RenderState(String result, int baseCalls, int overrideCalls, boolean locked) { }
  private record RenderObservation(String failure, RenderState caller, RenderState other, int effects, int trace) { }

  private static Map<Receiver, RenderObservation> renderObservations(Path classes) throws Exception {
    Map<Receiver, RenderObservation> results = new LinkedHashMap<>();
    for (Receiver receiver : Receiver.values()) {
      try (URLClassLoader loader = loader(classes)) {
        Class<?> base = loader.loadClass("pkg.RenderBase");
        Class<?> child = loader.loadClass("pkg.RenderChild");
        int modifiers = base.getDeclaredMethod("render", String.class, String.class, String.class,
          Object[].class, String.class, boolean.class, boolean.class, boolean.class).getModifiers();
        assertTrue(Modifier.isPublic(modifiers) && Modifier.isSynchronized(modifiers));
        assertEquals(0, modifiers & (Modifier.PRIVATE | Modifier.STATIC | Modifier.FINAL | Modifier.ABSTRACT));
        Object caller = child.getConstructor(int.class).newInstance(10);
        Object other = child.getConstructor(int.class).newInstance(20);
        String failure = null;
        try {
          if (receiver == Receiver.SELF) {
            child.getMethod("run").invoke(caller);
          } else {
            String method = receiver == Receiver.VIRTUAL ? "virtualOnOther" : "onOther";
            child.getMethod(method, child)
              .invoke(caller, new Object[]{receiver == Receiver.NULL ? null : other});
          }
        } catch (InvocationTargetException exception) {
          failure = exception.getCause().getClass().getSimpleName();
        }
        results.put(receiver, new RenderObservation(failure, renderState(base, caller), renderState(base, other),
          base.getField("effects").getInt(null), base.getField("trace").getInt(null)));
      }
    }
    return results;
  }

  private static RenderState renderState(Class<?> base, Object receiver) throws Exception {
    return new RenderState((String)base.getField("result").get(receiver), base.getField("baseCalls").getInt(receiver),
      base.getField("overrideCalls").getInt(receiver), base.getField("locked").getBoolean(receiver));
  }

  private static void setCallOpcodes(Path classFile, String caller, String target, String descriptor, int... opcodes) throws Exception {
    ClassReader reader = new ClassReader(Files.readAllBytes(classFile));
    ClassWriter writer = new ClassWriter(0);
    int[] calls = {0};
    reader.accept(new ClassVisitor(ASM9, writer) {
      @Override
      public MethodVisitor visitMethod(int access, String name, String desc, String signature, String[] exceptions) {
        MethodVisitor delegate = super.visitMethod(access, name, desc, signature, exceptions);
        if (!name.equals(caller)) return delegate;
        return new MethodVisitor(ASM9, delegate) {
          @Override
          public void visitMethodInsn(int opcode, String owner, String name, String desc, boolean isInterface) {
            if (name.equals(target) && desc.equals(descriptor)) {
              assertTrue(calls[0] < opcodes.length, "Unexpected extra target call in " + caller);
              super.visitMethodInsn(opcodes[calls[0]++], reader.getClassName(), name, desc, false);
            } else {
              super.visitMethodInsn(opcode, owner, name, desc, isInterface);
            }
          }
        };
      }
    }, 0);
    assertEquals(opcodes.length, calls[0], "Target call count in " + caller);
    Files.write(classFile, writer.toByteArray());
  }

}
