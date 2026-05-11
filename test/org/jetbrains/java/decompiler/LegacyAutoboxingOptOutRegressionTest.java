package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacyAutoboxingOptOutRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "0"};
  }

  @Test
  public void testPreJava5SourceCanStillRenderAutoboxingWhenLegacyCompatibilityIsDisabled() throws IOException {
    Path source = writeSource("Repro.java", """
import java.util.Stack;

public class Repro {
  static void acceptInt(int value) {
  }

  static void acceptObject(Object value) {
  }

  public static int run(Stack stack) {
    acceptInt((Integer)stack.pop());
    int value = ((Integer)stack.pop()).intValue();
    acceptInt(value);
    acceptObject(3);
    return (Integer)stack.pop();
  }
}
""");

    compileJava8NoDebug(source, outRoot());
    LegacyAutoboxingRegressionTest.setClassMajorVersion(outRoot().resolve("Repro.class"), 48);

    String content = decompileDirectory(outRoot(), "Repro.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("acceptInt((Integer)var0.pop())"), content);
    assertTrue(content.contains("acceptObject(3)"), content);
    assertFalse(content.contains("acceptInt(((Integer)var0.pop()).intValue())"), content);
    assertFalse(content.contains("Integer.valueOf(3)"), content);

    recompile();
  }
}
