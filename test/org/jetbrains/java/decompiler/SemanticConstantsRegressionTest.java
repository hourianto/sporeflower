package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class SemanticConstantsRegressionTest extends SemanticTestBase {
  public SemanticConstantsRegressionTest() {
    super("constants", "Subject");
  }

  @Test
  public void testParameterSemanticsDoNotLeakIntoUnrelatedSlotLifetimes() throws IOException {
    Path source = writeSource("sample/ParameterReuseSubject.java", """
      package sample;

      public class ParameterReuseSubject {
        public boolean overwriteMask(int mask, int packed) {
          boolean hadRead = (mask & 1) != 0;
          mask = packed & 255;
          return hadRead && mask != 1;
        }

        public static boolean overwriteMaskAfterWide(long timestamp, int mask, int packed) {
          boolean hadRead = (mask & 1) != 0;
          mask = packed & 255;
          return timestamp > 0 && hadRead && mask != 1;
        }

        public boolean overwriteRecord(int[] record, int[] unrelated) {
          boolean startedSecondary = record[0] == 2;
          record = unrelated;
          return startedSecondary && record[0] == 2;
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());
    String content = decompileDirectory(outRoot(), "sample/ParameterReuseSubject.java");

    assertEquals(2, countOccurrences(content, "& 0xFF"), content);
    assertEquals(2, countOccurrences(content, "!= 1"), content);
    assertEquals(2, countOccurrences(content, "Mask.READ"), content);
    assertEquals(1, countOccurrences(content, "Slots.STATE"), content);
    assertEquals(1, countOccurrences(content, "State.SECONDARY"), content);
    recompile();
  }

  @Test
  public void testSemanticFactsRemainScopedToDefinitionsMergedForPrinting() throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/sample/SemanticDefinitionScopeSubject.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    String content = decompileClassFile(classFile, "sample/SemanticDefinitionScopeSubject.java");

    String stackMerges = methodSource(content, "independentStackMerges");
    assertTrue(stackMerges.contains("rawValue") && stackMerges.contains("100000"), content);
    assertEquals(0, countOccurrences(stackMerges, "Subject.PRIMARY"), content);
    assertEquals(1, countOccurrences(stackMerges, "State.SECONDARY"), content);

    String copiedParameter = methodSource(content, "copiedParameter");
    assertTrue(copiedParameter.contains("case 1:") && copiedParameter.contains("case 2:"), content);
    assertEquals(0, countOccurrences(copiedParameter, "case State."), content);
    assertContains(copiedParameter, "== State.SECONDARY");

    String reusedLocal = methodSource(content, "reusedLocal");
    assertTrue(reusedLocal.contains("case 1:") && reusedLocal.contains("case 2:"), content);
    assertEquals(0, countOccurrences(reusedLocal, "case State."), content);
    assertContains(reusedLocal, "== State.SECONDARY");
    recompile();
  }

  @Test
  public void testParameterSemanticsSurviveDomainPreservingUpdates() throws IOException {
    Path source = writeSource("sample/ParameterUpdateSubject.java", """
      package sample;

      public class ParameterUpdateSubject {
        public boolean narrowMask(int mask) {
          mask = mask & 3;
          return mask == 1;
        }
      }
      """);

    compileJava8NoDebug(source, outRoot());
    String content = decompileDirectory(outRoot(), "sample/ParameterUpdateSubject.java");

    assertEquals(2, countOccurrences(content, "Mask.READ"), content);
    assertEquals(1, countOccurrences(content, "Mask.WRITE"), content);
    recompile();
  }

  @Test
  public void testInlineAssignmentsExposeOnlyUnambiguousRhsDomains() throws IOException {
    Path helpers = writeSource("sample/SemanticHelpers.java", """
      package sample;

      final class SemanticHelpers {
        static int preserve(int value) { return value; }
      }
      """);
    Path source = writeSource("sample/InlineAssignmentSubject.java", """
      package sample;

      public class InlineAssignmentSubject {
        public int stateResult() { return 2; }
        public int maskResult() { return 2; }

        public boolean matchesState() {
          int result;
          return (result = stateResult()) == 2 || result == 0;
        }

        public boolean matchesEither(boolean state) {
          int result;
          return (result = state ? stateResult() : maskResult()) == 2 || result == 0;
        }

        public boolean matchesPassthroughMerge(boolean outer, boolean inner) {
          return (outer ? SemanticHelpers.preserve(inner ? stateResult() : maskResult()) : stateResult()) == 2;
        }
      }
      """);

    compileJava8NoDebug(java.util.List.of(helpers, source), outRoot());
    String content = decompileDirectory(outRoot(), "sample/InlineAssignmentSubject.java");

    assertEquals(2, countOccurrences(content, "State.SECONDARY"), content);
    assertEquals(1, countOccurrences(content, "Mask.WRITE"), content);
    assertEquals(2, countOccurrences(content, "== 0"), content);
    assertEquals(2, countOccurrences(content, "== 2"), content);
    recompile();
  }

  @Test
  public void testBindingsRespectHidingAndCovariantOverrides() throws IOException {
    Path baseResult = writeSource("sample/BaseResult.java", """
      package sample;

      class BaseResult {}
      """);
    Path childResult = writeSource("sample/ChildResult.java", """
      package sample;

      class ChildResult extends BaseResult {}
      """);
    Path base = writeSource("sample/BindingBase.java", """
      package sample;

      class BindingBase {
        int state;

        private boolean privateCheck(int value) { return value == 2; }
        static boolean hiddenCheck(int value) { return value == 2; }
        BaseResult choose(int value) { return value == 2 ? new BaseResult() : null; }
      }
      """);
    Path child = writeSource("sample/BindingChild.java", """
      package sample;

      class BindingChild extends BindingBase {
        int state;

        public boolean privateCheck(int value) { return value == 2; }
        static boolean hiddenCheck(int value) { return value == 2; }
        @Override ChildResult choose(int value) { return value == 2 ? new ChildResult() : null; }
        boolean fieldCheck() { return state == 2; }
      }
      """);

    compileJava8NoDebug(java.util.List.of(baseResult, childResult, base, child), outRoot());
    String content = decompileDirectory(outRoot(), "sample/BindingChild.java");

    assertEquals(1, countOccurrences(content, "State.SECONDARY"), content);
    assertEquals(3, countOccurrences(content, "== 2"), content);
    recompile();
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
        public boolean inlineRecordSecondary() { return readRecord(new int[]{2}); }
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

    assertContains(content, "State.SECONDARY", "Mask.READ | Mask.WRITE", "Mask.READ | 16", "~(Mask.READ | Mask.WRITE)",
      "mask & Mask.READ", "mask | Mask.WRITE", "properties[Slots.STATE]", "PRIMARY", "[PRIMARY]",
      "stateRow()[State.SECONDARY]");
    assertTrue(content.contains("values[State.SECONDARY]") || content.contains("var1[State.SECONDARY]"), content);
    assertContains(content, "records[var1][Slots.STATE] == State.SECONDARY", "recordRow()[Slots.STATE] == State.SECONDARY");
    assertTrue(content.contains("record[Slots.STATE] == State.SECONDARY") || content.contains("var1[Slots.STATE] == State.SECONDARY"), content);
    assertContains(content, "readRecord(new int[]{State.SECONDARY})");
    assertTrue(content.contains("row[1] == State.SECONDARY") || content.contains("var1[1] == State.SECONDARY"), content);
    assertContains(content, "stateValues()[0] == PRIMARY", "var1[0] == State.SECONDARY",
      "new int[][]{{PRIMARY, State.SECONDARY}}", "new int[][]{{State.SECONDARY}}");
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
        public ExternalApi(int anchor) {}
        public static void consume(int anchor) {}
      }
      """);
    Path subject = writeSource("sample/ExternalSubject.java", """
      package sample;

      public class ExternalSubject {
        public void call() {
          ExternalApi.consume(20);
          new ExternalApi(20);
        }
      }
      """);

    compileJava8NoDebug(java.util.List.of(api, subject), outRoot());
    Path libraryRoot = fixture.getTempDir().resolve("external-library");
    Files.createDirectories(libraryRoot.resolve("sample"));
    Files.move(outRoot().resolve("sample/ExternalApi.class"), libraryRoot.resolve("sample/ExternalApi.class"));
    fixture.getDecompiler().addLibrary(libraryRoot.toFile());

    String content = decompileDirectory(outRoot(), "sample/ExternalSubject.java");
    assertEquals(2, countOccurrences(content, "ExternalApi.LEFT | ExternalApi.TOP"), content);
    recompile(java.util.List.of(api));
  }

}
