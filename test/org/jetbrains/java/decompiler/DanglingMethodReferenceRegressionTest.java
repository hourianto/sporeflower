package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class DanglingMethodReferenceRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testDanglingStaticMethodReferenceCompiles() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("dangling-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestDanglingMethodReferenceTarget.class"), input.resolve("TestDanglingMethodReferenceTarget.class"));
    Files.copy(jasmClasses.resolve("TestDanglingMethodReferenceCaller.class"), input.resolve("TestDanglingMethodReferenceCaller.class"));

    String target = decompileDirectory(input.getParent(), "pkg/TestDanglingMethodReferenceTarget.java");
    assertTrue(target.contains("missing()"), target);
    assertTrue(target.contains("throw new Error"), target);

    recompile();
  }
}

class DanglingMethodReferenceStubsDisabledRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.EMIT_UNRESOLVED_STATIC_METHOD_STUBS, "0"};
  }

  @Test
  public void testDanglingStaticMethodReferenceStubCanBeDisabled() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("dangling-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestDanglingMethodReferenceTarget.class"), input.resolve("TestDanglingMethodReferenceTarget.class"));
    Files.copy(jasmClasses.resolve("TestDanglingMethodReferenceCaller.class"), input.resolve("TestDanglingMethodReferenceCaller.class"));

    String target = decompileDirectory(input.getParent(), "pkg/TestDanglingMethodReferenceTarget.java");
    assertFalse(target.contains("missing()"), target);
    assertFalse(target.contains("throw new Error"), target);
  }
}
