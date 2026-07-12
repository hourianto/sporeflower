package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class SourceLevelOverrideAndNullReceiverRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testPrimitiveReturnDoesNotReceiveFalseOverrideAnnotation() throws IOException {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve("primitive-return/pkg");
    Files.createDirectories(input);
    Files.copy(classes.resolve("TestPrimitiveReturnBase.class"), input.resolve("TestPrimitiveReturnBase.class"));
    Files.copy(classes.resolve("TestPrimitiveReturnChild.class"), input.resolve("TestPrimitiveReturnChild.class"));

    String content = decompileDirectory(input.getParent(), "pkg/TestPrimitiveReturnChild.java");
    int method = content.indexOf("int value()");
    assertTrue(method >= 0, content);
    int previousMethod = content.lastIndexOf('}', method);
    assertFalse(content.substring(Math.max(0, previousMethod), method).contains("@Override"), content);
  }

  @Test
  public void testPrimitiveArrayReturnOverridesObjectReturn() throws IOException {
    String content = compileDecompileAndRead("pkg/ArrayCovariantOverride.java", """
      package pkg;

      public class ArrayCovariantOverride {
        static class Base {
          Object value() {
            return null;
          }
        }

        static class Child extends Base {
          @Override
          int[] value() {
            return null;
          }
        }
      }
      """);

    int method = content.indexOf("int[] value()");
    assertTrue(method >= 0, content);
    assertTrue(content.lastIndexOf("@Override", method) >= 0, content);
    recompile();
  }

  @Test
  public void testRawNullReceiverIsRenderedWithReceiverTypeCast() throws IOException {
    Path source = writeSource("pkg/TypedNullReceiver.java", """
      package pkg;

      public class TypedNullReceiver {
        static class Target {
          void call() {
          }
        }

        public void test() {
          ((Target)null).call();
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());
    removeCheckcastAfterAconstNull(outRoot().resolve("pkg/TypedNullReceiver.class"));

    String content = decompileDirectory(outRoot(), "pkg/TypedNullReceiver.java");
    assertTrue(content.contains("((TypedNullReceiver.Target)null).call();"), content);
    assertFalse(content.contains("null.call();"), content);

    recompile();
  }

  private static void removeCheckcastAfterAconstNull(Path classFile) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    boolean patched = false;
    for (int i = 0; i <= bytes.length - 7; i++) {
      if (Byte.toUnsignedInt(bytes[i]) == 0x01
        && Byte.toUnsignedInt(bytes[i + 1]) == 0xC0
        && Byte.toUnsignedInt(bytes[i + 4]) == 0xB6) {
        bytes[i + 1] = 0;
        bytes[i + 2] = 0;
        bytes[i + 3] = 0;
        patched = true;
        break;
      }
    }

    assertTrue(patched, "Missing aconst_null/checkcast/invokevirtual sequence in " + classFile);
    Files.write(classFile, bytes);
  }
}
