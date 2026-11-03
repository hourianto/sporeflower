package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

public class LocalSlotRecoveryTest extends DecompileRegressionTestBase {
  @Test
  public void testMonitorObjectSlotReusedForThrowablePreservesLocking() throws Exception {
    String name = "TestMonitorObjectSlotReusedForThrowable";
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTestDataDir().resolve("classes/jasm"), name, 3);
    String content = decompileFixture(name, IFernflowerPreferences.RENAME_ENTITIES, "1");
    assertNotContains(content, "$VF: monitorenter");

    recompile();
    MonitorReconstructionTestSupport.assertRunBehavior(recompiledRoot(), name, 3);
  }

  @Test
  public void testObjectLocalReusedForMonitorThrowablePreservesLocking() throws Exception {
    String name = "TestObjectLocalReusedForMonitorThrowable";
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTestDataDir().resolve("classes/jasm"), name, 1);
    MonitorReconstructionTestSupport.assertRunBehavior(fixture.getTestDataDir().resolve("classes/jasm"), name, "runCatchAll", 1);
    String content = decompileFixture(name,
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, "1"
    );
    assertNotContains(content, "$VF: monitorenter");

    recompile();
    MonitorReconstructionTestSupport.assertRunBehavior(recompiledRoot(), name, 1);
    MonitorReconstructionTestSupport.assertRunBehavior(recompiledRoot(), name, "runCatchAll", 1);
  }
}
