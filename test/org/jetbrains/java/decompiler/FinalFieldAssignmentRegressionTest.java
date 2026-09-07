package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.junit.jupiter.api.Test;
import java.net.URL;
import java.net.URLClassLoader;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import static org.junit.jupiter.api.Assertions.assertEquals;

public class FinalFieldAssignmentRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testBlankFinalPathsAndDelegation() throws Exception {
    Map<String, String> bodies = Map.of(
      "Partial", "if (b) value = 7;",
      "Branches", "if (b) value = 7; else value = 9;",
      "Loop", "while (b) { value = 7; b = false; }",
      "Twice", "value = 7; if (b) value = 9;",
      "Throwing", "throw new IllegalStateException();",
      "ThrowBranch", "if (b) throw new IllegalStateException(); value = 7;",
      "ReadBefore", "value = value + (b ? 7 : 9);"
    );
    for (var entry : bodies.entrySet()) {
      Path source = writeSource("pkg/" + entry.getKey() + ".java", "package pkg; public class " + entry.getKey()
        + " { public int value; public " + entry.getKey() + "(boolean b) { " + entry.getValue() + " } }");
      compileJava8NoDebug(source, outRoot());
    }
    Path delegated = writeSource("pkg/Delegating.java", """
      package pkg;
      public class Delegating {
        public int value;
        public Delegating(boolean b) { this(b ? 7 : 9); }
        public Delegating(int n) { value = n; }
      }
      """);
    compileJava8NoDebug(delegated, outRoot());
    try (var files = Files.list(outRoot().resolve("pkg"))) {
      for (Path file : files.toList()) {
        // Only the access flag changes: conditional/repeated writes are legal
        // constructor bytecode, although Java source forbids these blank finals.
        Files.write(file, ClassFileTestUtil.addMemberFlags(Files.readAllBytes(file), false, "value", CodeConstants.ACC_FINAL));
      }
    }
    decompileDirectory(outRoot(), "pkg/Partial.java");
    recompile();
    try (var original = loader(outRoot()); var rebuilt = loader(fixture.getTempDir().resolve("recompiled-out"))) {
      for (String name : new String[]{"Partial", "Branches", "Loop", "Twice", "Throwing", "ThrowBranch", "ReadBefore", "Delegating"}) {
        Class<?> first = original.loadClass("pkg." + name), second = rebuilt.loadClass("pkg." + name);
        for (boolean flag : new boolean[]{false, true}) assertEquals(outcome(first, flag), outcome(second, flag), name);
        boolean keepFinal = name.equals("Branches") || name.equals("Throwing") || name.equals("ThrowBranch") || name.equals("Delegating");
        assertEquals(keepFinal, Modifier.isFinal(second.getField("value").getModifiers()), name);
      }
    }
  }

  private static URLClassLoader loader(Path classes) throws Exception {
    return new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static String outcome(Class<?> cls, boolean flag) throws Exception {
    try {
      Object instance = cls.getConstructor(boolean.class).newInstance(flag);
      return "value:" + cls.getField("value").get(instance);
    } catch (InvocationTargetException exception) {
      return "throw:" + exception.getCause().getClass().getName();
    }
  }
}
