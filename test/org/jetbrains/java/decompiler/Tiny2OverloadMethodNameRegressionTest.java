package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
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

    fixture.getDecompiler().addSource(outRoot().toFile());
    fixture.getDecompiler().decompileContext();

    Path decompiledFile = fixture.getTargetDir().resolve("GameEngine.java");
    assertTrue(Files.isRegularFile(decompiledFile), "Decompiled file not found: " + decompiledFile);

    String content = DecompilerTestFixture.getContent(decompiledFile);
    assertFalse(content.contains("showMenu_1("), content);

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
