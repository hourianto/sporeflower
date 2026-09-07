package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

abstract class TinyMappingTestBase extends DecompileRegressionTestBase {
  @TempDir
  Path mappingDirectory;

  protected abstract String tinyMappings();

  @Override
  protected Object[] fixtureOptions() throws IOException {
    Path mapping = Files.writeString(mappingDirectory.resolve("mappings.tiny"), tinyMappings());
    return new Object[]{
      IFernflowerPreferences.MAPPINGS_PATH, mapping.toString(),
      IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE, "official",
      IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE, "named"
    };
  }
}
