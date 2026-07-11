package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class IntLocalBooleanUseRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testStaleIntegerDebugTypeDoesNotOverrideInferredBoolean() throws IOException {
    Path patchedClassFile = addDebugLocal("check", "(II)I", 1, 3, "flag", "I", "boolean");

    String content = decompileClassFile(patchedClassFile, "pkg/TestIntLocalBooleanUse.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.matches("(?s).*\\bboolean\\s+flag\\s*=\\s*false;.*"), content);
    assertFalse(content.matches("(?s).*\\bint\\s+flag\\s*=\\s*(?:false|true);.*"), content);

    recompile();
  }

  @Test
  public void testCompatibleIntegerDebugTypeIsPreserved() throws IOException {
    Path patchedClassFile = addDebugLocal("count", "(I)I", 1, 1, "total", "I", "integer");

    String content = decompileClassFile(patchedClassFile, "pkg/TestIntLocalBooleanUse.java");
    assertTrue(content.matches("(?s).*\\bint\\s+total\\s*=\\s*0;.*"), content);

    recompile();
  }

  private Path addDebugLocal(
    String methodName,
    String methodDescriptor,
    int startPc,
    int slot,
    String localName,
    String localDescriptor,
    String outputName
  ) throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestIntLocalBooleanUse.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    byte[] patched = ClassFileTestUtil.appendLocalVariableTableEntry(
      Files.readAllBytes(classFile), methodName, methodDescriptor, startPc, slot, localName, localDescriptor);
    Path output = fixture.getTempDir().resolve(outputName + "/pkg/TestIntLocalBooleanUse.class");
    Files.createDirectories(output.getParent());
    Files.write(output, patched);
    return output;
  }
}
