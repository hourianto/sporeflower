package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ObjectLocalReusedForMonitorThrowableRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1",
      IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, "1"
    };
  }

  @Test
  public void testObjectLocalReusedForMonitorThrowablePreservesLocking() throws Exception {
    String name = "TestObjectLocalReusedForMonitorThrowable";
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTestDataDir().resolve("classes/jasm"), name, 1);
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTestDataDir().resolve("classes/jasm"), name, "runCatchAll", 1);
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestObjectLocalReusedForMonitorThrowable.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestObjectLocalReusedForMonitorThrowable.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("$VF: monitorenter"), content);

    recompile();
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTempDir().resolve("recompiled-out"), name, 1);
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTempDir().resolve("recompiled-out"), name, "runCatchAll", 1);
  }
}
