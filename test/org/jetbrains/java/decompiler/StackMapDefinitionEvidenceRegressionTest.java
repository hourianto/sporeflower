package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class StackMapDefinitionEvidenceRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFrameTypesSurviveSlotReuseAndVariableReconstruction() throws Exception {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm");
    String content = decompileClassFile(classes.resolve("pkg/TestStackMapDefinitionEvidence.class"),
      "pkg/TestStackMapDefinitionEvidence.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    String delayed = content.substring(content.indexOf("boolean delayedArray"), content.indexOf("int wideReuse"));
    assertTrue(delayed.contains("boolean[]"), content);
    assertFalse(delayed.contains("byte[]"), content);
    String copied = content.substring(content.indexOf("boolean copiedArray"));
    assertTrue(copied.contains("boolean[]"), content);
    String unrelated = content.substring(content.indexOf("int unrelatedReferences"));
    assertTrue(unrelated.contains("int[]"), content);
    assertTrue(unrelated.contains("StringBuilder"), content);
    assertFalse(unrelated.contains("Object"), content);
    recompile();

    Path recompiled = fixture.getTempDir().resolve("recompiled-out");
    for (String method : new String[]{"delayedArray", "copiedArray", "wideReuse", "loop", "protectedReuse", "unrelatedReferences", "conditionalAssignment"}) {
      for (int input : new int[]{-1, 0, 1, 5}) {
        assertEquals(invoke(classes, method, int.class, input), invoke(recompiled, method, int.class, input),
          method + "(" + input + ")");
      }
    }
    for (boolean input : new boolean[]{false, true}) {
      assertEquals(invoke(classes, "overwrite", boolean.class, input), invoke(recompiled, "overwrite", boolean.class, input));
    }
  }

  private static Object invoke(Path classes, String method, Class<?> parameter, Object input) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> probe = Class.forName("pkg.TestStackMapDefinitionEvidence", true, loader);
      try {
        return probe.getMethod(method, parameter).invoke(null, input);
      } catch (InvocationTargetException exception) {
        return exception.getCause().getClass().getName();
      }
    }
  }
}
