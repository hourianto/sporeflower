package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ResidualInterfaceInitializerRenameRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testDescriptorDistinctFieldsDoNotBlockResidualSliceExtraction() throws Exception {
    String interfaceName = "TestResidualInterfaceInitializerOverloadedFields";
    String probeName = "TestResidualInterfaceInitializerProbe";
    Path input = fixture.getTempDir().resolve("jasm-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClass(interfaceName), input.resolve(interfaceName + ".class"));
    Files.copy(jasmClass(probeName), input.resolve(probeName + ".class"));

    String content = decompileDirectory(input.getParent(), "pkg/" + interfaceName + ".java");
    assertFalse(content.contains("\n   static {"), "An interface initializer block is illegal source:\n" + content);
    assertTrue(content.contains("final class VFInterfaceInitializer"), content);
    assertTrue(content.contains("field_"), "The duplicate source field names must be disambiguated:\n" + content);

    assertEquals(1234, initializeAndReadLog(fixture.getTestDataDir().resolve("classes/jasm"), interfaceName, probeName));
    recompile();
    assertEquals(1234, initializeAndReadLog(fixture.getTempDir().resolve("recompiled-out"), interfaceName, probeName));
  }

  @Test
  public void testDefaultPackageHolderNeedsNoSelfImport() throws Exception {
    String interfaceName = "TestDefaultResidualInterfaceInitializer";
    Path input = fixture.getTempDir().resolve("default-input");
    Files.createDirectories(input);
    Files.copy(fixture.getTestDataDir().resolve("classes/jasm/" + interfaceName + ".class"),
      input.resolve(interfaceName + ".class"));

    String content = decompileDirectory(input, interfaceName + ".java");
    assertFalse(content.contains("import " + interfaceName), content);
    assertFalse(content.contains("\n   static {"), content);
    assertTrue(content.contains("VFInterfaceInitializer.$VF$init0()"), content);
    recompile();
  }

  private Path jasmClass(String simpleName) {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/" + simpleName + ".class");
    assertTrue(Files.isRegularFile(classFile), "Missing JASM fixture: " + classFile);
    return classFile;
  }

  private static int initializeAndReadLog(Path root, String interfaceName, String probeName) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(
      new URL[] {root.toUri().toURL()},
      ClassLoader.getPlatformClassLoader()
    )) {
      Class<?> probe = Class.forName("pkg." + probeName, true, loader);
      probe.getField("log").setInt(null, 0);
      Class.forName("pkg." + interfaceName, true, loader);
      return probe.getField("log").getInt(null);
    }
  }
}
