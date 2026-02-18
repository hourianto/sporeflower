package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class OverrideAccessWideningRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testIllegalWeakerOverrideAccessIsWidenedForCompilation() throws IOException {
    Path baseSource = writeSource("pkg/Base.java", """
      package pkg;

      public class Base {
        public void ping() {
        }
      }
      """);
    Path childSource = writeSource("pkg/Child.java", """
      package pkg;

      public class Child extends Base {
        @Override
        public void ping() {
        }
      }
      """);

    compileJava8NoDebug(java.util.List.of(baseSource, childSource), outRoot());

    Path childClass = outRoot().resolve("pkg/Child.class");
    assertTrue(Files.isRegularFile(childClass), "Missing compiled class: " + childClass);
    patchMethodVisibility(childClass, "ping", "()V", CodeConstants.ACC_PROTECTED);

    String content = decompileDirectory(outRoot(), "pkg/Child.java");
    assertFalse(content.contains("protected void ping()"), content);
    assertTrue(content.contains("public void ping()"), content);

    recompile();
  }

  private static void patchMethodVisibility(Path classFile, String targetName, String targetDescriptor, int visibilityFlag) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    if (ClassFileTestUtil.u4(bytes, 0) != 0xCAFEBABE) {
      throw new IOException("Not a class file: " + classFile);
    }

    int cpCount = ClassFileTestUtil.u2(bytes, 8);
    Map<Integer, String> utf8 = new HashMap<>();
    int offset = 10;
    for (int i = 1; i < cpCount; i++) {
      int tag = ClassFileTestUtil.u1(bytes, offset++);
      switch (tag) {
        case 1 -> {
          int len = ClassFileTestUtil.u2(bytes, offset);
          offset += 2;
          utf8.put(i, new String(bytes, offset, len, StandardCharsets.UTF_8));
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

    offset += 6; // access_flags, this_class, super_class
    int interfacesCount = ClassFileTestUtil.u2(bytes, offset);
    offset += 2 + interfacesCount * 2;

    int fieldsCount = ClassFileTestUtil.u2(bytes, offset);
    offset += 2;
    for (int i = 0; i < fieldsCount; i++) {
      offset = ClassFileTestUtil.skipMember(bytes, offset);
    }

    int methodsCount = ClassFileTestUtil.u2(bytes, offset);
    offset += 2;

    boolean patched = false;
    for (int i = 0; i < methodsCount; i++) {
      int accessOffset = offset;
      int oldAccess = ClassFileTestUtil.u2(bytes, offset);
      int nameIndex = ClassFileTestUtil.u2(bytes, offset + 2);
      int descriptorIndex = ClassFileTestUtil.u2(bytes, offset + 4);
      String methodName = utf8.get(nameIndex);
      String descriptor = utf8.get(descriptorIndex);

      if (targetName.equals(methodName) && targetDescriptor.equals(descriptor)) {
        int newAccess = (oldAccess & ~(
          CodeConstants.ACC_PUBLIC | CodeConstants.ACC_PROTECTED | CodeConstants.ACC_PRIVATE
        )) | visibilityFlag;
        ClassFileTestUtil.putU2(bytes, accessOffset, newAccess);
        patched = true;
      }

      offset = ClassFileTestUtil.skipMember(bytes, offset);
    }

    assertTrue(patched, "Failed to patch method access for " + targetName + targetDescriptor + " in " + classFile);
    Files.write(classFile, bytes);
  }
}
