package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.HashMap;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class MalformedExceptionsAttributeRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testMalformedExceptionsEntriesDoNotCrashDecompilation() throws IOException {
    Path brokenSource = writeSource("pkg/Broken.java", """
      package pkg;

      public class Broken {
        private static final int MARK = 7;

        public static void brokenInt() throws java.io.IOException {
        }

        public static void brokenLink() throws java.io.IOException {
        }
      }
      """);
    Path callerSource = writeSource("pkg/Caller.java", """
      package pkg;

      public class Caller {
        public static void probe() {
          try {
            Broken.brokenInt();
            Broken.brokenLink();
          } catch (java.io.IOException e) {
            e.printStackTrace();
          }
        }
      }
      """);

    compileJava8NoDebug(java.util.List.of(brokenSource, callerSource), outRoot());

    Path brokenClass = outRoot().resolve("pkg/Broken.class");
    assertTrue(Files.isRegularFile(brokenClass), "Missing compiled class: " + brokenClass);
    corruptExceptionsEntries(brokenClass);

    String content = decompileDirectory(outRoot(), "pkg/Caller.java");
    assertFalse(content.contains("$VF: Unable to decompile class"), content);
    assertTrue(content.contains("catch (IOException") || content.contains("catch (java.io.IOException"), content);
    assertFalse(content.contains("catch (RuntimeException"), content);
    assertFalse(content.contains("substituted checked catch types"), content);

    // Intentionally malformed checked-throws metadata is not guaranteed to produce
    // source-recompilable output; this regression focuses on robust decompilation
    // and conservative checked-catch rendering.
  }

  @Test
  public void testOutOfRangeExceptionsIndexDoesNotCrashDecompilation() throws IOException {
    Path brokenSource = writeSource("pkg/BrokenRange.java", """
      package pkg;

      public class BrokenRange {
        public static void brokenRange() throws java.io.IOException {
        }
      }
      """);
    Path callerSource = writeSource("pkg/CallerRange.java", """
      package pkg;

      public class CallerRange {
        public static void probe() {
          try {
            BrokenRange.brokenRange();
          } catch (java.io.IOException e) {
            e.printStackTrace();
          }
        }
      }
      """);

    compileJava8NoDebug(java.util.List.of(brokenSource, callerSource), outRoot());

    Path brokenClass = outRoot().resolve("pkg/BrokenRange.class");
    assertTrue(Files.isRegularFile(brokenClass), "Missing compiled class: " + brokenClass);
    corruptExceptionsEntryWithOutOfRangeIndex(brokenClass, "brokenRange");

    String content = decompileDirectory(outRoot(), "pkg/CallerRange.java");
    assertFalse(content.contains("$VF: Unable to decompile class"), content);

    // Intentionally malformed checked-throws metadata is not guaranteed to produce
    // source-recompilable output; this regression focuses on avoiding decompilation crashes.
  }

  private static void corruptExceptionsEntries(Path classFile) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    if (ClassFileTestUtil.u4(bytes, 0) != 0xCAFEBABE) {
      throw new IOException("Not a class file: " + classFile);
    }

    int cpCount = ClassFileTestUtil.u2(bytes, 8);
    Map<Integer, String> utf8 = new HashMap<>();
    int methodRefIndex = -1;
    int integerIndex = -1;

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
        case 3, 4 -> {
          if (tag == 3 && integerIndex < 0) {
            integerIndex = i;
          }
          offset += 4;
        }
        case 5, 6 -> {
          offset += 8;
          i++;
        }
        case 7, 8, 16, 19, 20 -> offset += 2;
        case 9, 10, 11, 12, 18 -> {
          if (tag == 10 && methodRefIndex < 0) {
            methodRefIndex = i;
          }
          offset += 4;
        }
        case 15 -> offset += 3;
        default -> throw new IOException("Unsupported constant pool tag " + tag + " in " + classFile);
      }
    }

    assertTrue(integerIndex > 0, "Expected an Integer constant in " + classFile);
    assertTrue(methodRefIndex > 0, "Expected a Methodref constant in " + classFile);
    Map<String, Integer> replacements = new LinkedHashMap<>();
    replacements.put("brokenInt", integerIndex);
    replacements.put("brokenLink", methodRefIndex);
    patchExceptionEntries(bytes, classFile, utf8, replacements);
    Files.write(classFile, bytes);
  }

  private static void corruptExceptionsEntryWithOutOfRangeIndex(Path classFile, String methodName) throws IOException {
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

    Map<String, Integer> replacements = new LinkedHashMap<>();
    replacements.put(methodName, cpCount); // deliberately invalid CP index
    patchExceptionEntries(bytes, classFile, utf8, replacements);
    Files.write(classFile, bytes);
  }

  private static void patchExceptionEntries(
    byte[] bytes,
    Path classFile,
    Map<Integer, String> utf8,
    Map<String, Integer> replacementByMethod
  ) throws IOException {
    int offset = 10;
    int cpCount = ClassFileTestUtil.u2(bytes, 8);
    for (int i = 1; i < cpCount; i++) {
      int tag = ClassFileTestUtil.u1(bytes, offset++);
      switch (tag) {
        case 1 -> {
          int len = ClassFileTestUtil.u2(bytes, offset);
          offset += 2 + len;
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

    Map<String, Boolean> patched = new LinkedHashMap<>();
    for (String method : replacementByMethod.keySet()) {
      patched.put(method, false);
    }

    for (int i = 0; i < methodsCount; i++) {
      offset += 2; // access_flags
      int nameIndex = ClassFileTestUtil.u2(bytes, offset);
      offset += 2; // name_index
      offset += 2; // descriptor_index
      int attributesCount = ClassFileTestUtil.u2(bytes, offset);
      offset += 2;

      String methodName = utf8.get(nameIndex);
      for (int j = 0; j < attributesCount; j++) {
        int attrNameIndex = ClassFileTestUtil.u2(bytes, offset);
        int attrLength = ClassFileTestUtil.u4(bytes, offset + 2);
        int attrData = offset + 6;
        String attrName = utf8.get(attrNameIndex);
        Integer replacement = replacementByMethod.get(methodName);
        if (replacement != null && "Exceptions".equals(attrName)) {
          int exceptionsCount = ClassFileTestUtil.u2(bytes, attrData);
          if (exceptionsCount > 0) {
            int entryOffset = attrData + 2;
            ClassFileTestUtil.putU2(bytes, entryOffset, replacement);
            patched.put(methodName, true);
          }
        }
        offset += 6 + attrLength;
      }
    }

    for (Map.Entry<String, Boolean> entry : patched.entrySet()) {
      assertTrue(entry.getValue(), "Failed to patch Exceptions for " + entry.getKey() + " in " + classFile);
    }
  }
}
