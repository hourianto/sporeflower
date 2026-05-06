package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2OverrideFamilyRenameRegressionTest extends DecompileRegressionTestBase {
  private Path mapping;

  @Override
  @BeforeEach
  public void setUp() throws IOException {
    mapping = Files.createTempFile("vf-tiny2-override-family-", ".tiny");
    Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\td\tSceneNode
c\tp\tGameCanvas
\tm\t(Ld;)V\ta\taddSceneNode
c\tah\tRenderable
\tm\t()[I\tb\tgetBounds
c\tw\tGridItemProvider
\tm\t(Ld;)V\ta\tattachNode
\tm\t()[I\tb\tgetViewportBounds
c\tac\tSnakeArcadeGame
c\tv\tSnakeArcadeMenu
\tm\t()V\tl\tdisposeResources
c\trr\tRenameRoot
c\trb\tRenameBase
\tm\t()Lrx;\tb\ta
c\trc\tRenameChild
\tm\t()Lrx;\tb\ta
c\tro\tRenameOther
c\trx\tRenameResult
""", StandardCharsets.UTF_8);

    fixture = new DecompilerTestFixture();
    fixture.setUp(
      IFernflowerPreferences.MAPPINGS_PATH, mapping.toString(),
      IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE, "official",
      IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE, "named"
    );
  }

  @Override
  @AfterEach
  public void tearDown() {
    super.tearDown();
    try {
      if (mapping != null) {
        Files.deleteIfExists(mapping);
      }
    }
    catch (IOException ignored) {
    }
  }

  @Test
  public void testConcreteOnlyTinyRenamePropagatesToAbstractOverrideFamily() throws IOException {
    Path base = writeSource("ac.java", """
abstract class ac {
  public void e() {
    l();
  }

  public abstract void l();
}
""");

    Path child = writeSource("v.java", """
class v extends ac {
  public void l() {
  }
}
""");

    compileJava8NoDebug(List.of(base, child), outRoot());

    String childContent = decompileDirectory(outRoot(), "SnakeArcadeMenu.java");
    String baseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("SnakeArcadeGame.java"));

    assertTrue(baseContent.contains("public abstract void disposeResources();"), baseContent);
    assertTrue(childContent.contains("public void disposeResources()"), childContent);
    assertTrue(baseContent.contains("this.disposeResources();"), baseContent);
    assertFalse(baseContent.contains("public abstract void l();"), baseContent);

    recompile();
  }

  @Test
  public void testInheritedRealizedNameWinsOverStaleOverrideHintAfterReturnOnlyCollision() throws IOException {
    Path other = writeSource("ro.java", """
class ro {
}
""");

    Path result = writeSource("rx.java", """
class rx {
}
""");

    Path root = writeSource("rr.java", """
abstract class rr {
  protected final ro a() {
    return new ro();
  }
}
""");

    Path base = writeSource("rb.java", """
abstract class rb extends rr {
  protected abstract rx b();
}
""");

    Path child = writeSource("rc.java", """
final class rc extends rb {
  protected final rx b() {
    return new rx();
  }
}
""");

    compileJava8NoDebug(List.of(other, result, root, base, child), outRoot());

    String childContent = decompileDirectory(outRoot(), "RenameChild.java");
    String baseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("RenameBase.java"));

    assertTrue(baseContent.contains("protected abstract RenameResult method_"), baseContent);
    assertTrue(childContent.contains("protected final RenameResult method_"), childContent);

    recompile();
  }

  @Test
  public void testInheritedSuperclassImplementationKeepsInterfaceNameCompilable() throws IOException {
    Path node = writeSource("d.java", """
class d {
}
""");

    Path canvas = writeSource("p.java", """
class p {
  public void a(d node) {
  }

  public int[] b() {
    return new int[0];
  }
}
""");

    Path provider = writeSource("w.java", """
interface w {
  void a(d node);

  int[] b();
}
""");

    Path renderable = writeSource("ah.java", """
interface ah {
  int[] b();
}
""");

    Path game = writeSource("ac.java", """
abstract class ac extends p implements w {
}
""");

    Path menu = writeSource("v.java", """
class v extends ac implements ah {
}
""");

    compileJava8NoDebug(List.of(node, canvas, provider, renderable, game, menu), outRoot());

    decompileDirectory(outRoot(), "SnakeArcadeMenu.java");
    StringBuilder allSources = new StringBuilder();
    for (Path source : listJavaSources(fixture.getTargetDir())) {
      allSources.append(DecompilerTestFixture.getContent(source)).append('\n');
    }
    String content = allSources.toString();

    assertTrue(content.contains("public void addSceneNode(SceneNode"), content);
    assertTrue(content.contains("void addSceneNode(SceneNode"), content);
    assertFalse(content.contains("attachNode"), content);
    assertTrue(content.contains("int[] getBounds()") ^ content.contains("int[] getViewportBounds()"), content);

    recompile();
  }
}
