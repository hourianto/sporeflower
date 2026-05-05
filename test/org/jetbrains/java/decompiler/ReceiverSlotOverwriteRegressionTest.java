package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;

public class ReceiverSlotOverwriteRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testReceiverSlotOverwriteDoesNotDecompileToThisAssignment() throws IOException {
    Path source = writeSource("pkg/TestReceiverSlotOverwrite.java", """
package pkg;

public class TestReceiverSlotOverwrite {
  public TestReceiverSlotOverwrite rewrite() {
    TestReceiverSlotOverwrite tmp = this;
    tmp = new TestReceiverSlotOverwrite();
    return tmp;
  }
}
""");

    compileJava8NoDebug(source, outRoot());

    Path classFile = outRoot().resolve("pkg/TestReceiverSlotOverwrite.class");
    patchMethodLocalToLocal0(classFile, "rewrite", "()Lpkg/TestReceiverSlotOverwrite;", 1);

    String content = decompileDirectory(outRoot(), "pkg/TestReceiverSlotOverwrite.java");
    assertFalse(content.contains("this ="), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestReceiverSlotOverwrite.java");
    compileJava8NoDebug(decompiledFile, fixture.getTempDir().resolve("recompiled-out"));
  }

  @Test
  public void testNoOpReceiverStoreIsRemovedInsteadOfPrintedAsThisAssignment() throws IOException {
    Path source = writeSource("pkg/TestReceiverNoopStore.java", """
package pkg;

public class TestReceiverNoopStore {
  public TestReceiverNoopStore identity() {
    TestReceiverNoopStore tmp = this;
    return tmp;
  }
}
""");

    Path outDir = fixture.getTempDir().resolve("compile-out-noop");
    compileJava8NoDebug(source, outDir);

    Path classFile = outDir.resolve("pkg/TestReceiverNoopStore.class");
    patchMethodLocalToLocal0(classFile, "identity", "()Lpkg/TestReceiverNoopStore;", 1);

    String content = decompileDirectory(outDir, "pkg/TestReceiverNoopStore.java");
    assertFalse(content.contains("this ="), content);
    assertTrue(content.contains("return this;"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestReceiverNoopStore.java");
    compileJava8NoDebug(decompiledFile, fixture.getTempDir().resolve("recompiled-out-noop"));
  }

  @Test
  public void testReceiverStoreWithCompanionStackValueDoesNotRenderAsThisAssignment() throws IOException {
    Path source = writeSource("pkg/TestReceiverStackStore.java", """
package pkg;

public class TestReceiverStackStore {
  private int n = 3;

  public TestReceiverStackStore probe() {
    TestReceiverStackStore[] arr = new TestReceiverStackStore[]{new TestReceiverStackStore()};
    TestReceiverStackStore self = this;
    int count = 0;

    for (int i = 0; i < self.n; i++) {
      count++;
    }

    if (count == 0) {
      return self;
    }

    return arr[0];
  }
}
""");

    Path outDir = fixture.getTempDir().resolve("compile-out-stack");
    compileJava8NoDebug(source, outDir);

    Path classFile = outDir.resolve("pkg/TestReceiverStackStore.class");
    patchMethodLocalToLocal0(classFile, "probe", "()Lpkg/TestReceiverStackStore;", 2);

    String content = decompileDirectory(outDir, "pkg/TestReceiverStackStore.java");
    assertFalse(content.contains("this ="), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestReceiverStackStore.java");
    compileJava8NoDebug(decompiledFile, fixture.getTempDir().resolve("recompiled-out-stack"));
  }

  @Test
  public void testBranchReceiverSlotOverwriteReturnPhiDoesNotRenderAsThisAssignment() throws IOException {
    Path source = writeSource("pkg/TestReceiverBranchStore.java", """
package pkg;

class TestReceiverBranchStoreNode {
  public TestReceiverBranchStoreNode target() {
    return this;
  }
}

public class TestReceiverBranchStore extends TestReceiverBranchStoreNode {
  private int selectedIndex = -1;
  private TestReceiverBranchStoreNode[] selectableChildren = new TestReceiverBranchStoreNode[0];

  public TestReceiverBranchStoreNode target() {
    TestReceiverBranchStoreNode target;
    if (this.selectedIndex == -1) {
      target = this;
    } else {
      target = this.selectableChildren[this.selectedIndex].target();
    }
    return target;
  }
}
""");

    Path outDir = fixture.getTempDir().resolve("compile-out-branch");
    compileJava8NoDebug(source, outDir);

    Path classFile = outDir.resolve("pkg/TestReceiverBranchStore.class");
    patchMethodLocalToLocal0(classFile, "target", "()Lpkg/TestReceiverBranchStoreNode;", 1);

    String content = decompileDirectory(outDir, "pkg/TestReceiverBranchStore.java");
    assertFalse(content.contains("this ="), content);

    compileJava8NoDebug(listJavaSources(fixture.getTargetDir()), fixture.getTempDir().resolve("recompiled-out-branch"));
  }

  // --- bytecode patching ---

  private static void patchMethodLocalToLocal0(Path classFile, String targetMethodName, String targetDescriptor, int localIndex) throws IOException {
    patchMethodCode(classFile, targetMethodName, targetDescriptor, (classBytes, codeStart, codeLength) -> patchCodeAttributeLocalToLocal0(classBytes, codeStart, codeLength, localIndex));
  }

  @FunctionalInterface
  private interface CodeAttributePatcher {
    boolean patch(byte[] classBytes, int codeStart, int codeLength);
  }

  private static void patchMethodCode(
    Path classFile,
    String targetMethodName,
    String targetDescriptor,
    CodeAttributePatcher patcher
  ) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    ByteBuffer buffer = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN);

    if (buffer.getInt() != 0xCAFEBABE) {
      fail("Not a class file: " + classFile);
    }

    buffer.getShort(); // minor
    buffer.getShort(); // major

    int cpCount = Short.toUnsignedInt(buffer.getShort());
    String[] utf8 = new String[cpCount];

    for (int i = 1; i < cpCount; i++) {
      int tag = Byte.toUnsignedInt(buffer.get());
      switch (tag) {
        case 1 -> {
          int len = Short.toUnsignedInt(buffer.getShort());
          byte[] data = new byte[len];
          buffer.get(data);
          utf8[i] = new String(data, StandardCharsets.UTF_8);
        }
        case 3, 4, 9, 10, 11, 12, 17, 18 -> buffer.position(buffer.position() + 4);
        case 5, 6 -> {
          buffer.position(buffer.position() + 8);
          i++;
        }
        case 7, 8, 16, 19, 20 -> buffer.position(buffer.position() + 2);
        case 15 -> buffer.position(buffer.position() + 3);
        default -> fail("Unsupported constant pool tag " + tag);
      }
    }

    buffer.position(buffer.position() + 6); // access, this, super

    int interfacesCount = Short.toUnsignedInt(buffer.getShort());
    buffer.position(buffer.position() + interfacesCount * 2);

    int fieldsCount = Short.toUnsignedInt(buffer.getShort());
    for (int i = 0; i < fieldsCount; i++) {
      skipMember(buffer);
    }

    int methodsCount = Short.toUnsignedInt(buffer.getShort());
    boolean patchedMethod = false;

    for (int i = 0; i < methodsCount; i++) {
      buffer.getShort(); // access
      int nameIndex = Short.toUnsignedInt(buffer.getShort());
      int descriptorIndex = Short.toUnsignedInt(buffer.getShort());
      String methodName = utf8[nameIndex];
      String descriptor = utf8[descriptorIndex];

      int attributesCount = Short.toUnsignedInt(buffer.getShort());
      for (int j = 0; j < attributesCount; j++) {
        int attributeNameIndex = Short.toUnsignedInt(buffer.getShort());
        String attributeName = utf8[attributeNameIndex];
        int attributeLength = buffer.getInt();
        int attributeStart = buffer.position();

        if ("Code".equals(attributeName)
          && targetMethodName.equals(methodName)
          && targetDescriptor.equals(descriptor)) {
          int codeLengthPos = attributeStart + 4;
          int codeLength = ByteBuffer.wrap(bytes, codeLengthPos, 4).order(ByteOrder.BIG_ENDIAN).getInt();
          int codeStart = codeLengthPos + 4;
          patchedMethod = patcher.patch(bytes, codeStart, codeLength);
        }

        buffer.position(attributeStart + attributeLength);
      }
    }

    assertTrue(patchedMethod, "Failed to patch " + targetMethodName + targetDescriptor + " in " + classFile);
    Files.write(classFile, bytes);
  }

  private static boolean patchCodeAttributeLocalToLocal0(byte[] classBytes, int codeStart, int codeLength, int localIndex) {
    byte aload = switch (localIndex) {
      case 0 -> 0x2A;
      case 1 -> 0x2B;
      case 2 -> 0x2C;
      case 3 -> 0x2D;
      default -> 0;
    };
    byte astore = switch (localIndex) {
      case 0 -> 0x4B;
      case 1 -> 0x4C;
      case 2 -> 0x4D;
      case 3 -> 0x4E;
      default -> 0;
    };

    int replacements = 0;
    for (int i = 0; i < codeLength; i++) {
      int pos = codeStart + i;
      int opcode = Byte.toUnsignedInt(classBytes[pos]);

      if (aload != 0 && opcode == Byte.toUnsignedInt(aload)) {
        classBytes[pos] = 0x2A;
        replacements++;
      } else if (astore != 0 && opcode == Byte.toUnsignedInt(astore)) {
        classBytes[pos] = 0x4B;
        replacements++;
      } else if ((opcode == 0x19 || opcode == 0x3A) && i + 1 < codeLength) {
        int indexPos = pos + 1;
        if (Byte.toUnsignedInt(classBytes[indexPos]) == localIndex) {
          classBytes[indexPos] = 0;
          replacements++;
          i++;
        }
      }
    }

    return replacements > 0;
  }

  private static void skipMember(ByteBuffer buffer) {
    buffer.position(buffer.position() + 6); // access, name, descriptor
    int attributesCount = Short.toUnsignedInt(buffer.getShort());
    for (int i = 0; i < attributesCount; i++) {
      buffer.getShort(); // name
      int length = buffer.getInt();
      buffer.position(buffer.position() + length);
    }
  }
}
