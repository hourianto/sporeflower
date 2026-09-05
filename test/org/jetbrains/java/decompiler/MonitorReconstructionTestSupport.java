package org.jetbrains.java.decompiler;

import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Path;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

final class MonitorReconstructionTestSupport {
  private MonitorReconstructionTestSupport() {
  }

  static void assertRunBehavior(Path classes, String name, int iterations) throws Exception {
    assertRunBehavior(classes, name, "run", iterations);
  }

  static void assertRunBehavior(Path classes, String name, String methodName, int iterations) throws Exception {
    // Loading and executing the input is intentional: successful assembly and
    // decompilation alone do not establish that a hand-written fixture is valid.
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = Class.forName("pkg." + name, true, loader);
      Object instance = type.getConstructor().newInstance();
      Method run = type.getMethod(methodName);
      Field lockField = type.getDeclaredField("lock");
      lockField.setAccessible(true);
      Object lock = lockField.get(null);
      Field action = type.getField("action");
      Field stop = iterations > 1 ? type.getField("stop") : null;
      AtomicInteger calls = new AtomicInteger();
      Runnable work = () -> {
        assertTrue(Thread.holdsLock(lock), "Protected work escaped the monitor");
        if (calls.incrementAndGet() == iterations && stop != null) {
          set(stop, instance, true);
        }
      };

      action.set(null, work);
      run.invoke(instance);
      assertEquals(iterations, calls.get());
      assertFalse(Thread.holdsLock(lock), "Normal completion leaked the monitor");

      // A nested acquisition must release exactly its own hold, not the caller's.
      calls.set(0);
      if (stop != null) stop.setBoolean(instance, false);
      synchronized (lock) {
        run.invoke(instance);
        assertTrue(Thread.holdsLock(lock), "The caller's monitor hold was released");
      }
      assertEquals(iterations, calls.get());
      assertFalse(Thread.holdsLock(lock), "Reentrant completion leaked a monitor hold");

      for (Throwable failure : new Throwable[]{new IllegalStateException("probe"), new AssertionError("probe")}) {
        if (stop != null) stop.setBoolean(instance, false);
        action.set(null, (Runnable)() -> {
          assertTrue(Thread.holdsLock(lock), "Exceptional work escaped the monitor");
          if (failure instanceof RuntimeException exception) throw exception;
          throw (Error)failure;
        });
        InvocationTargetException thrown = assertThrows(InvocationTargetException.class, () -> run.invoke(instance));
        assertSame(failure, thrown.getCause(), "Cleanup replaced the original exception");
        assertFalse(Thread.holdsLock(lock), "Exceptional completion leaked the monitor");
      }

      // Release must use the captured lock even if the source field changes.
      Object replacement = new Object();
      if (stop != null) stop.setBoolean(instance, false);
      action.set(null, (Runnable)() -> {
        assertTrue(Thread.holdsLock(lock));
        set(lockField, null, replacement);
        if (stop != null) set(stop, instance, true);
      });
      run.invoke(instance);
      assertFalse(Thread.holdsLock(lock));
      assertFalse(Thread.holdsLock(replacement));

      lockField.set(null, null);
      if (stop != null) stop.setBoolean(instance, false);
      action.set(null, (Runnable)() -> fail("Work ran after a failed monitorenter"));
      InvocationTargetException thrown = assertThrows(InvocationTargetException.class, () -> run.invoke(instance));
      assertInstanceOf(NullPointerException.class, thrown.getCause());
    }
  }

  private static void set(Field field, Object receiver, Object value) {
    try {
      field.set(receiver, value);
    } catch (IllegalAccessException exception) {
      throw new AssertionError(exception);
    }
  }
}
