package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class LocalMergeTypeConstraintsRegressionTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @CsvSource({"false, false", "true, false", "false, true", "true, true"})
  void accumulatedUsesSurviveSuccessiveMergesWithoutAddingCasts(boolean strict, boolean debug) throws Exception {
    fixture.tearDown();
    fixture.setUp(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, strict ? "1" : "0");
    Path input = writeSource("LocalTypes.java", """
      public class LocalTypes {
        static class Base { int value; Base(int value) { this.value = value; } }
        static class Child extends Base {
          Child(int value) { super(value); }
          int child() { return value + 10; }
        }
        static Child child(int value) { return new Child(value); }
        static Base base(int value) { return new Base(value); }
        static int read(Base value) { return value.value; }
        static int overload(Base value) { return 1; }
        static int overload(Child value) { return 2; }
        public static int shared(int input) {
          int result = 0;
          { Base value = child(input); result += overload(value); result += read(value); }
          { Base value = child(input + 1); result += overload(value); result += read(value); }
          { Base value = base(input + 2); result += overload(value); result += read(value); }
          return result;
        }
        static class FieldBase { public int data = 3; }
        static class FieldChild extends FieldBase { public int data = 7; }
        static class FieldGrandChild extends FieldChild { }
        public static int fields(int input) {
          int result = input;
          { FieldBase value = new FieldGrandChild(); result += value.data++; result += value.data; }
          { FieldChild value = new FieldChild(); result += value.data; result += value.data; }
          return result;
        }
        interface Left { int left(); }
        interface Right { int right(); }
        static class Both implements Left, Right {
          final int value;
          Both(int value) { this.value = value; }
          public int left() { return value; }
          public int right() { return value + 1; }
        }
        static int left(Left value) { return value.left(); }
        static int right(Right value) { return value.right(); }
        public static int interfaces(int input) {
          int result = 0;
          { Both value = new Both(input); result += left(value); result += right(value); }
          { Both value = new Both(input + 1); result += left(value); result += right(value); }
          { Object value = new Object(); result += value == null ? 100 : 3; }
          return result;
        }
        public static int arrays(int input) {
          int result = 0;
          { byte[] value = new byte[]{(byte)input}; result += value[0]; result += value.length; }
          { byte[] value = new byte[]{(byte)(input + 1)}; result += value[0]; result += value.length; }
          { Object value = new Object(); result += value == null ? 100 : 3; }
          return result;
        }
        public static int chain(int input) {
          int result = 0;
          { Child value = child(input); result += read(value); result += read(value); }
          { Child value = child(input + 1); result += read(value); result += value.child(); }
          { Base value = base(input + 2); result += read(value); result += read(value); }
          return result;
        }
        public static int parameter(int input) {
          return parameter(base(input), input);
        }
        static int parameter(Base value, int input) {
          int result = overload(value);
          value = child(input);
          return result * 100 + overload(value);
        }
        public static int nullable(boolean useValue) {
          int result;
          {
            Object value = null;
            if (useValue) value = base(3);
            result = value == null ? 1 : read((Base)value);
          }
          { Child value = child(5); result += value.child(); result += read(value); }
          return result;
        }
        public static int arrayThenNullable(int input) {
          int result;
          { byte[] value = new byte[]{(byte)input}; result = value[0] + value.length; }
          Object value = null;
          if (input > 0) value = child(input);
          else if (input < 0) value = base(input);
          return result + (value == null ? 0 : read((Base)value));
        }
      }
      """);
    if (debug) compileJava8WithDebug(input, outRoot());
    else compileJava8NoDebug(input, outRoot());
    String source = decompileDirectory(outRoot(), "LocalTypes.java");
    recompile();
    for (Path classes : List.of(outRoot(), fixture.getTempDir().resolve("recompiled-out"))) {
      try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        Class<?> type = loader.loadClass("LocalTypes");
        for (int value : new int[]{Integer.MIN_VALUE, -10, 0, 1, 127, Integer.MAX_VALUE}) {
          assertEquals(6 * value + 16, type.getMethod("chain", int.class).invoke(null, value));
          assertEquals(101, type.getMethod("parameter", int.class).invoke(null, value));
          assertEquals(3 * value + 6, type.getMethod("shared", int.class).invoke(null, value));
          assertEquals(value + 21, type.getMethod("fields", int.class).invoke(null, value));
          assertEquals((byte)value + 1 + value, type.getMethod("arrayThenNullable", int.class).invoke(null, value));
          assertEquals(4 * value + 7, type.getMethod("interfaces", int.class).invoke(null, value));
          assertEquals((byte)value + (byte)(value + 1) + 5, type.getMethod("arrays", int.class).invoke(null, value));
        }
        assertEquals(21, type.getMethod("nullable", boolean.class).invoke(null, false));
        assertEquals(23, type.getMethod("nullable", boolean.class).invoke(null, true));
      }
    }
    String chain = source.substring(source.indexOf("public static int chain("), source.indexOf("public static int parameter("));
    assertFalse(chain.contains("(LocalTypes.Child)"), "Merging must retain the narrower receiver requirement:\n" + chain);
    assertEquals(1, chain.split("LocalTypes.Child ", -1).length - 1, "Compatible child lifetimes should still share a local:\n" + chain);
    assertFalse(source.contains("(LocalTypes.Left)") || source.contains("(LocalTypes.Right)"), source);
    assertFalse(source.contains("(byte[])"), source);
    String shared = source.substring(source.indexOf("public static int shared("), source.indexOf("public static int fields("));
    assertFalse(shared.contains("(LocalTypes.Base)"), "Use the common API type instead of repeated upcasts:\n" + shared);
    assertEquals(1, shared.split("LocalTypes.Base ", -1).length - 1, shared);
  }
}
