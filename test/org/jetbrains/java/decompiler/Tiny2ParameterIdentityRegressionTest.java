package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2ParameterIdentityRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testRealizedIdentitiesIncludeOverrideAndConflictRenames() throws Exception {
    Path mapping = Files.createTempFile("vf-parameter-identities-", ".tiny");
    try {
      Files.writeString(mapping, """
        tiny\t2\t0\tofficial\tnamed
        c\tpkg/A\tpkg/Renamed
        \tm\t(I)I\ta\tb
        \t\tp\t0\t\tfirst
        \tm\t(I)I\tb\tc
        \t\tp\t0\t\tsecond
        \tm\t(I)I\tx\tc
        \t\tp\t0\t\tconflictValue
        \tm\t(Lpkg/A;)Lpkg/A;\tidentity\t
        \t\tp\t0\t\tentity
        \tm\t(I)I\tvirtual\tbaseName
        \t\tp\t1\t\tbaseValue
        c\tpkg/B\tpkg/Child
        \tm\t(I)I\tvirtual\tchosenName
        \t\tp\t1\t\tchildValue
        """);
      fixture.tearDown();
      fixture.setUp(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
      Path a = writeSource("pkg/A.java", """
        package pkg;
        public abstract class A {
          public static int a(int n) { return n + 1; }
          public static int b(int n) { return n + 2; }
          public static int x(int n) { return n + 3; }
          public static A identity(A a) { return a; }
          public abstract int virtual(int n);
        }
        """);
      Path b = writeSource("pkg/B.java", """
        package pkg;
        public class B extends A {
          public int virtual(int n) { return n + 4; }
        }
        """);
      compileJava8NoDebug(List.of(a, b), outRoot());
      String source = decompileDirectory(outRoot(), "pkg/Renamed.java");
      String child = Files.readString(fixture.getTargetDir().resolve("pkg/Child.java"));
      assertTrue(source.contains("b(int first)"), source);
      assertTrue(source.contains("int second)"), source);
      assertTrue(source.contains("int conflictValue)"), source);
      assertTrue(source.contains("identity(Renamed entity)"), source);
      assertTrue(source.contains("chosenName(int baseValue)"), source);
      assertTrue(child.contains("chosenName(int childValue)"), child);
      recompile();
    } finally {
      Files.deleteIfExists(mapping);
    }
  }
}
