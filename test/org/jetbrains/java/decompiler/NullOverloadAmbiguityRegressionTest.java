package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class NullOverloadAmbiguityRegressionTest extends DecompileRegressionTestBase {
  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.RENAME_ENTITIES, "1"};
  }

  @Test
  public void testNullLiteralCallKeepsExplicitCastForOverloadSelection() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/TestNullOverloadAmbiguity.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "pkg/TestNullOverloadAmbiguity.java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(Pattern.compile("\\(.*\\)null").matcher(content).find(), content);

    List<Path> decompiledSources = listJavaSources(fixture.getTargetDir());
    assertFalse(decompiledSources.isEmpty(), "No decompiled sources found");
    compileJava8(decompiledSources, fixture.getTempDir().resolve("compile-out"));
  }

  @Test
  public void testNullLiteralCallUsesRenderedReceiverTypeForOverloadSelection() throws IOException {
    Path jasmClasses = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path baseClass = jasmClasses.resolve("TestRenderedReceiverNullBase.class");
    Path childClass = jasmClasses.resolve("TestRenderedReceiverNullChild.class");
    Path callerClass = jasmClasses.resolve("TestRenderedReceiverNullCaller.class");
    assertTrue(Files.isRegularFile(baseClass), "Missing test class: " + baseClass);
    assertTrue(Files.isRegularFile(childClass), "Missing test class: " + childClass);
    assertTrue(Files.isRegularFile(callerClass), "Missing test class: " + callerClass);

    Path input = fixture.getTempDir().resolve("rendered-receiver-null-input/pkg");
    Files.createDirectories(input);
    Files.copy(baseClass, input.resolve(baseClass.getFileName()));
    Files.copy(childClass, input.resolve(childClass.getFileName()));
    Files.copy(callerClass, input.resolve(callerClass.getFileName()));

    String content = decompileDirectory(input.getParent(), "pkg/TestRenderedReceiverNullCaller.java");
    assertTrue(content.contains("(int[])null"), content);

    recompile();
  }

  @Test
  public void testObjectArgumentCallUsesRenderedReceiverTypeForOverloadSelection() throws IOException {
    Path listener = writeSource("pkg/RenderedReceiverListener.java", """
package pkg;

public interface RenderedReceiverListener {
}
""");

    Path node = writeSource("pkg/RenderedReceiverNode.java", """
package pkg;

public class RenderedReceiverNode {
  public void a(RenderedReceiverListener listener) {
  }
}
""");

    Path group = writeSource("pkg/RenderedReceiverGroup.java", """
package pkg;

public class RenderedReceiverGroup extends RenderedReceiverNode {
  public final void a(RenderedReceiverNode node) {
  }
}
""");

    Path widget = writeSource("pkg/RenderedReceiverWidget.java", """
package pkg;

public class RenderedReceiverWidget extends RenderedReceiverNode implements RenderedReceiverListener {
}
""");

    Path caller = writeSource("pkg/TestRenderedReceiverObjectArg.java", """
package pkg;

public class TestRenderedReceiverObjectArg {
  public static void call(RenderedReceiverGroup group, RenderedReceiverWidget widget) {
    group.a((RenderedReceiverListener)(Object)widget);
  }
}
""");

    compileJava8NoDebug(List.of(listener, node, group, widget, caller), outRoot());
    removeSingleCheckcast(outRoot().resolve("pkg/TestRenderedReceiverObjectArg.class"));

    String content = decompileDirectory(outRoot(), "pkg/TestRenderedReceiverObjectArg.java");
    assertTrue(content.contains("(RenderedReceiverListener)var1"), content);

    recompile();
  }

  private static void removeSingleCheckcast(Path classFile) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);

    for (int i = 0; i <= bytes.length - 9; i++) {
      if (Byte.toUnsignedInt(bytes[i]) == 0x2A
        && Byte.toUnsignedInt(bytes[i + 1]) == 0x2B
        && Byte.toUnsignedInt(bytes[i + 2]) == 0xC0
        && Byte.toUnsignedInt(bytes[i + 5]) == 0xB6
        && Byte.toUnsignedInt(bytes[i + 8]) == 0xB1) {
        bytes[i + 2] = 0;
        bytes[i + 3] = 0;
        bytes[i + 4] = 0;
        Files.write(classFile, bytes);
        return;
      }
    }

    assertTrue(false, "Could not locate checkcast/invokevirtual sequence in " + classFile);
  }
}
