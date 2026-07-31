package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class SemanticConstantsRegressionTest extends DecompileRegressionTestBase {
  private Path semanticMappings;

  @Override
  @BeforeEach
  public void setUp() throws IOException {
    semanticMappings = Files.createTempFile("vf-semantic-", ".json");
    Files.writeString(semanticMappings, """
      {
        "version": 3,
        "namespace": "named",
        "domains": [
          {"id": "sample/State", "kind": "value"},
          {"id": "sample/Mask", "kind": "flags"},
          {"id": "sample/ExternalAnchor", "kind": "flags"},
          {"id": "sample/Slots", "kind": "slots"}
        ],
        "values": [
          {"domain": "sample/State", "value": 1, "owner": "sample/Subject", "name": "PRIMARY", "desc": "I", "access": 25, "synthetic": false},
          {"domain": "sample/State", "value": 2, "owner": "sample/State", "name": "SECONDARY", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "sample/Mask", "value": 1, "owner": "sample/Mask", "name": "READ", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "sample/Mask", "value": 2, "owner": "sample/Mask", "name": "WRITE", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "sample/ExternalAnchor", "value": 4, "owner": "sample/ExternalApi", "name": "LEFT", "desc": "I", "access": 25, "synthetic": false},
          {"domain": "sample/ExternalAnchor", "value": 16, "owner": "sample/ExternalApi", "name": "TOP", "desc": "I", "access": 25, "synthetic": false},
          {"domain": "sample/Slots", "value": 0, "owner": "sample/Slots", "name": "STATE", "desc": "I", "access": 25, "synthetic": true, "element_domain": "sample/State"}
        ],
        "scalar_bindings": [
          {"target": {"kind": "field", "owner": "sample/Subject", "name": "state", "desc": "I"}, "domain": "sample/State"},
          {"target": {"kind": "field", "owner": "sample/Subject", "name": "mask", "desc": "I"}, "domain": "sample/Mask"},
          {"target": {"kind": "return", "owner": "sample/Subject", "name": "stateResult", "desc": "()I"}, "domain": "sample/State"},
          {"target": {"kind": "parameter", "owner": "sample/Subject", "name": "setState", "desc": "(I)V", "index": 0}, "domain": "sample/State"},
          {"target": {"kind": "parameter", "owner": "sample/Subject", "name": "setMask", "desc": "(I)V", "index": 0}, "domain": "sample/Mask"},
          {"target": {"kind": "parameter", "owner": "sample/ExternalApi", "name": "consume", "desc": "(I)V", "index": 0}, "domain": "sample/ExternalAnchor"},
          {"target": {"kind": "parameter", "owner": "sample/Subject", "name": "select", "desc": "(I)I", "index": 0}, "domain": "sample/State"}
        ],
        "array_bindings": [
          {"target": {"kind": "field", "owner": "sample/Subject", "name": "properties", "desc": "[I"}, "slot_domains": [{"dimension": 0, "domain": "sample/Slots"}]},
          {"target": {"kind": "field", "owner": "sample/Subject", "name": "records", "desc": "[[I"}, "slot_domains": [{"dimension": 1, "domain": "sample/Slots"}]},
          {"target": {"kind": "field", "owner": "sample/Subject", "name": "stateGrid", "desc": "[[I"}, "element_domain": "sample/State"},
          {"target": {"kind": "field", "owner": "sample/Subject", "name": "table", "desc": "[[I"}, "index_domains": [{"dimension": 1, "domain": "sample/State"}]},
          {"target": {"kind": "return", "owner": "sample/Subject", "name": "stateRow", "desc": "()[I"}, "index_domains": [{"dimension": 0, "domain": "sample/State"}]},
          {"target": {"kind": "return", "owner": "sample/Subject", "name": "recordRow", "desc": "()[I"}, "slot_domains": [{"dimension": 0, "domain": "sample/Slots"}]},
          {"target": {"kind": "return", "owner": "sample/Subject", "name": "stateValues", "desc": "()[I"}, "element_domain": "sample/State"},
          {"target": {"kind": "parameter", "owner": "sample/Subject", "name": "readState", "desc": "([I)I", "index": 0}, "index_domains": [{"dimension": 0, "domain": "sample/State"}]},
          {"target": {"kind": "parameter", "owner": "sample/Subject", "name": "readRecord", "desc": "([I)Z", "index": 0}, "slot_domains": [{"dimension": 0, "domain": "sample/Slots"}]},
          {"target": {"kind": "parameter", "owner": "sample/Subject", "name": "readValues", "desc": "([I)Z", "index": 0}, "element_domain": "sample/State"}
        ]
      }
      """, StandardCharsets.UTF_8);

    fixture = new DecompilerTestFixture();
    fixture.setUp(IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH, semanticMappings.toString());
  }

  @Override
  @AfterEach
  public void tearDown() {
    super.tearDown();
    try {
      Files.deleteIfExists(semanticMappings);
    }
    catch (IOException ignored) {
    }
  }

  @Test
  public void testContextualConstantsAndGeneratedHoldersProduceCompilableSource() throws IOException {
    Path source = writeSource("sample/Subject.java", """
      package sample;

      public class Subject {
        public static final int PRIMARY = 1;
        private int state;
        private int mask;
        private int[] properties = new int[1];
        private int[][] records = new int[][]{{2}};
        private int[][] stateGrid = new int[][]{{1, 2}};
        private int[][] table = new int[][]{{10, 20, 30}};

        public void setState(int value) { state = value; }
        public void setMask(int value) { mask = value; }
        public void setPartialMask() { setMask(17); }
        public void setComplementedMask() { setMask(-4); }
        public int stateResult() { return 2; }
        public boolean isPrimary() { return state == 1; }
        public boolean isSecondaryProperty() { return properties[0] == 2; }
        public boolean canRead() { return (mask & 1) != 0; }
        public int withWrite() { return mask | 2; }
        public void configure(boolean alternate) {
          setState(alternate ? 1 : 2);
          setMask(alternate ? 1 : 3);
        }
        public int aliasedIndex() { int[] row = table[0]; return row[1]; }
        public int[] stateRow() { return table[0]; }
        public int returnedIndex() { return stateRow()[2]; }
        public int readState(int[] values) { return values[2]; }
        public boolean isRecordSecondary(int row) { return records[row][0] == 2; }
        public int[] recordRow() { return records[0]; }
        public boolean returnedRecordSecondary() { return recordRow()[0] == 2; }
        public boolean readRecord(int[] record) { return record[0] == 2; }
        public boolean aliasedElementSecondary() { int[] row = stateGrid[0]; return row[1] == 2; }
        public int[] stateValues() { return stateGrid[0]; }
        public boolean returnedElementPrimary() { return stateValues()[0] == 1; }
        public boolean readValues(int[] values) { return values[0] == 2; }
        public int select(int value) {
          switch (value) {
            case 1: return 10;
            case 2: return 20;
            default: return 0;
          }
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());
    String content = decompileDirectory(outRoot(), "sample/Subject.java");

    assertTrue(content.contains("State.SECONDARY"), content);
    assertTrue(content.contains("Mask.READ | Mask.WRITE"), content);
    assertTrue(content.contains("Mask.READ | 16"), content);
    assertTrue(content.contains("~(Mask.READ | Mask.WRITE)"), content);
    assertTrue(content.contains("mask & Mask.READ"), content);
    assertTrue(content.contains("mask | Mask.WRITE"), content);
    assertTrue(content.contains("properties[Slots.STATE]"), content);
    assertTrue(content.contains("PRIMARY"), content);
    assertTrue(content.contains("[PRIMARY]"), content);
    assertTrue(content.contains("stateRow()[State.SECONDARY]"), content);
    assertTrue(content.contains("values[State.SECONDARY]") || content.contains("var1[State.SECONDARY]"), content);
    assertTrue(content.contains("records[var1][Slots.STATE] == State.SECONDARY"), content);
    assertTrue(content.contains("recordRow()[Slots.STATE] == State.SECONDARY"), content);
    assertTrue(content.contains("record[Slots.STATE] == State.SECONDARY") || content.contains("var1[Slots.STATE] == State.SECONDARY"), content);
    assertTrue(content.contains("row[1] == State.SECONDARY") || content.contains("var1[1] == State.SECONDARY"), content);
    assertTrue(content.contains("stateValues()[0] == PRIMARY"), content);
    assertTrue(content.contains("var1[0] == State.SECONDARY"), content);
    assertTrue(content.contains("new int[][]{{PRIMARY, State.SECONDARY}}"), content);
    assertTrue(content.contains("new int[][]{{State.SECONDARY}}"), content);
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve("sample/State.java")));
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve("sample/Mask.java")));
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve("sample/Slots.java")));
    recompile();
  }

  @Test
  public void testSemanticBindingsOnExternalLibraryMethods() throws IOException {
    Path api = writeSource("sample/ExternalApi.java", """
      package sample;

      public class ExternalApi {
        public static final int LEFT = 4;
        public static final int TOP = 16;
        public static void consume(int anchor) {}
      }
      """);
    Path subject = writeSource("sample/ExternalSubject.java", """
      package sample;

      public class ExternalSubject {
        public void call() { ExternalApi.consume(20); }
      }
      """);

    compileJava8NoDebug(java.util.List.of(api, subject), outRoot());
    Path libraryRoot = fixture.getTempDir().resolve("external-library");
    Files.createDirectories(libraryRoot.resolve("sample"));
    Files.move(outRoot().resolve("sample/ExternalApi.class"), libraryRoot.resolve("sample/ExternalApi.class"));
    fixture.getDecompiler().addLibrary(libraryRoot.toFile());

    String content = decompileDirectory(outRoot(), "sample/ExternalSubject.java");
    assertTrue(content.contains("ExternalApi.LEFT | ExternalApi.TOP"), content);
    recompile(java.util.List.of(api));
  }
}
