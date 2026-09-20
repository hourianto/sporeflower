package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

public class SharedParsingFallbackRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void originalBytecodeRetainsSharedFallback() throws Exception {
    assertParsingBehavior(fixture.getTestDataDir().resolve("classes/jasm"));
  }

  @Test
  public void decompiledPrefixHandlersRetainSharedFallback() throws Exception {
    // Both prefix handlers must still reach the shared continuation after
    // if/else restructuring, including decimal overflow into the long parser.
    // Keep the behavioral check after recompilation: a terminal return null (or
    // returning the token directly) would compile but still parse incorrectly.
    String source = decompileClassFile(
      fixture.getTestDataDir().resolve("classes/jasm/pkg/TestSharedParsingFallback.class"),
      "pkg/TestSharedParsingFallback.java");
    assertFalse(source.contains("$VF: Couldn't be decompiled"), source);
    recompile();
    assertParsingBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertParsingBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Method parse = loader.loadClass("pkg.TestSharedParsingFallback").getMethod("parse", String.class);
      Object[][] cases = {
        // Octal failure must retry decimal, including overflow into the long parser.
        {"09", 9}, {"08", 8}, {"077", 63}, {"0xFF", 255}, {"0Xff", 255},
        {"0xG", "0xG"}, {"0123456789", 123456789}, {"2147483648", 2147483648L},
        {"02147483648", 2147483648L}, {"9223372036854775808", "9223372036854775808"},
        {"0", 0}, {"42", 42}, {"-42", -42}, {"+42", 42}, {".5", ".5"}, {"word", "word"}
      };
      for (Object[] entry : cases) {
        Object result = parse.invoke(null, entry[0]);
        assertEquals(entry[1].getClass(), result.getClass(), "Result type for " + entry[0]);
        assertEquals(entry[1], result, "Result value for " + entry[0]);
      }
    }
  }
}
