package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

public class LegacyLanguageCompatibilityTest extends DecompileRegressionTestBase {
  @Test
  public void preJava5SourceKeepsExplicitBoxingAndUnboxingByDefault() throws IOException {
    String content = decompilePreJava5Boxing();
    assertContains(content, "acceptInt(((Integer)var0.pop()).intValue())", ".intValue()", "Integer.valueOf(3)");
    recompile();
  }

  @Test
  public void preJava5SourceUsesAutoboxingWhenLegacyCompatibilityIsDisabled() throws IOException {
    String content = decompilePreJava5Boxing(IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "0");
    assertContains(content, "acceptInt((Integer)var0.pop())", "acceptObject(3)");
    assertNotContains(content, "acceptInt(((Integer)var0.pop()).intValue())", "Integer.valueOf(3)");
    recompile();
  }

  private String decompilePreJava5Boxing(Object... options) throws IOException {
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
    Path classFile = outRoot().resolve("Repro.class");
    byte[] bytes = Files.readAllBytes(classFile);
    ClassFileTestUtil.putU2(bytes, 6, 48);
    Files.write(classFile, bytes);

    return decompileDirectory(outRoot(), "Repro.java", options);
  }
}
