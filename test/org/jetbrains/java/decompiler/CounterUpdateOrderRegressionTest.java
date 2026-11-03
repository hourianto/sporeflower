package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.Random;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class CounterUpdateOrderRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void paddingChecksSeeBothEarlierCounterUpdates() throws Exception {
    Path source = writeSource("pkg/CounterUpdates.java", """
      package pkg;
      public class CounterUpdates {
        private static final char[] alphabet =
          "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".toCharArray();

        public static String encode(byte[] input) {
          int length;
          int contentLength = ((length = input.length) * 4 + 2) / 3;
          int outputLength = (length + 2) / 3 * 4;
          StringBuffer output = new StringBuffer(outputLength);
          int inputPosition = 0;
          int outputPosition = 0;
          while (inputPosition < length) {
            int first = input[inputPosition++] & 255;
            int second = inputPosition < length ? input[inputPosition++] & 255 : 0;
            int third = inputPosition < length ? input[inputPosition++] & 255 : 0;
            int a = first >>> 2;
            int b = (first & 3) << 4 | second >>> 4;
            int c = (second & 15) << 2 | third >>> 6;
            int d = third & 63;
            output.append(alphabet[a]);
            outputPosition++;
            output.append(alphabet[b]);
            outputPosition++;
            output.append(outputPosition < contentLength ? alphabet[c] : '=');
            outputPosition++;
            output.append(outputPosition < contentLength ? alphabet[d] : '=');
            outputPosition++;
          }
          return output.toString();
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    // The failure depends on standalone updates interleaved with appends. Keep
    // this shape faithful even if a future fixture compiler lowers it differently.
    assertEncoderCounterOrder(outRoot().resolve("pkg/CounterUpdates.class"));
    decompileClassFile(outRoot().resolve("pkg/CounterUpdates.class"), "pkg/CounterUpdates.java");
    recompile();
    try (BehaviorComparison behavior = compareBehavior()) {
      checkEncoding(behavior);
    }
  }

  @ParameterizedTest(name = "counter step {0}")
  @ValueSource(ints = {1, -1, 2, -2})
  public void repeatedUpdatesKeepInterveningReads(int delta) throws Exception {
    // Unit steps become increment/decrement expressions; nonunit steps remain
    // assignments. Both can repeat as structurally equal loop expressions.
    String update = delta == 1 ? "position++" : delta == -1 ? "position--" : "position += " + delta;
    Path source = writeSource("pkg/GroupedCounterUpdates.java", """
      package pkg;
      public class GroupedCounterUpdates {
        public static String scan(int rounds, int limit) {
          StringBuffer output = new StringBuffer();
          int iteration = 0;
          int position = %1$d;
          while (iteration++ < rounds) {
            output.append('a');
            %2$s;
            output.append('b');
            %2$s;
            output.append(position %3$s limit ? 'c' : '=');
            %2$s;
            output.append(position %3$s limit ? 'd' : '=');
            %2$s;
          }
          return output.toString();
        }
      }
      """.formatted(delta < 0 ? 20 : 0, update, delta < 0 ? ">" : "<"));
    compileJava8NoDebug(source, outRoot());
    decompileClassFile(outRoot().resolve("pkg/GroupedCounterUpdates.class"), "pkg/GroupedCounterUpdates.java");
    recompile();
    try (BehaviorComparison behavior = compareBehavior()) {
      checkGroupedUpdates(behavior, delta);
    }
  }

  private static void assertEncoderCounterOrder(Path classFile) throws Exception {
    List<String> events = new ArrayList<>();
    new ClassReader(Files.readAllBytes(classFile)).accept(new ClassVisitor(Opcodes.ASM9) {
      @Override
      public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        if (!name.equals("encode")) return null;
        return new MethodVisitor(Opcodes.ASM9) {
          @Override
          public void visitIincInsn(int varIndex, int increment) {
            // Parameters and locals intentionally retain slots 0..13 from the
            // reduced bytecode pattern; slot 6 is the output position.
            if (varIndex == 6) events.add("update " + increment);
          }

          @Override
          public void visitVarInsn(int opcode, int varIndex) {
            if (opcode == Opcodes.ILOAD && varIndex == 6) events.add("read");
          }

          @Override
          public void visitMethodInsn(int opcode, String owner, String name, String descriptor, boolean isInterface) {
            if (owner.equals("java/lang/StringBuffer") && name.equals("append")) events.add("append");
          }
        };
      }
    }, ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
    assertEquals(List.of("append", "update 1", "append", "update 1", "read", "append", "update 1", "read", "append", "update 1"), events);
  }

  private static void checkEncoding(BehaviorComparison behavior) throws Exception {
    for (String text : new String[]{"", "f", "fo", "foo", "foob", "fooba", "foobar"}) {
      checkEncoding(behavior, text.getBytes(StandardCharsets.US_ASCII), "input " + text);
    }
    Random random = new Random(0xC0FFEE);
    // Every remainder modulo three, empty input, and several loop iterations.
    for (int length = 0; length <= 128; length++) {
      byte[] bytes = new byte[length];
      random.nextBytes(bytes);
      checkEncoding(behavior, bytes, "length " + length);
    }
  }

  private static void checkEncoding(BehaviorComparison behavior, byte[] bytes, String context) throws Exception {
    behavior.assertSame(context, Base64.getEncoder().encodeToString(bytes), loader ->
      loader.loadClass("pkg.CounterUpdates").getMethod("encode", byte[].class).invoke(null, (Object)bytes.clone()));
  }

  private static void checkGroupedUpdates(BehaviorComparison behavior, int delta) throws Exception {
    for (int rounds = 0; rounds <= 5; rounds++) {
      for (int limit = -21; limit <= 41; limit++) {
        StringBuilder expected = new StringBuilder();
        for (int iteration = 0; iteration < rounds; iteration++) {
          int thirdPosition = (delta < 0 ? 20 : 0) + delta * (iteration * 4 + 2);
          int fourthPosition = thirdPosition + delta;
          expected.append("ab");
          expected.append((delta < 0 ? thirdPosition > limit : thirdPosition < limit) ? 'c' : '=');
          expected.append((delta < 0 ? fourthPosition > limit : fourthPosition < limit) ? 'd' : '=');
        }
        int count = rounds, bound = limit;
        behavior.assertSame("step=" + delta + " rounds=" + rounds + " limit=" + limit, expected.toString(), loader ->
          loader.loadClass("pkg.GroupedCounterUpdates").getMethod("scan", int.class, int.class).invoke(null, count, bound));
      }
    }
  }
}
