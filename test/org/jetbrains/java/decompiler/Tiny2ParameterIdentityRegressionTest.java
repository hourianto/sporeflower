package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import java.nio.file.Path;
import java.util.List;

public class Tiny2ParameterIdentityRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
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
        """;
  }

  @Test
  public void testRealizedIdentitiesIncludeOverrideAndConflictRenames() throws Exception {
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
    String child = readDecompiled("pkg/Child.java");
    assertContains(source, "b(int first)", "int second)", "int conflictValue)", "identity(Renamed entity)",
      "chosenName(int baseValue)");
    assertContains(child, "chosenName(int childValue)");
    recompile();
  }
}
