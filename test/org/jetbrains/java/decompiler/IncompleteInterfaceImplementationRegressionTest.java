package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class IncompleteInterfaceImplementationRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testFinalClassMissingInheritedInterfaceMethodStillCompiles() throws Exception {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("incomplete-interface-input/pkg");
    Files.createDirectories(input);
    Files.copy(jasmClasses.resolve("TestIncompleteInterfaceParent.class"), input.resolve("TestIncompleteInterfaceParent.class"));
    Files.copy(jasmClasses.resolve("TestIncompleteInterfaceChild.class"), input.resolve("TestIncompleteInterfaceChild.class"));
    Files.copy(jasmClasses.resolve("TestIncompleteInterfaceImpl.class"), input.resolve("TestIncompleteInterfaceImpl.class"));

    String content = decompileDirectory(input.getParent(), "pkg/TestIncompleteInterfaceImpl.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("public void draw(Object var0, Object var1, int var2, int var3, int var4, int var5, int var6)"), content);
    assertTrue(content.contains("throw new AbstractMethodError();"), content);

    recompile();

    Class<?>[] parameterTypes = {
      Object.class, Object.class, int.class, int.class, int.class, int.class, int.class
    };
    Object[] arguments = {null, null, 0, 0, 0, 0, 0};
    List<Path> runtimeClasses = List.of(
      fixture.getTestDataDir().resolve("classes/jasm"),
      fixture.getTempDir().resolve("recompiled-out")
    );
    for (Path classes : runtimeClasses) {
      assertInvocationThrows(
        classes,
        AbstractMethodError.class,
        "pkg.TestIncompleteInterfaceImpl",
        new Class<?>[] {Object.class},
        new Object[] {new Object()},
        "pkg.TestIncompleteInterfaceParent",
        "draw",
        parameterTypes,
        arguments
      );
    }
  }
}
