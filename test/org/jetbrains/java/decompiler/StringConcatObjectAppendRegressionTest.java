package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class StringConcatObjectAppendRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testObjectAppendDoesNotWidenStringAccumulator() throws IOException {
    String content = compileDecompileAndRead("pkg/TestStringConcatObjectAppend.java", """
package pkg;

import java.util.Vector;

public class TestStringConcatObjectAppend {
  public static void store(Vector values) {
    String text = "";
    for (int i = 0; i < values.size(); i++) {
      text = text + values.elementAt(i) + ";";
    }
    sink(text);
    sink(String.valueOf(text));
  }

  private static void sink(String value) {
  }
}
""");

    assertTrue(Pattern.compile("\\bString\\s+var\\d+\\s*=\\s*\"\";").matcher(content).find(), content);
    assertFalse(Pattern.compile("\\bObject\\s+var\\d+\\s*=\\s*\"\";").matcher(content).find(), content);
    assertFalse(content.contains("(String)var"), content);

    recompile();
  }
}
