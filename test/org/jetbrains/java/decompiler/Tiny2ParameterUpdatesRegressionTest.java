package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.nio.file.Files;
import java.util.List;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

class Tiny2ParameterUpdatesRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
      tiny\t2\t0\tofficial\tnamed
      c\tUpdates\tUpdates
      \tm\t(Ljava/lang/String;)I\tpath\tpath
      \t\tp\t0\t\tresourceName
      \tm\t(II)I\tcapacity\tcapacity
      \t\tp\t0\t\tcapacity
      \t\tp\t1\t\tminimum
      \tm\t(IIIIZ)I\trectangle\trectangle
      \t\tp\t0\t\tx
      \t\tp\t1\t\ty
      \t\tp\t2\t\twidth
      \t\tp\t3\t\theight
      \t\tp\t4\t\trotate
      \tm\t(IIII)I\tconsume\tconsume
      \t\tp\t0\t\tx
      \t\tp\t1\t\ty
      \t\tp\t2\t\twidth
      \t\tp\t3\t\theight
      \tm\t(I)I\tsize\tsize
      \t\tp\t0\t\tcapacity
      \tm\t(III)I\trepurpose\trepurpose
      \t\tp\t0\t\tdeltaMillis
      \t\tp\t1\t\tspeed
      \t\tp\t2\t\tlimit
      \tm\t(II)I\tintervening\tintervening
      \t\tp\t0\t\twidth
      \t\tp\t1\t\theight
      \tm\t(I)I\tsaved\tsaved
      \t\tp\t0\t\tcount
      """;
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void keepsUpdatesReadableAndSeparatesIndependentOverwrites(boolean renameCalleeParameters) throws Exception {
    if (renameCalleeParameters) {
      String mappings = tinyMappings();
      int start = mappings.indexOf("\tm\t(IIII)I\tconsume\tconsume");
      int end = mappings.indexOf("\tm\t(III)I\trepurpose\trepurpose");
      String callees = mappings.substring(start, end).replaceAll("(?m)(\t\tp\t\\d+\t\t)(\\w+)", "$1callee_$2");
      Path mapping = Files.writeString(mappingDirectory.resolve("renamed-callees.tiny"),
        mappings.substring(0, start) + callees + mappings.substring(end));
      fixture.tearDown();
      fixture.setUp(IFernflowerPreferences.MAPPINGS_PATH, mapping.toString());
    }
    Path input = writeSource("Updates.java", """
      public class Updates {
        public static int path(String value) {
          value = "/" + value;
          return value.length() + value.hashCode();
        }
        public static int capacity(int value, int minimum) {
          value *= 3;
          if (value < minimum) value = minimum;
          return size(value) + size(value);
        }
        public static int rectangle(int x, int y, int width, int height, boolean rotate) {
          int rotatedWidth = rotate ? height : width;
          int rotatedHeight = rotate ? width : height;
          x += 2;
          y += 3;
          width = rotatedWidth;
          height = rotatedHeight;
          return consume(x, y, width, height) + consume(x, y, width, height);
        }
        static int consume(int x, int y, int width, int height) { return x + 3*y + 7*width + 11*height; }
        static int size(int capacity) { return capacity; }
        public static int repurpose(int delta, int speed, int limit) {
          int result = delta;
          delta *= speed;
          result += delta;
          for (delta = 0; delta < limit; delta++) result += delta;
          return result;
        }
        public static int intervening(int value, int other) {
          int adjusted = value + 2;
          value = other;
          int result = value * value;
          value = adjusted;
          return result + value * value;
        }
        public static int saved(int input) {
          int saved = input;
          try { input++; throw new Exception(); }
          catch (Exception failure) { return saved * 31 + input; }
        }
      }
      """);
    compileJava8NoDebug(input, outRoot());
    String source = decompileDirectory(outRoot(), "Updates.java");
    recompile();
    for (Path classes : List.of(outRoot(), fixture.getTempDir().resolve("recompiled-out"))) {
      try (var loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
        Class<?> type = loader.loadClass("Updates");
        for (String text : new String[]{"", "file", "a/b", "\u03bb"}) {
          String path = "/" + text;
          assertEquals(path.length() + path.hashCode(), type.getMethod("path", String.class).invoke(null, text));
        }
        for (int value : new int[]{Integer.MIN_VALUE, -3, 0, 7, Integer.MAX_VALUE}) {
          assertEquals(2 * Math.max(value * 3, 5), type.getMethod("capacity", int.class, int.class).invoke(null, value, 5));
          assertEquals(value + value * 3 + 6, type.getMethod("repurpose", int.class, int.class, int.class).invoke(null, value, 3, 4));
          assertEquals(25 + (value + 2) * (value + 2), type.getMethod("intervening", int.class, int.class).invoke(null, value, 5));
          assertEquals(value * 31 + value + 1, type.getMethod("saved", int.class).invoke(null, value));
          for (boolean rotate : new boolean[]{false, true}) {
            int width = rotate ? 5 : 7, height = rotate ? 7 : 5;
            assertEquals(2 * (value + 2 + 3 * (value + 3) + 7 * width + 11 * height),
              type.getMethod("rectangle", int.class, int.class, int.class, int.class, boolean.class)
                .invoke(null, value, value, 7, 5, rotate));
          }
        }
      }
    }
    assertAll(
      () -> assertTrue(source.contains("resourceName = \"/\" + resourceName"), source),
      () -> assertTrue(source.contains("capacity *= 3"), source),
      () -> assertTrue(source.contains("size(capacity) + size(capacity)"), source),
      () -> assertTrue(source.contains("consume(x, y, width, height)"), source),
      () -> assertTrue(source.contains("deltaMillis *= speed"), source),
      () -> assertFalse(source.contains("width = height"), source),
      () -> assertTrue(Pattern.compile("width = var\\d+;").matcher(source).find(), source),
      () -> assertFalse(source.contains("for (deltaMillis ="), source));
  }
}
