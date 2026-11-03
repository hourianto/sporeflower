package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.renamer.ConverterHelper;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class CustomMemberNamePolicyRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void asksOnceAndFallsBackForCollidingSuggestions(boolean prepared) throws Exception {
    compileProbe();
    Decompiler.Builder builder = builder(SingleSuggestionRenamer.class);
    NamingPlan plan = builder.build().prepareNames();
    assertEquals("taken", plan.fields().get(new NamingPlan.Member("p/Probe", "first", "I")));
    assertEquals("taken", plan.methods().get(new NamingPlan.Member("p/Probe", "first", "()I")));
    assertTrue(plan.fields().get(new NamingPlan.Member("p/Probe", "second", "I")).startsWith("field_"));
    assertTrue(plan.methods().get(new NamingPlan.Member("p/Probe", "second", "()I")).startsWith("method_"));
    rebuild(builder, plan, prepared);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void asksOnceAndFallsBackForIllegalSuggestions(boolean prepared) throws Exception {
    compileProbe();
    Decompiler.Builder builder = builder(InvalidSuggestionRenamer.class);
    NamingPlan plan = builder.build().prepareNames();
    assertTrue(plan.fields().get(new NamingPlan.Member("p/Probe", "first", "I")).startsWith("field_"));
    assertTrue(plan.methods().get(new NamingPlan.Member("p/Probe", "first", "()I")).startsWith("method_"));
    rebuild(builder, plan, prepared);
  }

  private void compileProbe() throws Exception {
    compileJava8NoDebug(writeSource("p/Probe.java", """
      package p;
      public class Probe {
        public static int first = 11;
        public static int second = 22;
        public static int retry = 33;
        public static int first() { return 11; }
        public static int second() { return 22; }
        public static int retry() { return 33; }
        public static int run() { return first + second + first() + second(); }
      }
      """), outRoot());
  }

  private Decompiler.Builder builder(Class<? extends ConverterHelper> renamer) {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.USER_RENAMER_CLASS, renamer.getName())
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private void rebuild(Decompiler.Builder builder, NamingPlan plan, boolean prepared) throws Exception {
    assertEquals(66, invoke(outRoot()));
    if (prepared) builder.option(IFernflowerPreferences.USER_RENAMER_CLASS, "").preparedNames(plan);
    builder.build().decompile();
    recompile();
    assertEquals(66, invoke(fixture.getTempDir().resolve("recompiled-out")));
  }

  private static Object invoke(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return loader.loadClass("p.Probe").getMethod("run").invoke(null);
    }
  }

  public static class SingleSuggestionRenamer extends ConverterHelper {
    private final Set<String> requested = new HashSet<>();

    @Override
    public boolean toBeRenamed(Type type, String owner, String name, String descriptor) {
      return type != Type.ELEMENT_CLASS && ("first".equals(name) || "second".equals(name));
    }

    @Override
    public String getNextFieldName(String owner, String name, String descriptor) {
      return request("field:" + name);
    }

    @Override
    public String getNextMethodName(String owner, String name, String descriptor) {
      return request("method:" + name);
    }

    private String request(String key) {
      if (!requested.add(key)) throw new AssertionError("Repeated custom request: " + key);
      return candidate();
    }

    protected String candidate() { return "taken"; }
  }

  public static class InvalidSuggestionRenamer extends SingleSuggestionRenamer {
    @Override
    protected String candidate() { return "if"; }
  }
}
