package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class LoopExitTailRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSearchPreservesExitsResultsAndTailEffects() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    String content = decompileClassFile(original.resolve("pkg/TestLoopExitTail.class"), "pkg/TestLoopExitTail.java");
    recompile();

    try (URLClassLoader input = loader(original);
         URLClassLoader output = loader(recompiledRoot())) {
      for (String method : new String[]{"search", "withTailEffects"}) {
        var before = input.loadClass("pkg.TestLoopExitTail").getMethod(method, int.class, int.class, int.class);
        var after = output.loadClass("pkg.TestLoopExitTail").getMethod(method, int.class, int.class, int.class);
        for (int mode = -1; mode <= 2; mode++) {
          for (int rows = 0; rows <= 3; rows++) {
            for (int match = -1; match <= 6; match++) {
              int expected;
              if (method.equals("search")) {
                expected = mode < 0 ? 0 : mode == 1 ? 30 : mode == 2 ? 70 :
                  20 + (match >= 0 && match < 2 * rows ? match + 101 : 2 * rows);
              } else {
                expected = match >= 0 && match < rows ? (match + 1) * 10 + 1 :
                  (rows + (mode == 0 ? 20 : 30)) * 10;
              }
              String context = method + ": mode=" + mode + ", rows=" + rows + ", match=" + match;
              // Check the hand-written bytecode independently, then compare the round trip.
              assertEquals(expected, before.invoke(null, mode, rows, match), context);
              assertEquals(expected, after.invoke(null, mode, rows, match), context);
            }
          }
        }
      }
    }
  }

}
