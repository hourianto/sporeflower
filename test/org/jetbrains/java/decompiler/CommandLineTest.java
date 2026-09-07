package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.decompiler.ConsoleDecompiler;
import org.jetbrains.java.decompiler.util.TextBuffer;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import static org.jetbrains.java.decompiler.DecompilerTestFixture.assertFilesEqual;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class CommandLineTest extends DecompileRegressionTestBase {
  @ParameterizedTest(name = "{0} -> {1}")
  @CsvSource({
    "bulk.jar,     bulk_out.jar, bulk_decomp.jar",
    "bulk.zip,     bulk_out.jar, bulk_decomp.jar",
    "classes/bulk, bulk_out.jar, bulk_decomp.jar",
    "bulk.jar,     bulk_out,     bulk_cli",
    "bulk.zip,     bulk_out,     bulk_cli",
    "classes/bulk, bulk_out,     bulk_cli"
  })
  public void decompileArchivesAndDirectories(String input, String output, String expected) {
    Path destination = fixture.getTempDir().resolve(output);
    ConsoleDecompiler.main(new String[]{fixture.getTestDataDir().resolve(input).toString(), destination.toString()});
    TextBuffer.checkLeaks();
    assertFilesEqual(fixture.getTestDataDir().resolve(expected), destination);
  }

  @Test
  public void testJarToDirSkipsUnsafeEntries() throws IOException {
    Path archive = fixture.getTempDir().resolve("unsafe.jar");
    try (ZipOutputStream out = new ZipOutputStream(Files.newOutputStream(archive))) {
      out.putNextEntry(new ZipEntry("good.txt"));
      out.write("ok".getBytes(StandardCharsets.UTF_8));
      out.closeEntry();

      out.putNextEntry(new ZipEntry("../escaped.txt"));
      out.write("bad".getBytes(StandardCharsets.UTF_8));
      out.closeEntry();
    }

    Path output = fixture.getTempDir().resolve("unsafe_out");
    ConsoleDecompiler.main(new String[]{archive.toAbsolutePath().toString(), output.toAbsolutePath().toString()});

    assertTrue(Files.exists(output.resolve("good.txt")));
    assertFalse(Files.exists(fixture.getTempDir().resolve("escaped.txt")));
  }

  @Test
  public void testJarToDirCopiesNonClassClassEntries() throws IOException {
    Path archive = fixture.getTempDir().resolve("fake-class-resource.jar");
    try (ZipOutputStream out = new ZipOutputStream(Files.newOutputStream(archive))) {
      out.putNextEntry(new ZipEntry("main.class"));
      out.write(new byte[]{(byte)0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A});
      out.closeEntry();
    }

    Path output = fixture.getTempDir().resolve("fake_class_resource_out");
    ConsoleDecompiler.main(new String[]{archive.toAbsolutePath().toString(), output.toAbsolutePath().toString()});

    assertTrue(Files.exists(output.resolve("main.class")));
  }

}
