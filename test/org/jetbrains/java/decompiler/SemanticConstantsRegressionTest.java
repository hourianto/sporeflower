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
        "version": 1,
        "namespace": "named",
        "domains": [
          {"id": "sample/State", "kind": "value"},
          {"id": "sample/Mask", "kind": "flags"},
          {"id": "sample/Slots", "kind": "slots"}
        ],
        "values": [
          {"domain": "sample/State", "value": 1, "owner": "sample/Subject", "name": "PRIMARY", "desc": "I", "access": 25, "synthetic": false},
          {"domain": "sample/State", "value": 2, "owner": "sample/State", "name": "SECONDARY", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "sample/Mask", "value": 1, "owner": "sample/Mask", "name": "READ", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "sample/Mask", "value": 2, "owner": "sample/Mask", "name": "WRITE", "desc": "I", "access": 25, "synthetic": true},
          {"domain": "sample/Slots", "value": 0, "owner": "sample/Slots", "name": "STATE", "desc": "I", "access": 25, "synthetic": true, "element_domain": "sample/State"}
        ],
        "field_bindings": [
          {"owner": "sample/Subject", "name": "state", "desc": "I", "domain": "sample/State"},
          {"owner": "sample/Subject", "name": "mask", "desc": "I", "domain": "sample/Mask"}
        ],
        "return_bindings": [
          {"owner": "sample/Subject", "name": "stateResult", "desc": "()I", "domain": "sample/State"}
        ],
        "parameter_bindings": [
          {"owner": "sample/Subject", "name": "setState", "desc": "(I)V", "index": 0, "domain": "sample/State"},
          {"owner": "sample/Subject", "name": "setMask", "desc": "(I)V", "index": 0, "domain": "sample/Mask"},
          {"owner": "sample/Subject", "name": "select", "desc": "(I)I", "index": 0, "domain": "sample/State"}
        ],
        "index_bindings": [
          {"owner": "sample/Subject", "name": "table", "desc": "[[I", "dimension": 1, "domain": "sample/State"}
        ],
        "return_index_bindings": [
          {"owner": "sample/Subject", "name": "stateRow", "desc": "()[I", "dimension": 0, "domain": "sample/State"}
        ],
        "parameter_index_bindings": [
          {"owner": "sample/Subject", "name": "readState", "desc": "([I)I", "index": 0, "dimension": 0, "domain": "sample/State"}
        ],
        "slot_bindings": [
          {"owner": "sample/Subject", "name": "properties", "desc": "[I", "domain": "sample/Slots"}
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
        private int[][] table = new int[][]{{10, 20, 30}};

        public void setState(int value) { state = value; }
        public void setMask(int value) { mask = value; }
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
    assertTrue(content.contains("mask & Mask.READ"), content);
    assertTrue(content.contains("mask | Mask.WRITE"), content);
    assertTrue(content.contains("properties[Slots.STATE]"), content);
    assertTrue(content.contains("PRIMARY"), content);
    assertTrue(content.contains("[PRIMARY]"), content);
    assertTrue(content.contains("stateRow()[State.SECONDARY]"), content);
    assertTrue(content.contains("values[State.SECONDARY]") || content.contains("var1[State.SECONDARY]"), content);
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve("sample/State.java")));
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve("sample/Mask.java")));
    assertTrue(Files.isRegularFile(fixture.getTargetDir().resolve("sample/Slots.java")));
    recompile();
  }
}
