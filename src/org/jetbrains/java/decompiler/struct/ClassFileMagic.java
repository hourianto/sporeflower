// Copyright 2000-2026 JetBrains s.r.o. and ForgeFlower contributors Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.struct;

final class ClassFileMagic {
  private ClassFileMagic() {
  }

  static boolean isClassFile(byte[] bytes) {
    return bytes.length >= 4 &&
      bytes[0] == (byte)0xCA &&
      bytes[1] == (byte)0xFE &&
      bytes[2] == (byte)0xBA &&
      bytes[3] == (byte)0xBE;
  }
}
