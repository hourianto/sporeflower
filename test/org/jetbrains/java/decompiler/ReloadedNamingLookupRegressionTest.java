package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class ReloadedNamingLookupRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void genericConstructorsEnumsAndClassLiteralsKeepTheirDeclarations(boolean mapped) throws Exception {
    compileJava8NoDebug(List.of(
      writeSource("p/a.java", """
        package p;
        public enum a {
          FIRST(7), SECOND(11);
          private final int value;
          <T extends Number> a(T input) { value = input.intValue(); }
          public int value() { return value; }
        }
        """),
      writeSource("p/b.java", """
        package p;
        public class b<T extends Number> {
          private final int value;
          public b(T input) { value = input.intValue(); }
          public <U extends Number> b(T input, U extra) { value = input.intValue() + extra.intValue(); }
          public int value() { return value; }
        }
        """),
      writeSource("p/Runner.java", """
        package p;
        public class Runner {
          public static int run() throws Exception {
            if (a.class != a.FIRST.getClass()) return -1;
            if (b[].class.getComponentType() != new b<Integer>(1).getClass()) return -2;
            if (Class.forName("p.a") != a.class) return -3;
            return a.FIRST.value() + a.SECOND.value() + new b<Integer>(5, 2).value();
          }
          public static String literalName() { return "p.a"; }
        }
        """)), outRoot());
    assertEquals(25, invoke(outRoot(), "p/Runner", "run"));
    Decompiler.Builder preparation = builder();
    if (mapped) {
      Path request = fixture.getTempDir().resolve("requested.tiny");
      Files.writeString(request, "tiny\t2\t0\tofficial\tnamed\nc\tp/a\tnamed/Choice\nc\tp/b\tnamed/Box\n");
      preparation.option(IFernflowerPreferences.MAPPINGS_PATH, request.toString());
    }
    NamingPlan plan = preparation.build().prepareNames();
    assertNotEquals("p/a", plan.classes().get("p/a"));
    assertNotEquals("p/b", plan.classes().get("p/b"));
    builder().preparedNames(plan).build().decompile();
    for (Path source : listJavaSources(fixture.getTargetDir())) {
      String text = Files.readString(source);
      assertFalse(text.contains("Couldn't be decompiled"), text);
    }
    recompile();
    Path rebuilt = fixture.getTempDir().resolve("recompiled-out");
    String runner = plan.classes().get("p/Runner");
    assertEquals(25, invoke(rebuilt, runner, "run"));
    assertEquals("p.a", invoke(rebuilt, runner, "literalName"));
  }

  private Decompiler.Builder builder() {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private static Object invoke(Path classes, String owner, String method) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return loader.loadClass(owner.replace('/', '.')).getMethod(method).invoke(null);
    }
  }
}
