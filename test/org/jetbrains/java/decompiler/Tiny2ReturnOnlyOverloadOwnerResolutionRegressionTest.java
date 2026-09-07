package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2ReturnOnlyOverloadOwnerResolutionRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
tiny\t2\t0\tofficial\tnamed
""";
  }

  @Test
  public void testOwnerMethodIsNotRemappedToRenamedSuperclassReturnOnlyOverload() throws IOException {
    Path base = writeSource("g.java", """
class g {
  private void x() {
  }

  static String y() {
    return "base";
  }

  private static int z() {
    return 1;
  }

  static int touch() {
    return z();
  }
}
""");

    Path child = writeSource("a.java", """
class a extends g {
  static int a() {
    return 2;
  }
}
""");

    Path caller = writeSource("Caller.java", """
class Caller {
  static int call() {
    return a.a();
  }
}
""");

    compileJava8NoDebug(List.of(base, child, caller), outRoot());
    for (char name : new char[]{'x', 'y', 'z'}) {
      renameUtf8Constant(outRoot().resolve("g.class"), name, 'a');
    }

    String content = decompileDirectory(outRoot(), "Caller.java");
    assertTrue(content.contains("return a.a();") || content.contains("return a.method_0();"), content);

    recompile();
  }
}
