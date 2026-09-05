package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

public class SynchronizedSearchEarlyReturnRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testSynchronizedSearchWithEarlyReturnPreservesBehavior() throws Exception {
    assertSearchBehavior(fixture.getTestDataDir().resolve("classes/jasm"));
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestSynchronizedSearchEarlyReturn.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestSynchronizedSearchEarlyReturn.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertFalse(content.contains("$VF: monitorenter"), content);

    recompile();
    assertSearchBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertSearchBehavior(Path classes) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = Class.forName("pkg.TestSynchronizedSearchEarlyReturn", true, loader);
      Method find = type.getMethod("find");
      Field lockField = type.getDeclaredField("lock");
      lockField.setAccessible(true);
      Object target = new String("target");
      Object[][] inputs = {{}, {new String("target")}, {target}, {null, target, null, target}, {null, null, null}};
      for (Object[] values : inputs) {
        Object instance = type.getConstructor(Object[].class, Object.class).newInstance(values, target);
        int expected = 0;
        for (int i = values.length - 1; i >= 0; i--) {
          if (values[i] == target) {
            expected = i;
            break;
          }
        }
        Object lock = lockField.get(instance);
        type.getField("action").set(null, (Runnable)() -> assertTrue(Thread.holdsLock(lock), "Search escaped the monitor"));
        assertEquals(expected, find.invoke(instance));
        assertFalse(Thread.holdsLock(lock), "Search return leaked the monitor");
        synchronized (lock) {
          assertEquals(expected, find.invoke(instance));
          assertTrue(Thread.holdsLock(lock));
        }
        assertFalse(Thread.holdsLock(lock));
      }

      Object instance = type.getConstructor(Object[].class, Object.class).newInstance(null, target);
      Object lock = lockField.get(instance);
      type.getField("action").set(null, (Runnable)() -> assertTrue(Thread.holdsLock(lock)));
      InvocationTargetException thrown = assertThrows(InvocationTargetException.class, () -> find.invoke(instance));
      assertInstanceOf(NullPointerException.class, thrown.getCause());
      assertFalse(Thread.holdsLock(lockField.get(instance)), "Exceptional search leaked the monitor");
    }
  }
}
