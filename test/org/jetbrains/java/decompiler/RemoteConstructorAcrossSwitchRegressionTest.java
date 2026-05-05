package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class RemoteConstructorAcrossSwitchRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testRemoteConstructorAcrossSwitchResugarsToNewExpression() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path testClass = jasmClasses.resolve("TestRemoteConstructorAcrossSwitch.class");
    Path boxClass = jasmClasses.resolve("TestRemoteConstructorAcrossSwitchBox.class");
    assertTrue(Files.isRegularFile(testClass), "Missing test class: " + testClass);
    assertTrue(Files.isRegularFile(boxClass), "Missing support class: " + boxClass);

    Path classDir = fixture.getTempDir().resolve("classes");
    Files.createDirectories(classDir.resolve("pkg"));
    Files.copy(testClass, classDir.resolve("pkg/TestRemoteConstructorAcrossSwitch.class"));
    Files.copy(boxClass, classDir.resolve("pkg/TestRemoteConstructorAcrossSwitchBox.class"));

    String content = decompileDirectory(classDir, "pkg/TestRemoteConstructorAcrossSwitch.java");

    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("Unable to resugar constructor"), content);
    assertFalse(content.contains("new TestRemoteConstructorAcrossSwitchBox;"), content);
    assertTrue(content.contains("new TestRemoteConstructorAcrossSwitchBox("), content);
    assertTrue(content.contains("makeDupStore"), content);

    recompile();
  }
}
