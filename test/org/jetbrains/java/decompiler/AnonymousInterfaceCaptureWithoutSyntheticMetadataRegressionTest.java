package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class AnonymousInterfaceCaptureWithoutSyntheticMetadataRegressionTest extends DecompileRegressionTestBase {
  private static final int ACC_SYNTHETIC = 0x1000;

  @Test
  public void testAnonymousRunnableCaptureWithoutSyntheticFieldFlagsDoesNotLeakCtorArgs() throws IOException {
    Path source = writeSource("pkg/TestAnonymousInterfaceCapture.java", """
package pkg;

public class TestAnonymousInterfaceCapture {
  private int seed = 5;

  public Runnable build(String levelId) {
    final int local = this.seed;
    final String suffix = levelId + "_x";
    return new Runnable() {
      @Override
      public void run() {
        System.out.println(TestAnonymousInterfaceCapture.this.seed + local + suffix.length());
      }
    };
  }
}
""");

    compileJava8NoDebug(source, outRoot());

    Path anonymousClass = outRoot().resolve("pkg/TestAnonymousInterfaceCapture$1.class");
    assertTrue(Files.isRegularFile(anonymousClass), "Missing anonymous class file: " + anonymousClass);
    stripSyntheticFieldAccessFlags(anonymousClass);

    String content = decompileDirectory(outRoot(), "pkg/TestAnonymousInterfaceCapture.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("new Runnable() {"), content);
    assertFalse(Pattern.compile("new\\s+Runnable\\s*\\(\\s*[^)]").matcher(content).find(), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestAnonymousInterfaceCapture.java");
    compileJava8NoDebug(decompiledFile, fixture.getTempDir().resolve("recompiled-out"));
  }

  @Test
  public void testAnonymousSubclassKeepsOnlyRealSuperConstructorArgsWhenSyntheticMetadataMissing() throws IOException {
    Path source = writeSource("pkg/TestAnonymousClassCtorArgs.java", """
package pkg;

public class TestAnonymousClassCtorArgs {
  public Thread build(String name) {
    return new Thread(name) {
      @Override
      public void run() {
      }
    };
  }
}
""");

    compileJava8NoDebug(source, outRoot());

    Path anonymousClass = outRoot().resolve("pkg/TestAnonymousClassCtorArgs$1.class");
    assertTrue(Files.isRegularFile(anonymousClass), "Missing anonymous class file: " + anonymousClass);
    stripSyntheticFieldAccessFlags(anonymousClass);

    String content = decompileDirectory(outRoot(), "pkg/TestAnonymousClassCtorArgs.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("new Thread(var1) {"), content);
    assertFalse(content.contains("new Thread(this"), content);

    Path decompiledFile = fixture.getTargetDir().resolve("pkg/TestAnonymousClassCtorArgs.java");
    compileJava8NoDebug(decompiledFile, fixture.getTempDir().resolve("recompiled-out-super-args"));
  }

  private static void stripSyntheticFieldAccessFlags(Path classFile) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    ByteBuffer buffer = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN);

    if (buffer.getInt() != 0xCAFEBABE) {
      throw new IOException("Not a class file: " + classFile);
    }

    buffer.getShort(); // minor version
    buffer.getShort(); // major version
    skipConstantPool(buffer);

    buffer.position(buffer.position() + 6); // access_flags, this_class, super_class

    int interfacesCount = Short.toUnsignedInt(buffer.getShort());
    buffer.position(buffer.position() + interfacesCount * 2);

    int fieldsCount = Short.toUnsignedInt(buffer.getShort());
    boolean changed = false;

    // Simulate obfuscated/J2ME-like metadata loss by dropping only ACC_SYNTHETIC
    // from fields while keeping constructor descriptors/bytecode intact.
    for (int i = 0; i < fieldsCount; i++) {
      int accessFlagsPos = buffer.position();
      int accessFlags = Short.toUnsignedInt(buffer.getShort());
      int newAccessFlags = accessFlags & ~ACC_SYNTHETIC;
      if (newAccessFlags != accessFlags) {
        buffer.putShort(accessFlagsPos, (short)newAccessFlags);
        changed = true;
      }

      buffer.getShort(); // name_index
      buffer.getShort(); // descriptor_index
      skipAttributes(buffer);
    }

    assertTrue(changed, "Expected at least one synthetic field in " + classFile);
    Files.write(classFile, bytes);
  }

  private static void skipConstantPool(ByteBuffer buffer) {
    int cpCount = Short.toUnsignedInt(buffer.getShort());
    for (int i = 1; i < cpCount; i++) {
      int tag = Byte.toUnsignedInt(buffer.get());
      switch (tag) {
        case 1 -> {
          int length = Short.toUnsignedInt(buffer.getShort());
          buffer.position(buffer.position() + length);
        }
        case 3, 4, 9, 10, 11, 12, 17, 18 -> buffer.position(buffer.position() + 4);
        case 5, 6 -> {
          buffer.position(buffer.position() + 8);
          i++;
        }
        case 7, 8, 16, 19, 20 -> buffer.position(buffer.position() + 2);
        case 15 -> buffer.position(buffer.position() + 3);
        default -> throw new IllegalStateException("Unsupported constant pool tag: " + tag);
      }
    }
  }

  private static void skipAttributes(ByteBuffer buffer) {
    int attributesCount = Short.toUnsignedInt(buffer.getShort());
    for (int i = 0; i < attributesCount; i++) {
      buffer.getShort(); // attribute_name_index
      int length = buffer.getInt();
      buffer.position(buffer.position() + length);
    }
  }
}
