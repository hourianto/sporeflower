package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.main.decompiler.SingleFileSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
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
    mappings = Files.createTempFile("vf-semantic-integration-", ".tiny");
    Files.writeString(mappings, """
      tiny\t2\t0\tofficial\tnamed
      c\ta\tnamed/Router
      \tf\tI\tc\tPRIMARY
      \tm\t(II)I\tb\troute
      c\td\tnamed/Typed
      \tf\tB\ta\tmask
      \tf\t[I\tb\ttable
      \tm\t(I)V\tc\taccept
      \tm\t(I)V\te\tfeed
      \tm\t()Z\tf\tmatches
      \tm\t()I\tg\tread
      c\tp\tnamed/Base
      \tm\t(II)I\ta\tchoose
      c\tq\tnamed/Child
      \tm\t(II)I\ta\tchoose
      """, StandardCharsets.UTF_8);

    semantics = Files.createTempFile("vf-semantic-integration-", ".json");
    Files.writeString(semantics, """
      {
        "version": 3,
        "namespace": "named",
        "domains": [
          {"id": "named/Mode", "kind": "value"},
          {"id": "named/Flags", "kind": "flags"},
          {"id": "named/Index", "kind": "value"},
          {"id": "named/Left", "kind": "value"},
          {"id": "named/Right", "kind": "value"}
        ],
        "values": [
          {"domain": "named/Mode", "value": 1, "owner": "named/Router", "name": "PRIMARY", "desc": "I", "access": 26, "synthetic": false},
          {"domain": "named/Mode", "value": 2, "owner": "named/Mode", "name": "SECONDARY", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "named/Flags", "value": 1, "owner": "named/Flags", "name": "LOW", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "named/Flags", "value": 2, "owner": "named/Flags", "name": "WRITE", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "named/Flags", "value": 128, "owner": "named/Flags", "name": "HIGH", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "named/Index", "value": 1, "owner": "named/Index", "name": "SECOND", "desc": "J", "access": 25, "synthetic": true},
          {"domain": "named/Left", "value": 1, "owner": "named/Left", "name": "MATCH", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "named/Right", "value": 2, "owner": "named/Right", "name": "MATCH", "desc": "I", "access": 25, "synthetic": true}
        ],
        "scalar_bindings": [
          {"target": {"kind": "return", "owner": "named/Router", "name": "route", "desc": "(II)I"}, "domain": "named/Mode"},
          {"target": {"kind": "parameter", "owner": "named/Router", "name": "route", "desc": "(II)I", "index": 0}, "domain": "named/Mode"},
          {"target": {"kind": "field", "owner": "named/Typed", "name": "mask", "desc": "B"}, "domain": "named/Flags"},
          {"target": {"kind": "parameter", "owner": "named/Typed", "name": "accept", "desc": "(I)V", "index": 0}, "domain": "named/Flags"},
          {"target": {"kind": "parameter", "owner": "named/Base", "name": "choose", "desc": "(II)I", "index": 1}, "domain": "named/Right"},
          {"target": {"kind": "parameter", "owner": "named/Child", "name": "choose", "desc": "(II)I", "index": 0}, "domain": "named/Left"}
        ],
        "array_bindings": [
          {"target": {"kind": "field", "owner": "named/Typed", "name": "table", "desc": "[I"}, "index_domains": [{"dimension": 0, "domain": "named/Index"}]}
        ]
      }
      """, StandardCharsets.UTF_8);

    fixture = new DecompilerTestFixture();
    fixture.setUp(
      IFernflowerPreferences.MAPPINGS_PATH, mappings.toString(),
      IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE, "official",
      IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE, "named",
      IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH, semantics.toString()
    );
  }

  @Override
  @AfterEach
  public void tearDown() {
    super.tearDown();
    try {
      Files.deleteIfExists(mappings);
      Files.deleteIfExists(semantics);
    }
    catch (IOException ignored) {
    }
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
      }
      """);

    compileJava8NoDebug(source, outRoot());
    String content = decompileDirectory(outRoot(), "named/Typed.java");

    assertTrue(content.contains("| Flags.WRITE"), content);
    assertTrue(content.contains("== (byte)(") && content.contains("Flags.HIGH") && content.contains("Flags.LOW"), content);
    assertTrue(content.contains("[(int)(Index.SECOND)]"), content);
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
