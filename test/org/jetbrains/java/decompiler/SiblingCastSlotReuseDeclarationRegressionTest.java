package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;

public class SiblingCastSlotReuseDeclarationRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.DECOMPILER_COMMENTS, "0"
    };
  }

  @Test
  public void testSiblingCastSlotReuseDeclarationIsWidenedWhenMerged() throws IOException {
    Path source = writeSource("Repro.java", """
public class Repro {
  static class Base {
    int value;
  }

  static class Left extends Base {
    int left;
  }

  static class Right extends Base {
    int right;
  }

  static Base make(int value) {
    return value == 0 ? new Left() : new Right();
  }

  static void useBase(Base value) {
  }

  public static void test(int mode) {
    {
      Left local;
      useBase(local = (Left)make(0));
      useBase(local);
    }

    switch (mode) {
      case 1:
        Right local;
        useBase(local = (Right)make(1));
        useBase(local);
        break;
    }
  }
}
""");

    compileJava8NoDebug(source, outRoot());

    decompileDirectory(outRoot(), "Repro.java");

    Path decompiledFile = fixture.getTargetDir().resolve("Repro.java");
    compileJava8(decompiledFile, fixture.getTempDir().resolve("compile-out"));
  }
}
