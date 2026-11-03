package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class ConstructorSourceRecoveryTest extends DecompileRegressionTestBase {
  @Test
  public void testSelfAliasBeforeSuperDoesNotRenderBeforeConstructorCall() throws Exception {
    String content = decompileFixture("TestConstructorSelfAliasBeforeSuper");
    int constructorCall = content.indexOf("super();");
    int receiverCopy = content.indexOf("= this;");
    assertTrue(receiverCopy >= 0 && (constructorCall < 0 || receiverCopy > constructorCall), content);

    recompile();

    Path original = fixture.getTestDataDir().resolve("classes/jasm");
    try (BehaviorComparison behavior = compareBehavior(original)) {
      behavior.assertSame("constructor value", "ready", loader -> {
        Class<?> type = loader.loadClass("pkg.TestConstructorSelfAliasBeforeSuper");
        Object instance = type.getConstructor(Object.class).newInstance(new Object());
        return type.getMethod("value").invoke(instance);
      });
    }
  }
}
