package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2CompilerHygieneRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
tiny\t2\t0\tofficial\tnamed
c\tC\tGameEngine
""";
  }

  @Test
  public void testTinyModeKeepsLegalOriginalNamesButRenamesJavaKeywords() throws IOException {
    Path source = writeSource("C.java", """
public class C {
  static int b;
  static int xx;

  static int a() {
    return b + xx;
  }
}
""");

    compileJava8NoDebug(source, outRoot());
    renameUtf8Constant(outRoot().resolve("C.class"), "xx", "do");

    String content = decompileDirectory(outRoot(), "GameEngine.java");
    assertTrue(Pattern.compile("static\\s+int\\s+b\\s*;").matcher(content).find(), content);
    assertTrue(Pattern.compile("static\\s+int\\s+a\\s*\\(").matcher(content).find(), content);
    assertTrue(content.contains("field_0"), content);
    assertFalse(Pattern.compile("\\bdo\\b").matcher(stripComments(content)).find(), content);

    recompile();
  }

  private static String stripComments(String content) {
    return content.replaceAll("(?s)/\\*.*?\\*/", "").replaceAll("//.*", "");
  }
}
