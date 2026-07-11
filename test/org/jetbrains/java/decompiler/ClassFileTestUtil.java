package org.jetbrains.java.decompiler;

import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;

import static org.junit.jupiter.api.Assertions.assertTrue;

final class ClassFileTestUtil {
  private ClassFileTestUtil() {
  }

  static int u1(byte[] bytes, int offset) {
    return bytes[offset] & 0xFF;
  }

  static int u2(byte[] bytes, int offset) {
    return ((bytes[offset] & 0xFF) << 8) | (bytes[offset + 1] & 0xFF);
  }

  static int u4(byte[] bytes, int offset) {
    return (u2(bytes, offset) << 16) | u2(bytes, offset + 2);
  }

  static void putU2(byte[] bytes, int offset, int value) {
    bytes[offset] = (byte)((value >>> 8) & 0xFF);
    bytes[offset + 1] = (byte)(value & 0xFF);
  }

  static int skipMember(byte[] bytes, int offset) {
    offset += 6; // access_flags, name_index, descriptor_index
    int attributesCount = u2(bytes, offset);
    offset += 2;
    for (int i = 0; i < attributesCount; i++) {
      int attrLength = u4(bytes, offset + 2);
      offset += 6 + attrLength;
    }
    return offset;
  }

  static byte[] appendLocalVariableTableEntry(
    byte[] bytes,
    String methodName,
    String methodDescriptor,
    int startPc,
    int slotIndex,
    String localName,
    String localDescriptor
  ) {
    assertTrue(u4(bytes, 0) == 0xCAFEBABE, "Not a class file");

    int cpCount = u2(bytes, 8);
    String[] utf8 = new String[cpCount + 3];
    int cpEnd = readConstantPool(bytes, cpCount, utf8);

    int localVariableTableIndex = cpCount;
    int localNameIndex = cpCount + 1;
    int localDescriptorIndex = cpCount + 2;

    ByteArrayOutputStream out = new ByteArrayOutputStream(bytes.length + 64);
    out.write(bytes, 0, 8);
    writeU2(out, cpCount + 3);
    out.write(bytes, 10, cpEnd - 10);
    writeUtf8(out, "LocalVariableTable");
    writeUtf8(out, localName);
    writeUtf8(out, localDescriptor);

    int pos = cpEnd;
    pos += 6; // access_flags, this_class, super_class
    int interfacesCount = u2(bytes, pos);
    pos += 2 + interfacesCount * 2;

    int fieldsCount = u2(bytes, pos);
    pos += 2;
    for (int i = 0; i < fieldsCount; i++) {
      pos = skipMember(bytes, pos);
    }

    int methodsCount = u2(bytes, pos);
    pos += 2;
    out.write(bytes, cpEnd, pos - cpEnd);

    boolean patched = false;
    for (int i = 0; i < methodsCount; i++) {
      int methodStart = pos;
      int nameIndex = u2(bytes, pos + 2);
      int descriptorIndex = u2(bytes, pos + 4);
      int attributesCount = u2(bytes, pos + 6);
      pos += 8;

      boolean targetMethod = methodName.equals(utf8[nameIndex]) && methodDescriptor.equals(utf8[descriptorIndex]);
      if (!targetMethod) {
        pos = skipAttributes(bytes, pos, attributesCount);
        out.write(bytes, methodStart, pos - methodStart);
        continue;
      }

      out.write(bytes, methodStart, 8);
      for (int attr = 0; attr < attributesCount; attr++) {
        int attrStart = pos;
        int attrNameIndex = u2(bytes, pos);
        int attrLength = u4(bytes, pos + 2);
        int attrInfoStart = pos + 6;
        int attrEnd = attrInfoStart + attrLength;
        pos = attrEnd;

        if (!"Code".equals(utf8[attrNameIndex])) {
          out.write(bytes, attrStart, attrEnd - attrStart);
          continue;
        }

        byte[] code = appendCodeLocalVariableTable(
          bytes, attrInfoStart, attrLength, localVariableTableIndex, localNameIndex, localDescriptorIndex, startPc, slotIndex);
        out.write(bytes, attrStart, 2);
        writeU4(out, code.length);
        out.write(code, 0, code.length);
        patched = true;
      }
    }

    out.write(bytes, pos, bytes.length - pos);
    assertTrue(patched, "Missing method " + methodName + methodDescriptor + " in class fixture");
    return out.toByteArray();
  }

