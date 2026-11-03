package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;

public class AcrossStackEffectsRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void preservesOrderAcrossStatements() throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    decompileClassFile(original.resolve("pkg/TestAcrossStackEffects.class"), "pkg/TestAcrossStackEffects.java");
    recompile();
    try (BehaviorComparison behavior = compareBehavior(original)) {
      behavior.assertSame("pair order and calls", new Object[]{7 + 1 * 10 + 2 * 100, 2}, loader -> {
        Class<?> type = loader.loadClass("pkg.TestAcrossStackEffects");
        Object receiver = type.getConstructor().newInstance();
        type.getField("calls").setInt(null, 0);
        Object result = type.getMethod("pair", int.class, type).invoke(receiver, 7, receiver);
        return new Object[]{result, type.getField("calls").getInt(null)};
      });
      behavior.assertSame("divide before effect", new Object[]{2 * 100 + 9, 1}, loader -> {
        Class<?> type = loader.loadClass("pkg.TestAcrossStackEffects");
        type.getField("calls").setInt(null, 0);
        Object result = type.getMethod("divideBeforeEffect", int.class, int.class).invoke(null, 6, 3);
        return new Object[]{result, type.getField("calls").getInt(null)};
      });
      // The division runs before effect(), so failure must leave calls at zero.
      behavior.assertSame("calls before division by zero", 0, loader -> {
        Class<?> type = loader.loadClass("pkg.TestAcrossStackEffects");
        type.getField("calls").setInt(null, 0);
        var method = type.getMethod("divideBeforeEffect", int.class, int.class);
        InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> method.invoke(null, 6, 0));
        assertInstanceOf(ArithmeticException.class, failure.getCause());
        return type.getField("calls").getInt(null);
      });
      behavior.assertSame("localWrite", 15 * 100 + 16, loader ->
        loader.loadClass("pkg.TestAcrossStackEffects").getMethod("localWrite", int.class).invoke(null, 5));
    }
  }
}
