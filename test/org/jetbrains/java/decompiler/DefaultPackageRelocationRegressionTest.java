package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class DefaultPackageRelocationRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testRelocationMovesDefaultPackageClusterTogether() throws IOException {
    Path classDir = fixture.getTestDataDir().resolve("classes/jasm");
    Path defaultA = classDir.resolve("TestDefaultPackageRelocationA.class");
    Path defaultB = classDir.resolve("TestDefaultPackageRelocationB.class");
    Path packagedConsumer = classDir.resolve("pkg/TestDefaultPackageRelocationConsumer.class");

    assertTrue(Files.isRegularFile(defaultA), "Missing test class: " + defaultA);
    assertTrue(Files.isRegularFile(defaultB), "Missing test class: " + defaultB);
    assertTrue(Files.isRegularFile(packagedConsumer), "Missing test class: " + packagedConsumer);

    fixture.getDecompiler().addSource(defaultA.toFile());
    fixture.getDecompiler().addSource(defaultB.toFile());
    fixture.getDecompiler().addSource(packagedConsumer.toFile());
    fixture.getDecompiler().decompileContext();

    readAllDecompiled();
    recompile();
  }
}