  private static int readConstantPool(byte[] bytes, int cpCount, String[] utf8) {
    int pos = 10;
    for (int i = 1; i < cpCount; i++) {
      int tag = u1(bytes, pos++);
      switch (tag) {
        case 1 -> {
          int length = u2(bytes, pos);
          utf8[i] = new String(bytes, pos + 2, length, StandardCharsets.UTF_8);
          pos += 2 + length;
        }
        case 3, 4, 9, 10, 11, 12, 17, 18 -> pos += 4;
        case 5, 6 -> {
          pos += 8;
          i++;
        }
        case 7, 8, 16, 19, 20 -> pos += 2;
        case 15 -> pos += 3;
        default -> throw new IllegalArgumentException("Unknown constant-pool tag " + tag);
      }
    }
    return pos;
  }

  private static byte[] appendCodeLocalVariableTable(
    byte[] bytes,
    int codeInfoStart,
    int codeInfoLength,
    int lvtAttributeNameIndex,
    int localNameIndex,
    int localDescriptorIndex,
    int startPc,
    int slotIndex
  ) {
    int pos = codeInfoStart + 4;
    int codeLength = u4(bytes, pos);
    pos += 4 + codeLength;
    int exceptionTableLength = u2(bytes, pos);
    pos += 2 + exceptionTableLength * 8;

    int nestedCountPos = pos;
    int nestedAttributesCount = u2(bytes, pos);
    pos += 2;
    int nestedAttributesStart = pos;
    pos = skipAttributes(bytes, pos, nestedAttributesCount);
    assertTrue(pos == codeInfoStart + codeInfoLength, "Malformed Code attribute");
    assertTrue(startPc < codeLength, "Local variable start is outside the method code");

    ByteArrayOutputStream out = new ByteArrayOutputStream(codeInfoLength + 18);
    out.write(bytes, codeInfoStart, nestedCountPos - codeInfoStart);
    writeU2(out, nestedAttributesCount + 1);
    out.write(bytes, nestedAttributesStart, pos - nestedAttributesStart);
    writeU2(out, lvtAttributeNameIndex);
    writeU4(out, 12);
    writeU2(out, 1);
    writeU2(out, startPc);
    writeU2(out, codeLength - startPc);
    writeU2(out, localNameIndex);
    writeU2(out, localDescriptorIndex);
    writeU2(out, slotIndex);
    return out.toByteArray();
  }

  private static int skipAttributes(byte[] bytes, int pos, int attributesCount) {
    for (int i = 0; i < attributesCount; i++) {
      pos += 2;
      int length = u4(bytes, pos);
      pos += 4 + length;
    }
    return pos;
  }

  private static void writeU2(ByteArrayOutputStream out, int value) {
    assertTrue(value >= 0 && value <= 0xFFFF, "Value does not fit in u2: " + value);
    out.write((value >>> 8) & 0xFF);
    out.write(value & 0xFF);
  }

  private static void writeU4(ByteArrayOutputStream out, int value) {
    out.write((value >>> 24) & 0xFF);
    out.write((value >>> 16) & 0xFF);
    out.write((value >>> 8) & 0xFF);
    out.write(value & 0xFF);
  }

  private static void writeUtf8(ByteArrayOutputStream out, String value) {
    byte[] data = value.getBytes(StandardCharsets.UTF_8);
    out.write(1);
    writeU2(out, data.length);
    out.write(data, 0, data.length);
  }
}
