package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
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
    assertTrue(Files.isRegularFile(defaultClass), "Missing test class: " + defaultClass);
    assertTrue(Files.isRegularFile(packagedClass), "Missing test class: " + packagedClass);

    fixture.getDecompiler().addSource(defaultClass.toFile());
    fixture.getDecompiler().addSource(packagedClass.toFile());
    fixture.getDecompiler().decompileContext();

    List<Path> decompiledSources = listJavaSources(fixture.getTargetDir());
    assertFalse(decompiledSources.isEmpty(), "No decompiled sources found");

    for (Path source : decompiledSources) {
      String content = DecompilerTestFixture.getContent(source);
      assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    }

    compileJava8(decompiledSources, fixture.getTempDir().resolve("compile-out"));
  }
}
