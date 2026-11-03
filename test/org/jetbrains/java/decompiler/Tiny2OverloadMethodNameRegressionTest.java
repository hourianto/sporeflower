package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2OverloadMethodNameRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
tiny\t2\t0\tofficial\tnamed
c\tC\tGameEngine
\tm\t(LM;)V\ta\tshowMenu
\tm\t(LM;ZZ)V\tb\tshowMenu
c\tM\tMenuItem
""";
  }

  @Test
  public void testOverloadedMethodsKeepMappedBaseNameWithoutSuffix() throws IOException {
    Path sourceMenu = writeSource("M.java", """
public class M {
}
""");

    Path sourceEngine = writeSource("C.java", """
public class C {
  public static void a(M menu) {
    b(menu, false, true);
  }

  public static void b(M menu, boolean resetSelection, boolean linkBackToCurrent) {
  }
}
""");

    compileJava8NoDebug(List.of(sourceMenu, sourceEngine), outRoot());

    String content = decompileDirectory(outRoot(), "GameEngine.java");
    assertNotContains(content, "showMenu_1(");

    assertTrue(
      Pattern.compile("public\\s+static\\s+void\\s+showMenu\\s*\\(\\s*MenuItem\\s+[^,\\)]*\\)").matcher(content).find(),
      content
    );
    assertTrue(
      Pattern.compile("public\\s+static\\s+void\\s+showMenu\\s*\\(\\s*MenuItem\\s+[^,\\)]*\\s*,\\s*boolean\\s+[^,\\)]*\\s*,\\s*boolean\\s+[^\\)]*\\)").matcher(content).find(),
      content
    );
  }
}
