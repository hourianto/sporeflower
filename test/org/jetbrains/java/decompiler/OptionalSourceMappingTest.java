package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.nio.file.Path;
import java.util.concurrent.atomic.AtomicReference;

import static org.junit.jupiter.api.Assertions.*;

public class OptionalSourceMappingTest extends DecompileRegressionTestBase {
  @Test
  void disablingSourceMapsPreservesJavaAndEnabledMapsReachTheSaver() throws Exception {
    Path source = writeSource("MappingSubject.java", """
      public class MappingSubject {
        public static String describe(String prefix, int count) {
          int next = count + 1;
          return prefix + ": " + next + " remaining entries";
        }
      }
      """);
    compileJava8WithDebug(source, outRoot());
    SavedSource mapped = decompile(true);
    SavedSource plain = decompile(false);
    assertEquals(mapped.content(), plain.content());
    assertNull(plain.mapping());
    assertNotNull(mapped.mapping());
    assertTrue(mapped.mapping().length > 0);
    assertEquals(0, mapped.mapping().length % 2);
    String[] lines = mapped.content().split("\n");
    boolean mappedReturn = false;
    for (int i = 0; i < mapped.mapping().length; i += 2) {
      int generatedLine = mapped.mapping()[i + 1];
      assertTrue(generatedLine > 0 && generatedLine <= lines.length);
      if (mapped.mapping()[i] == 4) {
        assertTrue(lines[generatedLine - 1].contains("return"), lines[generatedLine - 1]);
        mappedReturn = true;
      }
    }
    assertTrue(mappedReturn, "Original return line should have a source mapping");
  }

  private SavedSource decompile(boolean mappings) {
    AtomicReference<SavedSource> saved = new AtomicReference<>();
    Decompiler.builder().inputs(outRoot().toFile())
      .option(IFernflowerPreferences.BYTECODE_SOURCE_MAPPING, mappings ? "1" : "0")
      .option(IFernflowerPreferences.PREFERRED_LINE_LENGTH, "40")
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()) {
        @Override
        public void saveClassFile(String path, String qualifiedName, String entryName, String content, int[] mapping) {
          saved.set(new SavedSource(content, mapping));
        }
      }).build().decompile();
    assertNotNull(saved.get());
    return saved.get();
  }

  private record SavedSource(String content, int[] mapping) {}
}
