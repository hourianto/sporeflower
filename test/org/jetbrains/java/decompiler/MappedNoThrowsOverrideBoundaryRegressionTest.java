package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class MappedNoThrowsOverrideBoundaryRegressionTest extends DecompileRegressionTestBase {
  private Path mapping;

  @Override
  @BeforeEach
  public void setUp() throws IOException {
    mapping = Files.createTempFile("vf-tiny2-no-throws-override-", ".tiny");
    Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\tpkg/TestMappedNoThrowsTask\tpkg/ImageTask
c\tpkg/TestMappedNoThrowsWorker\tpkg/TaskWorker
\tm\t(Lpkg/TestMappedNoThrowsTask;)V\ta_\tprocessTask
c\tpkg/TestMappedNoThrowsImpl\tpkg/ImageTaskProcessor
\tm\t(Lpkg/TestMappedNoThrowsTask;)V\ta_\tprocessTask
c\tpkg/TestMappedNoThrowsBase\tpkg/BaseTaskProcessor
\tm\t(Lpkg/TestMappedNoThrowsTask;)V\ta_\tprocessTask
c\tpkg/TestMappedNoThrowsSubImpl\tpkg/SubTaskProcessor
\tm\t(Lpkg/TestMappedNoThrowsTask;)V\ta_\tprocessTask
""", StandardCharsets.UTF_8);

    fixture = new DecompilerTestFixture();
    fixture.setUp(
      IFernflowerPreferences.MAPPINGS_PATH, mapping.toString(),
      IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE, "official",
      IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE, "named"
    );
  }

  @Override
  @AfterEach
  public void tearDown() {
    super.tearDown();
    try {
      if (mapping != null) {
        Files.deleteIfExists(mapping);
      }
    }
    catch (IOException ignored) {
    }
  }

  @Test
  public void testMappedNoThrowsInterfaceOverrideWrapsUndeclaredCheckedException() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path taskClass = jasmClasses.resolve("TestMappedNoThrowsTask.class");
    Path workerClass = jasmClasses.resolve("TestMappedNoThrowsWorker.class");
    Path implClass = jasmClasses.resolve("TestMappedNoThrowsImpl.class");
    assertTrue(Files.isRegularFile(taskClass), "Missing test class: " + taskClass);
    assertTrue(Files.isRegularFile(workerClass), "Missing test class: " + workerClass);
    assertTrue(Files.isRegularFile(implClass), "Missing test class: " + implClass);

    Path input = fixture.getTempDir().resolve("mapped-no-throws/pkg");
    Files.createDirectories(input);
    Files.copy(taskClass, input.resolve(taskClass.getFileName()));
    Files.copy(workerClass, input.resolve(workerClass.getFileName()));
    Files.copy(implClass, input.resolve(implClass.getFileName()));

    String content = decompileDirectory(input.getParent(), "pkg/ImageTaskProcessor.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("public void processTask(ImageTask"), content);
    assertFalse(content.contains("processTask(ImageTask var1) throws Exception"), content);
    assertTrue(content.contains("catch (Exception"), content);
    assertTrue(content.contains("throw new RuntimeException("), content);

    recompile();
  }

  @Test
  public void testMappedNoThrowsSuperclassOverrideWrapsUndeclaredCheckedException() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path taskClass = jasmClasses.resolve("TestMappedNoThrowsTask.class");
    Path baseClass = jasmClasses.resolve("TestMappedNoThrowsBase.class");
    Path implClass = jasmClasses.resolve("TestMappedNoThrowsSubImpl.class");
    assertTrue(Files.isRegularFile(taskClass), "Missing test class: " + taskClass);
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(implClass), "Missing test class: " + implClass);

    Path input = fixture.getTempDir().resolve("mapped-no-throws-super/pkg");
    Files.createDirectories(input);
    Files.copy(taskClass, input.resolve(taskClass.getFileName()));
    Files.copy(baseClass, input.resolve(baseClass.getFileName()));
    Files.copy(implClass, input.resolve(implClass.getFileName()));

    String content = decompileDirectory(input.getParent(), "pkg/SubTaskProcessor.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("public void processTask(ImageTask"), content);
    assertFalse(content.contains("processTask(ImageTask var1) throws Exception"), content);
    assertTrue(content.contains("catch (Exception"), content);
    assertTrue(content.contains("throw new RuntimeException("), content);

    recompile();
  }
}
