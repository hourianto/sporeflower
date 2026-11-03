package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class PreparedFieldScopeRegressionTest extends DecompileRegressionTestBase {
  @Test
  void aPrivateIntermediateDeclarationBlocksANewlyAliasedAncestor() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Parent.java", "package p; public class Parent { public static int first = 11; }"),
      writeSource("p/Middle.java", "package p; public class Middle extends Parent { private static int second = 22; }"),
      writeSource("p/Probe.java", "package p; public class Probe extends Middle { public static int run() { return Probe.first; } }")
    ), outRoot());
    assertEquals(11, invoke(outRoot()));
    Decompiler.Builder builder = builder();
    NamingPlan alias = aliasFields(builder.build().prepareNames());
    IllegalArgumentException error = assertThrows(IllegalArgumentException.class,
      () -> builder.preparedNames(alias).build().prepareNames());
    assertTrue(error.getMessage().contains("field names collide in source scope"), error.getMessage());
  }

  @Test
  void newlyAliasedClassAndInterfaceFieldsAreAmbiguous() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Parent.java", "package p; public class Parent { public static int first = 11; }"),
      writeSource("p/Contract.java", "package p; public interface Contract { int second = Integer.parseInt(\"22\"); }"),
      writeSource("p/Probe.java", """
        package p;
        public class Probe extends Parent implements Contract {
          public static int run() { return Probe.first + Probe.second; }
        }
        """)
    ), outRoot());
    assertEquals(33, invoke(outRoot()));
    Decompiler.Builder builder = builder();
    NamingPlan alias = aliasFields(builder.build().prepareNames());
    IllegalArgumentException error = assertThrows(IllegalArgumentException.class,
      () -> builder.preparedNames(alias).build().prepareNames());
    assertTrue(error.getMessage().contains("field names collide in source scope"), error.getMessage());
  }

  @Test
  void inaccessibleAncestorDoesNotMakeAnInheritedInterfaceFieldAmbiguous() throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/Parent.java", """
        package p;
        public class Parent {
          static int value = 11;
          public static int parentValue() { return value; }
        }
        """),
      writeSource("q/Contract.java", "package q; public interface Contract { int value = Integer.parseInt(\"22\"); }"),
      writeSource("q/Probe.java", """
        package q;
        public class Probe extends p.Parent implements Contract {
          public static int run() { return Probe.value + parentValue(); }
        }
        """)
    ), outRoot());
    Decompiler.Builder builder = builder();
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("value", plan.fields().get(new NamingPlan.Member("p/Parent", "value", "I")));
    assertEquals("value", plan.fields().get(new NamingPlan.Member("q/Contract", "value", "I")));
    assertEquals(33, invoke(outRoot(), "q.Probe"));
    builder.preparedNames(plan).build().decompile();
    recompile();
    assertEquals(33, invoke(fixture.getTempDir().resolve("recompiled-out"), "q.Probe"));
  }

  private static NamingPlan aliasFields(NamingPlan plan) {
    var fields = new LinkedHashMap<>(plan.fields());
    fields.replaceAll((member, name) -> "value");
    return new NamingPlan(plan.classes(), fields, plan.methods(), plan.parameters(), plan.innerNames());
  }

  private Decompiler.Builder builder() {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private static Object invoke(Path classes) throws Exception {
    return invoke(classes, "p.Probe");
  }

  private static Object invoke(Path classes, String owner) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return loader.loadClass(owner).getMethod("run").invoke(null);
    }
  }
}
