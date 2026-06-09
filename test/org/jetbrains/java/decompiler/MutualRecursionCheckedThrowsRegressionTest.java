package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Mutually recursive private methods where the checked exception originates in the
 * method that is analyzed first. The cycle guard used to permanently cache the other
 * cycle member with an incomplete "throws nothing" answer, so its rendered signature
 * missed the checked exception and javac reported the call as an unreported exception.
 */
public class MutualRecursionCheckedThrowsRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {
      IFernflowerPreferences.RENAME_ENTITIES, "1",
      IFernflowerPreferences.LEGACY_SOURCE_COMPATIBILITY, "1"
    };
  }

  @Test
  public void testMutualRecursionPropagatesCheckedThrowsAcrossCycle() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestMutualRecursionCheckedThrows.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestMutualRecursionCheckedThrows.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);

    recompile();
  }
}
