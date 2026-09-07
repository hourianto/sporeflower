package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class MethodRelationshipRenameRegressionTest extends DecompileRegressionTestBase {
  private Path mapping;

  @Override
  @AfterEach
  public void tearDown() {
    super.tearDown();
    if (mapping != null) {
      try {
        Files.deleteIfExists(mapping);
      } catch (IOException ignored) {
      }
    }
  }

  @Test
  public void testPackagePrivateDeclarationsKeepIndependentMappedNames() throws Exception {
    configureMapping("""
      c\tp/Base\tp/Base
      \tm\t()I\tm\tbaseValue
      c\tq/Child\tq/Child
      \tm\t()I\tm\tchildValue
      """);
    compileIndependentMethods();
    String child = decompileDirectory(outRoot(), "q/Child.java");
    String base = Files.readString(fixture.getTargetDir().resolve("p/Base.java"));
    assertTrue(base.contains("int baseValue()"), base);
    assertTrue(child.contains("int childValue()"), child);
    assertDispatchPreserved("q.Child", "q.Child", 11, 22);
  }

  @Test
  public void testRelocationDoesNotCreateAnOverride() throws Exception {
    configureMapping("""
      c\tp/Base\tmerged/Base
      \tm\t()I\tm\tvalue
      c\tq/Child\tmerged/Child
      \tm\t()I\tm\tvalue
      """);
    compileIndependentMethods();
    decompileDirectory(outRoot(), "merged/Child.java");
    assertDispatchPreserved("q.Child", "merged.Child", 11, 22);
  }

  @Test
  public void testDeclarationBackInOriginalPackageOverridesAcrossForeignSuperclass() throws Exception {
    configureMapping("""
      c\tp/Base\tp/Base
      c\tq/Middle\tq/Middle
      c\tp/Child\tp/Child
      \tm\t()I\tm\tvalue
      """);
    Path base = writeSource("p/Base.java", """
      package p;
      public class Base {
        int m() { return 11; }
        public int baseCall() { return m(); }
      }
      """);
    Path middle = writeSource("q/Middle.java", """
      package q;
      public class Middle extends p.Base { }
      """);
    Path child = writeSource("p/Child.java", """
      package p;
      public class Child extends q.Middle {
        int m() { return 22; }
        public int childCall() { return m(); }
      }
      """);
    compileJava8NoDebug(List.of(base, middle, child), outRoot());
    decompileDirectory(outRoot(), "p/Child.java");
    assertDispatchPreserved("p.Child", "p.Child", 22, 22);
  }

  @Test
  public void testPackagePrivateMethodDoesNotJoinInterfaceImplementationFamily() throws Exception {
    configureMapping("""
      c\tp/Base\tp/Base
      \tm\t()I\tm\tbaseValue
      c\tq/Contract\tq/Contract
      \tm\t()I\tm\tcontractValue
      c\tq/Child\tq/Child
      """);
    Path base = writeSource("p/Base.java", """
      package p;
      public class Base {
        int m() { return 11; }
        public int baseCall() { return m(); }
      }
      """);
    Path contract = writeSource("q/Contract.java", """
      package q;
      public interface Contract { int m(); }
      """);
    Path child = writeSource("q/Child.java", """
      package q;
      public class Child extends p.Base implements Contract {
        public int m() { return 22; }
        public int childCall() { return ((Contract)this).m(); }
      }
      """);
    compileJava8NoDebug(List.of(base, contract, child), outRoot());
    String output = decompileDirectory(outRoot(), "q/Child.java");
    assertTrue(output.contains("int contractValue()"), output);
    assertDispatchPreserved("q.Child", "q.Child", 11, 22);
  }

  @Test
  public void testInterfaceStaticsDoNotReserveImplementorNames() throws Exception {
    configureMapping("""
      c\tp/Contract\tp/Contract
      \tm\t()I\tm\tvalue
      c\tp/Child\tp/Child
      \tm\t()I\tm\tvalue
      """);
    Path contract = writeSource("p/Contract.java", """
      package p;
      public interface Contract {
        static int m() { return 11; }
      }
      """);
    Path child = writeSource("p/Child.java", """
      package p;
      public class Child implements Contract {
        public static int m() { return 22; }
        public int baseCall() { return Contract.m(); }
        public int childCall() { return m(); }
      }
      """);
    compileJava8NoDebug(List.of(contract, child), outRoot());
    String output = decompileDirectory(outRoot(), "p/Child.java");
    assertTrue(output.contains("static int value()"), output);
    assertDispatchPreserved("p.Child", "p.Child", 11, 22);
  }

  @Test
  public void testSwappedMappingNamesKeepOverloadsAndOverrideFamiliesSeparate() throws Exception {
    configureMapping("""
      c\tp/Base\tp/Base
      c\tq/Child\tq/Child
      \tm\t()I\ta\tb
      \tm\t()I\tb\ta
      \tm\t(I)I\ta\twithArgument
      """);
    Path base = writeSource("p/Base.java", """
      package p;
      public class Base {
        protected int a() { return 1; }
        protected int b() { return 2; }
        protected int a(int n) { return n; }
        public int baseCall() { return a() + a(10); }
      }
      """);
    Path child = writeSource("q/Child.java", """
      package q;
      public class Child extends p.Base {
        protected int a() { return 3; }
        protected int b() { return 4; }
        protected int a(int n) { return n + 5; }
        public int childCall() { return b(); }
      }
      """);
    compileJava8NoDebug(List.of(base, child), outRoot());
    String output = decompileDirectory(outRoot(), "q/Child.java");
    assertTrue(output.contains("int withArgument(int"), output);
    assertDispatchPreserved("q.Child", "q.Child", 18, 4);
  }

  @Test
  public void testExternalDeclarationKeepsItsRealizedNameInOverrides() throws Exception {
    configureMapping("""
      c\tq/Child\tq/Child
      \tm\t()I\tm\trequestedName
      """);
    Path base = writeSource("p/Base.java", """
      package p;
      public class Base {
        protected int m() { return 11; }
        public int baseCall() { return m(); }
      }
      """);
    Path child = writeSource("q/Child.java", """
      package q;
      public class Child extends p.Base {
        protected int m() { return 22; }
        public int childCall() { return m(); }
      }
      """);
    compileJava8NoDebug(List.of(base, child), outRoot());
    fixture.getDecompiler().addLibrary(outRoot().resolve("p/Base.class").toFile());
    String output = decompileClassFile(outRoot().resolve("q/Child.class"), "q/Child.java");
    assertTrue(output.contains("int m()"), output);
    assertEquals(List.of(22, 22), invokeCalls(outRoot(), "q.Child"));
    recompile(List.of(base));
    assertEquals(List.of(22, 22), invokeCalls(fixture.getTempDir().resolve("recompiled-out"), "q.Child"));
  }

  private void compileIndependentMethods() throws IOException {
    Path base = writeSource("p/Base.java", """
      package p;
      public class Base {
        int m() { return 11; }
        public int baseCall() { return m(); }
      }
      """);
    Path child = writeSource("q/Child.java", """
      package q;
      public class Child extends p.Base {
        public int m() { return 22; }
        public int childCall() { return m(); }
      }
      """);
    compileJava8NoDebug(List.of(base, child), outRoot());
  }

  private void configureMapping(String members) throws IOException {
    mapping = Files.createTempFile("vf-method-relationships-", ".tiny");
    Files.writeString(mapping, "tiny\t2\t0\tofficial\tnamed\n" + members);
    fixture.tearDown();
    fixture.setUp(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
  }

  private void assertDispatchPreserved(String originalClass, String renamedClass, int baseValue, int childValue) throws Exception {
    assertEquals(List.of(baseValue, childValue), invokeCalls(outRoot(), originalClass));
    recompile();
    assertEquals(List.of(baseValue, childValue), invokeCalls(fixture.getTempDir().resolve("recompiled-out"), renamedClass),
      Files.readString(fixture.getTargetDir().resolve(renamedClass.replace('.', '/') + ".java")));
  }

  private static List<Object> invokeCalls(Path classes, String className) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[] {classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> cls = loader.loadClass(className);
      Object instance = cls.getConstructor().newInstance();
      return List.of(cls.getMethod("baseCall").invoke(instance), cls.getMethod("childCall").invoke(instance));
    }
  }
}
