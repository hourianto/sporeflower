package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2ParameterFieldShadowRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
tiny\t2\t0\tofficial\tnamed
c\tC\tGameEngine
\tf\tI\tad\tdemoModeState
\tm\t(I)V\tl\tsetDemoMode
\t\tp\t0\tp0\tdemoModeState
c\tA\tEntity
\tf\tI\td\tentityType
\tm\t(I)V\ta\tsetEntityType
\t\tp\t1\tp0\tentityType
""";
  }

  @Test
  public void testMappedParameterCanShadowStaticFieldWhenFieldIsQualified() throws IOException {
    Path source = writeSource("C.java", """
public class C {
  static int ad;

  static void l(int p0) {
    ad = p0;
    if (ad == 1) {
      ad = 2;
    }
  }
}
""");

    compileJava8NoDebug(source, outRoot());

    String content = decompileDirectory(outRoot(), "GameEngine.java");
    assertFalse(content.contains("demoModeStatex"), content);
    assertTrue(Pattern.compile("static\\s+void\\s+setDemoMode\\s*\\(\\s*int\\s+demoModeState\\s*\\)").matcher(content).find(), content);
    assertTrue(content.contains("GameEngine.demoModeState = demoModeState;"), content);
    assertTrue(content.contains("if (GameEngine.demoModeState == 1)"), content);
  }

  @Test
  public void testMappedParameterCanShadowInstanceFieldWhenFieldIsQualified() throws IOException {
    Path source = writeSource("A.java", """
public class A {
  int d;

  void a(int p0) {
    this.d = p0;
    if (this.d == 1) {
      this.d = 2;
    }
  }
}
""");

    compileJava8NoDebug(source, outRoot());

    String content = decompileDirectory(outRoot(), "Entity.java");
    assertFalse(content.contains("entityTypex"), content);
    assertTrue(Pattern.compile("void\\s+setEntityType\\s*\\(\\s*int\\s+entityType\\s*\\)").matcher(content).find(), content);
    assertTrue(content.contains("this.entityType = entityType;"), content);
  }
}
