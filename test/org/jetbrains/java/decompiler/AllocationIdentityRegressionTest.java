package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class AllocationIdentityRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testAliasesAndControlFlowPreserveAllocationIdentity() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    // Load and execute the bytecode first: an uninitialized-reference fixture must
    // satisfy the verifier, independently of whether its decompiled source compiles.
    String[] methods = {"aliases", "alternatives", "branchAliases", "overwrittenAlias", "distinct", "loop", "protectedArguments", "guardedArguments"};
    int[] inputs = {-1, 0, 1, 12};
    Object[][] expected = new Object[methods.length][inputs.length];
    for (int method = 0; method < methods.length; method++) {
      for (int input = 0; input < inputs.length; input++) {
        expected[method][input] = invoke(original, methods[method], inputs[input]);
      }
    }

    String content = decompileClassFile(original.resolve("pkg/TestAllocationIdentity.class"), "pkg/TestAllocationIdentity.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    recompile();
    for (int method = 0; method < methods.length; method++) {
      for (int input = 0; input < inputs.length; input++) {
        assertEquals(expected[method][input], invoke(fixture.getTempDir().resolve("recompiled-out"), methods[method], inputs[input]),
          methods[method] + "(" + inputs[input] + ")");
      }
    }
  }

  private static Object invoke(Path classes, String method, int input) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      try {
        return Class.forName("pkg.TestAllocationIdentity", true, loader).getMethod(method, int.class).invoke(null, input);
      } catch (InvocationTargetException exception) {
        return exception.getCause().getClass().getName();
      }
    }
  }
}
