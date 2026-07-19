package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ConstructorSelfAliasBeforeSuperRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSelfAliasBeforeSuperDoesNotRenderBeforeConstructorCall() throws Exception {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestConstructorSelfAliasBeforeSuper.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestConstructorSelfAliasBeforeSuper.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    int constructorCall = content.indexOf("super();");
    int receiverCopy = content.indexOf("= this;");
    assertTrue(receiverCopy >= 0 && (constructorCall < 0 || receiverCopy > constructorCall), content);

    recompile();

    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    Path recompiled = fixture.getTempDir().resolve("recompiled-out");
    assertEquals("ready", readValue(original));
    assertEquals("ready", readValue(recompiled));
  }

  private static String readValue(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(
      new URL[]{classes.toUri().toURL()},
      ClassLoader.getPlatformClassLoader()
    )) {
      Class<?> type = Class.forName("pkg.TestConstructorSelfAliasBeforeSuper", true, loader);
      Object instance = type.getConstructor(Object.class).newInstance(new Object());
      return (String)type.getMethod("value").invoke(instance);
    }
  }
}
