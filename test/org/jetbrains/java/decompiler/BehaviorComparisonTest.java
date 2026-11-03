package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.io.IOException;

import static org.junit.jupiter.api.Assertions.assertThrows;

class BehaviorComparisonTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(strings = {"cast", "run"})
  void exceptionDiagnosticsDoNotDefineBehavior(String method) throws Exception {
    compilePrograms("Object value = null; return value.toString();", "return ((Object)null).toString();");
    try (BehaviorComparison behavior = compareBehavior()) {
      behavior.assertSameStaticCall("Subject", method, new Class<?>[0]);
      // Cast diagnostics identify each loader; helpful NPE diagnostics describe
      // the expression, which can legitimately change during decompilation.
      assertThrows(AssertionError.class, () -> behavior.assertSameIncludingExceptionMessage(method,
        loader -> loader.loadClass("Subject").getMethod(method).invoke(null)));
    }
  }

  @ParameterizedTest
  @ValueSource(strings = {"return 2;", "throw new IllegalStateException(\"failure\");"})
  void differentResultsAndSuccessVersusFailureAreRejected(String rebuiltBody) throws Exception {
    compilePrograms("return 1;", rebuiltBody);
    try (BehaviorComparison behavior = compareBehavior()) {
      assertThrows(AssertionError.class, () -> behavior.assertSameStaticCall("Subject", "run", new Class<?>[0]));
    }
  }

  @Test
  void differentExceptionTypesAreRejectedEvenWithTheSameMessage() throws Exception {
    compilePrograms("throw new IllegalArgumentException(\"failure\");", "throw new IllegalStateException(\"failure\");");
    try (BehaviorComparison behavior = compareBehavior()) {
      assertThrows(AssertionError.class, () -> behavior.assertSameStaticCall("Subject", "run", new Class<?>[0]));
    }
  }

  @Test
  void stableMessagesCanBeCheckedAndReflectionErrorsCannotMatch() throws Exception {
    compilePrograms("throw new IllegalStateException(\"failure\");", "throw new IllegalStateException(\"failure\");");
    try (BehaviorComparison behavior = compareBehavior()) {
      behavior.assertSameIncludingExceptionMessage("application message",
        loader -> loader.loadClass("Subject").getMethod("run").invoke(null));
      assertThrows(NoSuchMethodException.class, () -> behavior.assertSameStaticCall("Subject", "missing", new Class<?>[0]));
    }
  }

  private void compilePrograms(String originalBody, String rebuiltBody) throws IOException {
    compileJava8NoDebug(writeSource("Subject.java", program(originalBody)), outRoot());
    compileJava8NoDebug(writeSource("Subject.java", program(rebuiltBody)), recompiledRoot());
  }

  private static String program(String body) {
    return """
      public class Subject {
        public static Object cast() { return (String)(Object)new Subject(); }
        public static Object run() { %s }
      }
      """.formatted(body);
  }
}
