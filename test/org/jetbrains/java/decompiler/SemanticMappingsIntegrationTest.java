package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.main.decompiler.SingleFileSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;
import java.util.stream.Collectors;
import java.util.zip.ZipFile;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class SemanticMappingsIntegrationTest extends DecompileRegressionTestBase {
  private Path mappings;
  private Path semantics;

  @Override
  @BeforeEach
  public void setUp() throws IOException {
    fixture = new DecompilerTestFixture();
    mappings = fixture.getTestDataDir().resolve("semantic/integration.tiny");
    semantics = fixture.getTestDataDir().resolve("semantic/integration.json");
    fixture.setUp(
      IFernflowerPreferences.MAPPINGS_PATH, mappings.toString(),
      IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE, "official",
      IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE, "named",
      IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH, semantics.toString()
    );
  }

  @Test
  public void testRenamedCurrentMethodsAndPerParameterInheritanceUseNamedSemantics() throws IOException {
    Path router = writeSource("a.java", """
      class a {
        private static final int c = 1;

        int b(int value, int ignored) {
          switch (value) {
            case 1: return 2;
            case 2: return c;
            default: return 0;
          }
        }
      }
      """);
    Path base = writeSource("p.java", """
      class p {
        int a(int left, int right) { return 0; }
      }
      """);
    Path child = writeSource("q.java", """
      class q extends p {
        int a(int left, int right) {
          return left == 1 && right == 2 ? 1 : 0;
        }
      }
      """);

    compileJava8NoDebug(List.of(router, base, child), outRoot());
    decompileDirectory(outRoot(), "named/Router.java");

    String routerContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("named/Router.java"));
    String childContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("named/Child.java"));
    assertTrue(routerContent.contains("case PRIMARY:"), routerContent);
    assertTrue(routerContent.contains("case Mode.SECONDARY:"), routerContent);
    assertTrue(routerContent.contains("return Mode.SECONDARY;"), routerContent);
    assertTrue(childContent.contains("Left.MATCH") && childContent.contains("Right.MATCH"), childContent);
    recompile();
  }

  @Test
  public void testSymbolicRenderingPreservesWidthsAndFlowsThroughBitwiseConsumers() throws IOException {
    Path source = writeSource("d.java", """
      class d {
        private byte a = -127;
        private int[] b = new int[]{3, 7};

        void c(int flags) {}
        void e(int dynamic) { c(dynamic | 2); }
        boolean f() { return a == -127; }
        int g() { return b[1]; }
        boolean h() { return a == 1; }
        void i() { a = 1; }
        int j() {
          switch (a) {
            case 1: return 1;
            default: return 0;
          }
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());
    String content = decompileDirectory(outRoot(), "named/Typed.java");

    assertTrue(content.contains("| Flags.WRITE"), content);
    assertTrue(content.contains("== (byte)(") && content.contains("Flags.HIGH") && content.contains("Flags.LOW"), content);
    assertTrue(content.contains("[(int)(Index.SECOND)]"), content);
    assertTrue(content.contains("mask == Flags.LOW"), content);
    assertTrue(content.contains("mask = Flags.LOW;"), content);
    assertTrue(content.contains("case Flags.LOW:"), content);
    assertTrue(!content.contains("(byte)(Flags.LOW)"), content);
    String flags = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("named/Flags.java"));
    assertTrue(flags.indexOf("LOW = 1") < flags.indexOf("WRITE = 2"), flags);
    assertTrue(flags.indexOf("WRITE = 2") < flags.indexOf("HIGH = 128"), flags);
    recompile();
  }

  @Test
  public void testExplicitReturnDomainSourcesRemainCallSitePolymorphic() throws IOException {
    Path numeric = writeSource("x.java", """
      class x {
        static int a(int value) { return value < 0 ? -value : value; }
        static int b(int value) { return Integer.compare(value, 0); }
      }
      """);
    Path subject = writeSource("d.java", """
      class d {
        int k(int item) {
          switch (x.a(item)) {
            case 2: return 20;
            default: return x.b(item) == 0 ? 0 : -1;
          }
        }

        boolean l(int raw) {
          return x.a(raw) == 1;
        }
      }
      """);

    compileJava8NoDebug(List.of(numeric, subject), outRoot());
    String content = decompileDirectory(outRoot(), "named/Typed.java");

    assertTrue(content.contains("case Mode.SECONDARY:"), content);
    assertTrue(content.contains("Numeric.sign(") && content.contains(" == 0"), content);
    assertTrue(content.contains("Numeric.absolute(") && content.contains(" == 1"), content);
    recompile();
  }

  @Test
  public void testGeneratedSemanticHoldersAreWrittenInsideArchiveOutputs() throws IOException {
    Path source = writeSource("named/ArchiveSubject.java", """
      package named;

      class ArchiveSubject {
        int value() { return 7; }
      }
      """);
    compileJava8NoDebug(source, outRoot());

    Path archive = fixture.getTempDir().resolve("decompiled.zip");
    try (SingleFileSaver saver = new SingleFileSaver(archive.toFile())) {
      Decompiler.builder()
        .inputs(outRoot().toFile())
        .output(saver)
        .option(IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH, semantics.toString())
        .build()
        .decompile();
    }

    try (ZipFile zip = new ZipFile(archive.toFile())) {
      Set<String> entries = zip.stream().map(entry -> entry.getName()).collect(Collectors.toSet());
      assertTrue(entries.contains("named/ArchiveSubject.java"), entries.toString());
      assertTrue(entries.contains("named/Mode.java") && entries.contains("named/Flags.java"), entries.toString());
    }
  }
}
