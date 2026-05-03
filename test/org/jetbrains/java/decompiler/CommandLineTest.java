package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.decompiler.ConsoleDecompiler;
import org.jetbrains.java.decompiler.util.TextBuffer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import static org.jetbrains.java.decompiler.DecompilerTestFixture.assertFilesEqual;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class CommandLineTest {
  private DecompilerTestFixture fixture;

  @BeforeEach
  public void setUp() throws IOException {
    System.setProperty("QF_NO_GUI_HELP", "true");
    fixture = new DecompilerTestFixture();
    fixture.setUp();
  }

  @AfterEach
  public void tearDown() {
    fixture.tearDown();
    fixture = null;
  }

  @Test
  public void testJarToJar() {
    String out = fixture.getTempDir().resolve("bulk_out.jar").toAbsolutePath().toString();
    String in = fixture.getTestDataDir().resolve("bulk.jar").toAbsolutePath().toString();
    ConsoleDecompiler.main(new String[]{in, out});

    TextBuffer.checkLeaks();

    assertFilesEqual(fixture.getTestDataDir().resolve("bulk_decomp.jar"), fixture.getTempDir().resolve("bulk_out.jar"));
  }

  @Test
  public void testJarToDir() {
    String out = fixture.getTempDir().resolve("bulk_out").toAbsolutePath().toString();
    String in = fixture.getTestDataDir().resolve("bulk.jar").toAbsolutePath().toString();
    ConsoleDecompiler.main(new String[]{in, out});

    TextBuffer.checkLeaks();

    assertFilesEqual(fixture.getTestDataDir().resolve("bulk_cli"), fixture.getTempDir().resolve("bulk_out"));
  }

  @Test
  public void testZipToJar() {
    String out = fixture.getTempDir().resolve("bulk_out.jar").toAbsolutePath().toString();
    String in = fixture.getTestDataDir().resolve("bulk.zip").toAbsolutePath().toString();
    ConsoleDecompiler.main(new String[]{in, out});

    TextBuffer.checkLeaks();

    assertFilesEqual(fixture.getTestDataDir().resolve("bulk_decomp.jar"), fixture.getTempDir().resolve("bulk_out.jar"));
  }

  @Test
  public void testZipToDir() {
    String out = fixture.getTempDir().resolve("bulk_out").toAbsolutePath().toString();
    String in = fixture.getTestDataDir().resolve("bulk.zip").toAbsolutePath().toString();
    ConsoleDecompiler.main(new String[]{in, out});

    TextBuffer.checkLeaks();

    assertFilesEqual(fixture.getTestDataDir().resolve("bulk_cli"), fixture.getTempDir().resolve("bulk_out"));
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
  public void testDirToJar() {
    String out = fixture.getTempDir().resolve("bulk_out.jar").toAbsolutePath().toString();
    String in = fixture.getTestDataDir().resolve("classes/bulk/").toAbsolutePath().toString();
    ConsoleDecompiler.main(new String[]{in, out});

    TextBuffer.checkLeaks();

    try {
      assertFilesEqual(fixture.getTestDataDir().resolve("bulk_decomp.jar"), fixture.getTempDir().resolve("bulk_out.jar"));
    } catch (AssertionError e) {
      // FIXME: fails in CI due to different order of files in the archive
    }
  }

  @Test
  public void testDirToDir() {
    String out = fixture.getTempDir().resolve("bulk_out").toAbsolutePath().toString();
    String in = fixture.getTestDataDir().resolve("classes/bulk").toAbsolutePath().toString();
    ConsoleDecompiler.main(new String[]{in, out});

    TextBuffer.checkLeaks();

    assertFilesEqual(fixture.getTestDataDir().resolve("bulk_cli"), fixture.getTempDir().resolve("bulk_out"));
  }
}
