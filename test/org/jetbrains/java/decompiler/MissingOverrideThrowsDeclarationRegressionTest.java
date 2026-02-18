package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class MissingOverrideThrowsDeclarationRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testMissingExceptionsAttributeOnOverrideStillRendersInheritedCheckedThrows() throws IOException {
    Path source = writeSource("pkg/TestMissingOverrideThrows.java", """
      package pkg;

      import java.io.IOException;
      import java.io.InputStream;

      public class TestMissingOverrideThrows extends InputStream {
        @Override
        public int read() throws IOException {
          return -1;
        }

        public void probe() {
          try {
            this.read();
          } catch (IOException e) {
          }
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());

    Path classFile = outRoot().resolve("pkg/TestMissingOverrideThrows.class");
    assertTrue(Files.isRegularFile(classFile), "Missing compiled class: " + classFile);
    renameExceptionsAttributeUtf8(classFile);

    String content = decompileDirectory(outRoot(), "pkg/TestMissingOverrideThrows.java");
    assertTrue(content.contains("read() throws IOException"), content);

    recompile();
  }

  private static void renameExceptionsAttributeUtf8(Path classFile) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    if (ClassFileTestUtil.u4(bytes, 0) != 0xCAFEBABE) {
      throw new IOException("Not a class file: " + classFile);
    }

    int cpCount = ClassFileTestUtil.u2(bytes, 8);
    int offset = 10;
    boolean patched = false;
    for (int i = 1; i < cpCount; i++) {
      int tag = ClassFileTestUtil.u1(bytes, offset++);
      switch (tag) {
        case 1 -> {
          int len = ClassFileTestUtil.u2(bytes, offset);
          offset += 2;
          if (len == "Exceptions".length()) {
            String value = new String(bytes, offset, len, StandardCharsets.UTF_8);
            if ("Exceptions".equals(value)) {
              byte[] renamed = "ExcePtions".getBytes(StandardCharsets.UTF_8);
              System.arraycopy(renamed, 0, bytes, offset, renamed.length);
              patched = true;
            }
          }
          offset += len;
        }
        case 3, 4 -> offset += 4;
        case 5, 6 -> {
          offset += 8;
          i++;
        }
        case 7, 8, 16, 19, 20 -> offset += 2;
        case 9, 10, 11, 12, 18 -> offset += 4;
        case 15 -> offset += 3;
        default -> throw new IOException("Unsupported constant pool tag " + tag + " in " + classFile);
      }
    }

    assertTrue(patched, "Failed to patch Exceptions UTF-8 constant in " + classFile);
    Files.write(classFile, bytes);
  }
}
