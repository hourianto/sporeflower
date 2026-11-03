package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class NamedPackageReferencesDefaultPackageRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testDecompiledSourcesRemainCompilableWhenPackagedClassReferencesDefaultPackageClass() throws IOException {
    Path classDir = fixture.getTestDataDir().resolve("classes/jasm");
    Path defaultClass = classDir.resolve("a.class");
    Path packagedClass = classDir.resolve("pkg/b.class");
    Path packagedConsumerClass = classDir.resolve("app/c.class");
    assertTrue(Files.isRegularFile(defaultClass), "Missing test class: " + defaultClass);
    assertTrue(Files.isRegularFile(packagedClass), "Missing test class: " + packagedClass);
    assertTrue(Files.isRegularFile(packagedConsumerClass), "Missing test class: " + packagedConsumerClass);

    fixture.getDecompiler().addSource(defaultClass.toFile());
    fixture.getDecompiler().addSource(packagedClass.toFile());
    fixture.getDecompiler().addSource(packagedConsumerClass.toFile());
    fixture.getDecompiler().decompileContext();

    readAllDecompiled();
    recompile();
  }
}
