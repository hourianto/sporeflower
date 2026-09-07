package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.Modifier;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class ReassignedReceiverRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testEntryValueAndLoopUpdateSurviveReceiverSlotReuse() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    checkBehavior(original);
    Path input = outRoot().resolve("pkg");
    Files.createDirectories(input);
    for (String name : new String[]{"TestReassignedReceiver", "TestReassignedReceiverChild"}) {
      Files.copy(original.resolve("pkg/" + name + ".class"), input.resolve(name + ".class"));
    }
    decompileDirectory(outRoot(), "pkg/TestReassignedReceiver.java");
    recompile();
    checkBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void checkBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg.TestReassignedReceiver");
      var constructor = type.getConstructor(int.class, type);
      var childConstructor = loader.loadClass("pkg.TestReassignedReceiverChild").getConstructor(int.class, type);
      Object tail = childConstructor.newInstance(30, null);
      Object middle = childConstructor.newInstance(20, tail);
      Object head = constructor.newInstance(10, middle);
      var select = type.getMethod("select", boolean.class, Object.class);
      Object other = new Object();
      assertSame(head, select.invoke(head, false, other));
      assertSame(other, select.invoke(head, true, other));
      assertSame(null, select.invoke(head, true, null));
      var guarded = type.getMethod("guarded", boolean.class, Object.class);
      assertSame(head, guarded.invoke(head, false, other));
      assertSame(other, guarded.invoke(head, true, other));
      assertSame(null, guarded.invoke(head, true, null));
      var staticSelect = type.getMethod("staticSelect", Object.class, boolean.class, Object.class);
      assertSame(head, staticSelect.invoke(null, head, false, other));
      assertSame(other, staticSelect.invoke(null, head, true, other));
      assertEquals(30, type.getMethod("narrowWalk").invoke(head));
      var walk = type.getMethod("walk");
      assertEquals(31, walk.invoke(head));
      var value = type.getDeclaredField("value");
      assertTrue(Modifier.isPrivate(value.getModifiers()));
      value.setAccessible(true);
      assertEquals(11, value.get(head));
      assertEquals(21, value.get(middle));
      assertEquals(32, walk.invoke(tail));
      var conditionalConstructor = type.getConstructor(boolean.class, type);
      Object unchanged = conditionalConstructor.newInstance(false, tail);
      assertEquals(7, value.get(unchanged));
      assertEquals(32, value.get(tail));
      Object changed = conditionalConstructor.newInstance(true, tail);
      assertEquals(0, value.get(changed));
      assertEquals(7, value.get(tail));
    }
  }
}
