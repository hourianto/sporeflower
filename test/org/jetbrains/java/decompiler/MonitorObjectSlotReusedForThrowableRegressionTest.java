package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class MonitorObjectSlotReusedForThrowableRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testMonitorObjectSlotReusedForThrowablePreservesLocking() throws Exception {
    String name = "TestMonitorObjectSlotReusedForThrowable";
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTestDataDir().resolve("classes/jasm"), name, 3);
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestMonitorObjectSlotReusedForThrowable.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestMonitorObjectSlotReusedForThrowable.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("$VF: monitorenter"), content);

    recompile();
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTempDir().resolve("recompiled-out"), name, 3);
  }
}
