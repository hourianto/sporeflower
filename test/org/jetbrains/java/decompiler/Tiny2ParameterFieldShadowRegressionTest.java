package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Pattern;

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
  public void testMappedParametersCanShadowQualifiedStaticAndInstanceFields() throws IOException {
    Path staticSource = writeSource("C.java", """
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

    Path instanceSource = writeSource("A.java", """
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

    compileJava8NoDebug(List.of(staticSource, instanceSource), outRoot());

    String staticContent = decompileDirectory(outRoot(), "GameEngine.java");
    assertNotContains(staticContent, "demoModeStatex");
    assertTrue(Pattern.compile("static\\s+void\\s+setDemoMode\\s*\\(\\s*int\\s+demoModeState\\s*\\)").matcher(staticContent).find(), staticContent);
    assertContains(staticContent, "GameEngine.demoModeState = demoModeState;", "if (GameEngine.demoModeState == 1)");

    String content = readDecompiled("Entity.java");
    assertNotContains(content, "entityTypex");
    assertTrue(Pattern.compile("void\\s+setEntityType\\s*\\(\\s*int\\s+entityType\\s*\\)").matcher(content).find(), content);
    assertContains(content, "this.entityType = entityType;");
  }
}
