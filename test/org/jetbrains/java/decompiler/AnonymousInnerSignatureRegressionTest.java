package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class AnonymousInnerSignatureRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testInnerClassWithAnonymousMetadataButNamedTypeReferenceIsRepresentable() throws IOException {
    Path source = writeSource("pkg/Outer.java", """
      package pkg;

      public class Outer {
        Inner state;

        public void init() {
          this.state = new Inner();
          this.state.value = 1;
        }

        static class Inner {
          int value;
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());

    Path outerClass = outRoot().resolve("pkg/Outer.class");
    Path innerClass = outRoot().resolve("pkg/Outer$Inner.class");
    assertTrue(Files.isRegularFile(outerClass), "Missing compiled class: " + outerClass);
    assertTrue(Files.isRegularFile(innerClass), "Missing compiled class: " + innerClass);

    zeroInnerSimpleNameIndex(outerClass, "pkg/Outer$Inner");
    zeroInnerSimpleNameIndex(innerClass, "pkg/Outer$Inner");

    String content = decompileDirectory(outRoot(), "pkg/Outer.java");
    assertFalse(content.contains("<unrepresentable>"), content);
    assertFalse(content.contains("$VF: Unable to decompile class"), content);

    recompile();
  }

  @Test
  public void testGenericOnlyNestedTypeReferencePreventsUnrepresentableAnonymousFallback() throws IOException {
    Path source = writeSource("pkg/OuterGeneric.java", """
      package pkg;

      import java.util.Collections;
      import java.util.List;

      public class OuterGeneric {
        List<Inner> state;

        public void init() {
          this.state = Collections.singletonList(new Inner());
        }

        public List<Inner> getState() {
          return this.state;
        }

        static class Inner {
          int value;
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());

    Path outerClass = outRoot().resolve("pkg/OuterGeneric.class");
    Path innerClass = outRoot().resolve("pkg/OuterGeneric$Inner.class");
    assertTrue(Files.isRegularFile(outerClass), "Missing compiled class: " + outerClass);
    assertTrue(Files.isRegularFile(innerClass), "Missing compiled class: " + innerClass);

    zeroInnerSimpleNameIndex(outerClass, "pkg/OuterGeneric$Inner");
    zeroInnerSimpleNameIndex(innerClass, "pkg/OuterGeneric$Inner");

    String content = decompileDirectory(outRoot(), "pkg/OuterGeneric.java");
    assertFalse(content.contains("<unrepresentable>"), content);
    assertFalse(content.contains("$VF: Unable to decompile class"), content);

    recompile();
  }

  private static void zeroInnerSimpleNameIndex(Path classFile, String innerClassName) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    if (ClassFileTestUtil.u4(bytes, 0) != 0xCAFEBABE) {
      throw new IOException("Not a class file: " + classFile);
    }

    int cpCount = ClassFileTestUtil.u2(bytes, 8);
    Map<Integer, Object> cpValues = new HashMap<>();
    Map<Integer, Integer> cpTags = new HashMap<>();
    int offset = 10;
    for (int i = 1; i < cpCount; i++) {
      int tag = ClassFileTestUtil.u1(bytes, offset++);
      cpTags.put(i, tag);
      switch (tag) {
        case 1 -> {
          int len = ClassFileTestUtil.u2(bytes, offset);
          offset += 2;
          cpValues.put(i, new String(bytes, offset, len, StandardCharsets.UTF_8));
          offset += len;
        }
        case 3, 4 -> offset += 4;
        case 5, 6 -> {
          offset += 8;
          i++;
        }
        case 7, 8, 16, 19, 20 -> {
          cpValues.put(i, ClassFileTestUtil.u2(bytes, offset));
          offset += 2;
        }
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
    for (int i = 0; i < methodsCount; i++) {
      offset = ClassFileTestUtil.skipMember(bytes, offset);
    }

    int classAttrs = ClassFileTestUtil.u2(bytes, offset);
    offset += 2;
    boolean patched = false;
    for (int i = 0; i < classAttrs; i++) {
      int nameIndex = ClassFileTestUtil.u2(bytes, offset);
      int len = ClassFileTestUtil.u4(bytes, offset + 2);
      int data = offset + 6;
      String attrName = asUtf8(cpValues.get(nameIndex));
      if ("InnerClasses".equals(attrName)) {
        int count = ClassFileTestUtil.u2(bytes, data);
        int item = data + 2;
        for (int j = 0; j < count; j++) {
          int innerClassInfoIndex = ClassFileTestUtil.u2(bytes, item);
          String entryInnerName = resolveClassName(cpValues, cpTags, innerClassInfoIndex);
          if (innerClassName.equals(entryInnerName)) {
            ClassFileTestUtil.putU2(bytes, item + 4, 0); // inner_name_index
            patched = true;
          }
          item += 8;
        }
      }
      offset += 6 + len;
    }

    assertTrue(patched, "Failed to patch InnerClasses in " + classFile);
    Files.write(classFile, bytes);
  }

  private static String resolveClassName(Map<Integer, Object> cpValues, Map<Integer, Integer> cpTags, int classIndex) {
    if (cpTags.getOrDefault(classIndex, -1) != 7) {
      return null;
    }
    Object nameIndexObj = cpValues.get(classIndex);
    if (!(nameIndexObj instanceof Integer)) {
      return null;
    }
    return asUtf8(cpValues.get((Integer)nameIndexObj));
  }

  private static String asUtf8(Object value) {
    return value instanceof String ? (String)value : null;
  }
}
