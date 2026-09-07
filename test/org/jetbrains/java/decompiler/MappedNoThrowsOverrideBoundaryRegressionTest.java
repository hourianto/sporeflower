package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class MappedNoThrowsOverrideBoundaryRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
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
""";
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
