package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;

import java.io.IOException;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.assertTrue;

abstract class SemanticTestBase extends DecompileRegressionTestBase {
  private final String mappings;
  private final String subject;

  protected SemanticTestBase(String mappings, String subject) {
    this.mappings = mappings;
    this.subject = subject;
  }

  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH,
      fixture.getTestDataDir().resolve("semantic/" + mappings + ".json").toString()};
  }

  protected String subject(String body) throws IOException {
    return compileDecompileAndRead("sample/" + subject + ".java",
      "package sample; public class " + subject + " {\n" + body + "\n}");
  }

  protected void assertSameIntResults(String[] methods, int... values) throws Exception {
    Object[][] arguments = Arrays.stream(values).mapToObj(value -> new Object[]{value}).toArray(Object[][]::new);
    assertSameResults(methods, new Class<?>[]{int.class}, arguments);
  }

  protected void assertSameIntResults(String method, int... values) throws Exception {
    assertSameIntResults(new String[]{method}, values);
  }

  protected void assertSameResults(String... methods) throws Exception {
    assertSameResults(methods, new Class<?>[0], new Object[][]{{}});
  }

  protected void assertSameResults(String[] methods, Class<?>[] parameterTypes, Object[]... arguments) throws Exception {
    assertTrue(methods.length > 0 && arguments.length > 0, "Behavior comparison must invoke at least one case");
    try (BehaviorComparison behavior = compareBehavior()) {
      for (String name : methods) {
        for (Object[] input : arguments) {
          behavior.assertSameStaticCall("sample." + subject, name, parameterTypes, input);
        }
      }
    }
  }

  protected static void assertMethodsContain(String source, String[] methods, String... snippets) {
    for (String method : methods) assertContains(methodSource(source, method), snippets);
  }

  protected static void assertMethodsDoNotContain(String source, String[] methods, String... snippets) {
    for (String method : methods) assertNotContains(methodSource(source, method), snippets);
  }

  protected static String methodSource(String source, String name) {
    // These fixtures use the default three-space indentation and unambiguous
    // method names. Anchor at a declaration, never a preceding invocation.
    var declaration = java.util.regex.Pattern.compile(
      "(?m)^   \\S[^\\n]*\\b" + java.util.regex.Pattern.quote(name) + "\\([^\\n]*\\)\\s*\\{")
      .matcher(source);
    assertTrue(declaration.find(), "Missing method " + name + ":\n" + source);
    int end = source.indexOf("\n   }", declaration.end());
    assertTrue(end >= 0, "Missing method end for " + name + ":\n" + source);
    return source.substring(declaration.start(), end);
  }
}
