package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class ShortCircuitLoopInitializationTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  public void preservesValueWhenLoopConditionSkipsAssignment(boolean strict) throws Exception {
    DecompilerContext.setProperty(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, strict ? "1" : "0");
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    String content = decompileClassFile(original.resolve("pkg/TestShortCircuitLoopInitialization.class"),
      "pkg/TestShortCircuitLoopInitialization.java");
    assertFalse(content.contains("Couldn't be decompiled"), content);
    recompile();
    try (URLClassLoader before = new URLClassLoader(new URL[]{original.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
         URLClassLoader after = new URLClassLoader(new URL[]{fixture.getTempDir().resolve("recompiled-out").toUri().toURL()},
           ClassLoader.getPlatformClassLoader())) {
      Class<?>[] parameters = {Object.class, String.class, int.class, int.class};
      Method originalMethod = before.loadClass("pkg.TestShortCircuitLoopInitialization").getMethod("scan", parameters);
      Method decompiledMethod = after.loadClass("pkg.TestShortCircuitLoopInitialization").getMethod("scan", parameters);
      for (String input : new String[]{"", " ", " : ,", "x", " :x", "'", "\"", "[", "{", "\uffff"}) {
        for (int start = 0; start <= input.length(); start++) {
          Object expected = originalMethod.invoke(null, null, input, start, input.length());
          assertEquals(expected, decompiledMethod.invoke(null, null, input, start, input.length()), input + " at " + start);
        }
      }
    }
  }
}
