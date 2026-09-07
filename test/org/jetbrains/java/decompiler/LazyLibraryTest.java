package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.decompiler.ConsoleDecompiler;
import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.util.TextBuffer;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;

import static org.jetbrains.java.decompiler.DecompilerTestFixture.assertFilesEqual;

public class LazyLibraryTest extends DecompileRegressionTestBase {
  @Test
  public void testLazy() {
    ConsoleDecompiler decompiler = fixture.getDecompiler();
    decompiler.addSource(fixture.getTestDataDir().resolve("classes").resolve("java8").resolve("pkg").resolve("TestLibraryIsOnClasspath.class").toFile());
    decompiler.addLibrary(new IContextSource() {
      @Override
      public String getName() {
        return "Test Lazy Library";
      }

      @Override
      public Entries getEntries() {
        return Entries.EMPTY;
      }

      @Override
      public boolean isLazy() {
        return true;
      }

      @Override
      public InputStream getInputStream(String resource) throws IOException {
        if ("ext/TestLibrary.class".equals(resource)) {
          return Files.newInputStream(fixture.getTestDataDir().resolve("classes").resolve("java8").resolve("ext").resolve("TestLibrary.class"));
        }
        return null;
      }
    });
    decompiler.decompileContext();

    TextBuffer.checkLeaks();

    assertFilesEqual(fixture.getTestDataDir().resolve("lazyLibrary"), fixture.getTargetDir());
  }
}
