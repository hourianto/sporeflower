package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;
import java.nio.file.Files;
import java.nio.file.Path;
import static org.junit.jupiter.api.Assertions.*;

public class SemanticValueFlowRegressionTest extends SemanticTestBase {
  public SemanticValueFlowRegressionTest() {
    super("value-flow", "ValueFlowSubject");
  }

  @Test
  public void consumerContractsReachLocalDefinitionsAndArrayConstruction() throws Exception {
    String s = subject("""
        static int action, sink;
        static byte[] edges = {0, 1, 2, 1, 2, 0};
        static void touch(int x) {
          sink = x;
        }
        static void consume(int x) {
          sink = x;
        }
        static void consumeMask(int x) {
          sink = x;
        }
        public static int inline(int x) {
          return x > 0 ? 1 : 2;
        }
        public static int producer(int x) {
          int v = -1;
          if (x > 0) {
            v = 1;
            touch(x);
          }
          if (x < 0) {
            v = 2;
            touch(x + 1);
          }
          return v;
        }
        public static String suffix(int x) {
          String v = "";
          if (x > 0) {
            v = "s";
            touch(x);
          }
          return v;
        }
        public static int caller(int x) {
          int v = 1;
          if (x > 0) {
            v = 2;
            touch(x);
          }
          consume(v);
          return sink;
        }
        public static int maskCaller(int x) {
          int v = 1;
          if (x > 0) {
            v = 2;
            touch(x);
          }
          if (x < 0)
            v |= 16;
          consumeMask(v);
          return sink;
        }
        public static int[] point(int x) {
          int[] p = new int[2];
          if (x > 0)
            p[0] += x;
          if (x < 0)
            p[1] -= x;
          return p;
        }
        public static int[][] assemble(int x) {
          int[][] p = new int[2][];
          for (int i = 0; i < 2; i++) p[i] = point(x + i);
          if (p[0][1] > 0)
            touch(p[1][0]);
          return p;
        }
        public static boolean boundary(int x) {
          action = x;
          return action >= 0;
        }
        public static boolean record(int x) {
          for (int i = 0; i < edges.length; i += 3) {
            if ((edges[i] == 0 && edges[i + 1] == x || edges[i] == 1 && edges[i + 1] == x + 1) && edges[i + 2] == 2)
              return true;
          }
          return false;
        }
        public static boolean recordFixed(int x) {
          for (int i = 0; i < 6; i += 3) {
            if ((edges[i] == 0 && edges[i + 1] == x || edges[i] == 1 && edges[i + 1] == x + 1) && edges[i + 2] == 2)
              return true;
          }
          return false;
        }
        public static boolean recordMandatory(int x) {
          for (int i = 0; i < edges.length; i += 3) {
            int side = edges[i], type = edges[i + 1];
            if ((side == 0 && type == x || side == 1 && type == x + 1) && edges[i + 2] == 2)
              return true;
          }
          return false;
        }
        public static boolean recordBounded(int x) {
          for (int i = 0; i < edges.length - 2; i += 3) {
            if ((edges[i] == 0 && edges[i + 1] == x || edges[i] == 1 && edges[i + 1] == x + 1) && edges[i + 2] == 2)
              return true;
          }
          return false;
        }
      """);

    assertContains(s, "Action.SECOND");
    assertContains(methodSource(s, "producer"), "Action.NONE", "Action.SECOND", "Action.THIRD");
    assertContains(methodSource(s, "suffix"), "Suffix.FIRST", "Suffix.SECOND");
    assertContains(methodSource(s, "caller"), "Action.SECOND", "Action.THIRD");
    assertContains(methodSource(s, "maskCaller"), "Mask.BACK", "Mask.CONFIRM", "|= Mask.BUILD");
    assertContains(methodSource(s, "point"), "[Point.X]", "[Point.Y]", "new int[2]");
    assertContains(methodSource(s, "assemble"), "[Point.X]", "[Point.Y]");
    assertContains(methodSource(s, "boundary"), ">= 0");
    assertMethodsContain(s, new String[]{"record", "recordBounded", "recordFixed", "recordMandatory"},
      "Edge.TYPE", "Edge.DIRECTION", "Side.CURRENT", "+= 3");
    recompile();
    assertSameIntResults(new String[]{"inline", "producer", "suffix", "caller", "maskCaller", "boundary", "record", "recordBounded",
      "recordFixed", "recordMandatory"}, -2, -1, 0, 1, 2, Integer.MIN_VALUE, Integer.MAX_VALUE);
    assertSameIntResults(new String[]{"point", "assemble"}, -2, -1, 0, 1, 2);
  }

  @Test
  public void requirementsPreserveConflictsAndOpaqueBoundaries() throws Exception {
    String source = subject("""
        static int sink;
        static void touch(int value) {
          sink = value;
        }
        static void consume(int value) {
          sink = value;
        }
        static void consumeOther(int value) {
          sink = value;
        }
        static int identity(int value) {
          return value;
        }
        public static int conflict(int input) {
          int value = 1;
          if (input > 0) {
            value = 2;
            touch(input);
          }
          consume(value);
          consumeOther(value);
          return value;
        }
        public static int opaque(int input) {
          int value = 1;
          if (input > 0) {
            value = input;
            touch(input);
          }
          consume(value);
          return value == 1 ? 7 : 8;
        }
        public static int throughHelper(int input) {
          int value = 1;
          if (input > 0) {
            value = 2;
            touch(input);
          }
          consume(identity(value));
          return sink;
        }
      """);
    assertNotContains(methodSource(source, "conflict"), "Action.");
    assertNotContains(methodSource(source, "opaque"), "Action.");
    assertContains(methodSource(source, "throughHelper"), "Action.SECOND", "Action.THIRD");
    recompile();
    compareScalars("conflict", "opaque", "throughHelper");
  }

  @Test
  public void allocationShapesFollowAliasesAndAgreeingRowsButStopAtEscapes() throws Exception {
    String source = subject("""
        static int sink;
        static void touch(int value) {
          sink = value;
        }
        static void mutate(int[] values) {
          values[0] = 42;
        }
        static int[] point(int input) {
          return new int[] {input, input + 1};
        }
        static int[] otherPoint(int input) {
          return new int[] {input, input + 1};
        }
        public static int[] aliasPoint(int input) {
          int[] result = new int[2];
          int[] alias = result;
          alias[0] = input;
          if (input > 0)
            alias[1] = input + 1;
          return result;
        }
        public static int[] escapedPoint(int input) {
          int[] result = new int[2];
          mutate(result);
          result[1] = input;
          return result;
        }
        public static int inferredRows(int input) {
          int[][] rows = new int[2][];
          rows[0] = point(input);
          rows[1] = point(input + 1);
          if (rows[0][0] == 42)
            touch(input);
          return rows[1][1];
        }
        public static int conflictingRows(int input) {
          int[][] rows = new int[2][];
          rows[0] = point(input);
          rows[1] = otherPoint(input + 1);
          if (rows[0][0] == 42)
            touch(input);
          return rows[1][1];
        }
        public static Object[] cycle() {
          Object[] result = new Object[1];
          result[0] = result;
          return result;
        }
      """);
    assertContains(methodSource(source, "aliasPoint"), "Point.X", "Point.Y");
    assertNotContains(methodSource(source, "escapedPoint"), "Point.");
    assertContains(methodSource(source, "inferredRows"), "Point.X", "Point.Y");
    assertNotContains(methodSource(source, "conflictingRows"), "Point.");
    recompile();
    compareScalars("inferredRows", "conflictingRows");
  }

  @Test
  public void shortCircuitAndCaughtExceptionsCannotInventLoopAlignment() throws Exception {
    String source = subject("""
        static byte[] edges = {0, 1, 2, 1, 2, 0};
        public static int optional(boolean read) {
          int result = 0;
          for (int i = 0; i < edges.length; i += 3)
            if (read && edges[i + 1] == 1)
              result++;
          return result;
        }
        public static int caught() {
          int result = 0;
          for (int i = 0; i < edges.length; i += 3) {
            try {
              if (edges[i] == 0 && edges[i + 1] == 1)
                result++;
            } catch (ArrayIndexOutOfBoundsException failure) {
            }
          }
          return result;
        }
      """);
    assertNotContains(methodSource(source, "optional"), "Edge.TYPE");
    assertNotContains(methodSource(source, "caught"), "Edge.TYPE");
    recompile();
  }

  @Test
  public void returnedContainerContractsReachBranchBuiltBoxedElements() throws Exception {
    String source = subject("""
        static int sink;
        static void touch(int value) {
          sink = value;
        }
        public static java.util.Vector createTokens(int input) {
          java.util.Vector result = new java.util.Vector();
          int value = 1;
          if (input > 0) {
            value = 2;
            touch(input);
          }
          result.addElement(new Integer(value));
          return result;
        }
      """);
    assertContains(methodSource(source, "createTokens"), "Action.SECOND", "Action.THIRD");
    recompile();
    compareScalars("createTokens");
  }

  @Test
  public void arithmeticUpdatesDoNotRetypeTheirDeltasOrEarlierDefinitions() throws Exception {
    String source = subject("""
        static int sink;
        static void touch(int value) {
          sink = value;
        }
        public static int incremented(int input) {
          int value = 0;
          while (input-- > 0) {
            value++;
            touch(value);
          }
          return value;
        }
        public static int[] updatedArray(int input) {
          int[] result = new int[2];
          touch(input);
          result[0] += 2;
          if (input > 0)
            result[1] -= 1;
          return result;
        }
      """);
    assertNotContains(methodSource(source, "incremented"), "Action.");
    assertNotContains(methodSource(source, "updatedArray"), "Action.");
    recompile();
  }

  @Test
  public void invocationRequirementsRemainScopedWhenFollowingLocalDefinitions() throws Exception {
    Path source = writeSource("sample/ValueFlowSubject.java", """
      package sample;
      public class ValueFlowSubject {
        static int sink;
        static void touch(int value) {
          sink = value;
        }
        static int send(int value) {
          return value == 2 ? 1 : 0;
        }
        public static int scoped(int input) {
          int value = 1;
          if (input > 0) {
            value = 2;
            touch(input);
          }
          return send(value);
        }
        public static int unbound(int input) {
          int value = 1;
          if (input > 0) {
            value = 2;
            touch(input);
          }
          return send(value);
        }
      }
      """);
    compileJava8NoDebug(source, outRoot());
    int[] instruction = {-1}, selected = {-1};
    new org.objectweb.asm
      .ClassReader(Files.readAllBytes(outRoot().resolve("sample/ValueFlowSubject.class"))) {
        @Override
        protected void readBytecodeInstructionOffset(int offset) {
          instruction[0] = offset;
        }
      }
      .accept(new org.objectweb.asm.ClassVisitor(org.objectweb.asm.Opcodes.ASM9) {
        @Override
        public org.objectweb.asm.MethodVisitor visitMethod(
          int access, String name, String descriptor, String signature, String[] exceptions) {
          if (!name.equals("scoped"))
            return null;
          return new org.objectweb.asm.MethodVisitor(org.objectweb.asm.Opcodes.ASM9) {
            @Override
            public void visitMethodInsn(int opcode, String owner, String name, String descriptor, boolean isInterface) {
              if (name.equals("send"))
                selected[0] = instruction[0];
            }
          };
        }
      }, org.objectweb.asm.ClassReader.SKIP_DEBUG | org.objectweb.asm.ClassReader.SKIP_FRAMES);
    assertTrue(selected[0] >= 0);
    var data = org.jetbrains.java.decompiler.api.SemanticMappingData.read(Path.of("testData/semantic/value-flow.json"));
    var site = new org.jetbrains.java.decompiler.api.SemanticMappingData.CallBindingEntry(
      new org.jetbrains.java.decompiler.api.SemanticMappingData.TargetEntry("return", "sample/ValueFlowSubject", "scoped", "(I)I", null),
      selected[0],
      new org.jetbrains.java.decompiler.api.SemanticMappingData.TargetEntry("return", "sample/ValueFlowSubject", "send", "(I)I", null),
      "sample/Action", 0);
    var scoped = new org.jetbrains.java.decompiler.api.SemanticMappingData(data.domains(), data.values(), data.scalarBindings(),
      data.arrayBindings(), data.returnDomainSources(), java.util.List.of(site), data.stringValues(), data.conditionalBindings(),
      data.containerBindings(), data.slotDomainSources(), data.classNameLiterals());
    // This test injects mappings by instruction offset after compiling the fixture.
    fixture.getDecompiler();
    org.jetbrains.java.decompiler.main.DecompilerContext.setProperty(org.jetbrains.java.decompiler.main.DecompilerContext.SEMANTIC_MAPPINGS,
      org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.fromData(scoped));
    String content = decompileDirectory(outRoot(), "sample/ValueFlowSubject.java");
    assertContains(methodSource(content, "scoped"), "Action.SECOND", "Action.THIRD");
    assertNotContains(methodSource(content, "unbound"), "Action.");
    assertNotContains(methodSource(content, "send"), "Action.");
    recompile();
    compareScalars("scoped", "unbound");
  }

  @Test
  public void aTypedColumnDoesNotBecomeTheWholeRecordsDefaultDomain() throws Exception {
    String source = subject("""
        static int producer(int input) { return input == 0 ? 1 : 2; }
        public static int[] message(int input) {
          int[] result = new int[]{producer(input), 0};
          if (input > 0) result[1] = 1;
          return result;
        }
      """);
    String message = methodSource(source, "message");
    assertNotContains(message, "Action.");
    assertContains(message, "[MessageSlot.PAYLOAD] = 1");
    recompile();
  }

  @Test
  public void boundedLoopSelectionsKeepTheirDomainWithoutNamingLoopArithmetic() throws Exception {
    String source = subject("""
        static int[] weights = {5, 2, 7};
        static int sink;
        static void consume(int value) { sink = value; }
        public static int selected(int input) {
          int best = -1, score = input;
          for (int i = 0; i < 3; i++) {
            if (weights[i] < score) { best = i; score = weights[i]; }
          }
          int choice = -1;
          if (input > 0) choice = best;
          else if (best == 0) choice = 0;
          consume(choice);
          return choice == -1 ? 7 : choice;
        }
        public static int descending(int input) {
          int best = -1;
          for (int i = 2; 0 <= i; i--) { if (weights[i] < input) best = i; }
          consume(best);
          return best == -1 ? 7 : best;
        }
        public static int outside(int input) {
          int i;
          for (i = 0; i < 3; i++) { if (weights[i] == input) break; }
          consume(i);
          return i == 0 ? 7 : i;
        }
        public static int mutated(int input) {
          int best = -1;
          for (int i = 0; i < 3; i++) {
            if (input > 0) i += input;
            best = i;
          }
          consume(best);
          return best == -1 ? 7 : best;
        }
      """);
    String selected = methodSource(source, "selected");
    assertContains(selected, "Action.NONE", "Action.FIRST");
    assertTrue(selected.matches("(?s).*for \\(int \\w+ = 0; \\w+ < 3; \\w+\\+\\+\\).*"), source);
    assertContains(methodSource(source, "descending"), "Action.NONE");
    assertNotContains(methodSource(source, "descending"), "= Action.THIRD");
    assertNotContains(methodSource(source, "outside"), "Action.");
    assertNotContains(methodSource(source, "mutated"), "Action.");
    recompile();
    compareScalars("selected", "descending", "outside");
  }

  @Test
  public void booleanCorrelationsSelectReachingArrayDefinitions() throws Exception {
    String source = subject("""
        static int sink;
        static int[] replace(int[] path) {
          int[] result = new int[path.length];
          System.arraycopy(path, 0, result, 0, path.length);
          return result;
        }
        public static int guardedPath(int[] path, boolean change) {
          boolean replaced = false;
          if (change) { replaced = true; path = replace(path); }
          sink++;
          return !replaced ? path[0] + path[1] : 0;
        }
        public static int oppositePath(int[] path, boolean change) {
          boolean replaced = false;
          if (change) { replaced = true; path = replace(path); }
          sink++;
          return replaced ? path[0] + path[1] : 0;
        }
        public static int reassignedFlag(int[] path, boolean change) {
          boolean replaced = false;
          if (change) { replaced = true; path = replace(path); }
          replaced = false;
          sink++;
          return !replaced ? path[0] + path[1] : 0;
        }
        public static int copiedFlag(int[] path, boolean change) {
          boolean replaced = false;
          if (change) { replaced = true; path = replace(path); }
          boolean unchanged = !replaced;
          sink++;
          if (unchanged) return path[0] + path[1];
          return 0;
        }
        public static int exceptionPath(int[] path, boolean change) {
          boolean unchanged = true;
          try {
            if (change) { path = replace(path); sink = 1 / sink; unchanged = false; }
          } catch (ArithmeticException failure) { }
          return unchanged ? path[0] + path[1] : 0;
        }
        public static int repeatedPath(int[] path, boolean change) {
          int result = 0;
          for (int i = 0; i < 2; i++) {
            boolean replaced = false;
            if (change && i == 0) { replaced = true; path = replace(path); }
            if (!replaced) result += path[0] + path[1];
          }
          return result;
        }
      """);
    assertMethodsContain(source, new String[]{"guardedPath", "copiedFlag"}, "Point.X", "Point.Y");
    assertMethodsDoNotContain(source, new String[]{"oppositePath", "reassignedFlag", "exceptionPath", "repeatedPath"}, "Point.");
    recompile();
    try (BehaviorComparison behavior = compareBehavior()) {
      for (String name : new String[]{"guardedPath", "oppositePath", "reassignedFlag", "copiedFlag", "exceptionPath", "repeatedPath"}) {
        for (boolean change : new boolean[]{false, true}) {
          behavior.assertSame(name + ", change=" + change, loader -> loader.loadClass("sample.ValueFlowSubject")
            .getMethod(name, int[].class, boolean.class).invoke(null, new int[]{11, 29}, change));
        }
      }
    }
  }

  @Test
  public void objectAliasesAndNestedPublicationCannotHideArrayEscapes() throws Exception {
    String source = subject("""
        static int sink;
        static Object published;
        static void touch(int input) { sink = input; }
        static void mutate(Object input) { ((int[])input)[0] = 42; }
        static void mutateBox(Object input) { ((int[])((Object[])input)[0])[0] = 42; }
        public static int[] objectEscape(int input) {
          int[] result = new int[2];
          Object alias = result;
          mutate(alias);
          result[1] = input;
          return result;
        }
        public static int[] nestedEscape(int input) {
          int[] result = new int[2];
          Object[] box = new Object[]{result};
          mutateBox(box);
          result[1] = input;
          return result;
        }
        public static int[] fieldEscape(int input) {
          int[] result = new int[2];
          Object alias = result;
          published = alias;
          touch(input);
          result[1] = input;
          return result;
        }
        public static int[] copiedSource(int input) {
          int[] result = new int[2];
          int[] copy = new int[2];
          System.arraycopy(result, 0, copy, 0, 2);
          result[1] = input;
          return result;
        }
        public static int[] localBox(int input) {
          int[] result = new int[2];
          Object[] box = new Object[]{result};
          touch(box.length);
          result[1] = input;
          return result;
        }
      """);
    assertMethodsDoNotContain(source, new String[]{"objectEscape", "nestedEscape", "fieldEscape"}, "Point.");
    assertContains(methodSource(source, "localBox"), "Point.Y");
    assertContains(methodSource(source, "copiedSource"), "Point.Y");
    recompile();
  }

  @Test
  public void arrayCopiesKeepExplicitDestinationContractsWithoutTypingTheirSources() throws Exception {
    String source = subject("""
        public static short[] copiedSigns(boolean choose, boolean include) {
          int symbol;
          if (choose) symbol = 1; else symbol = 2;
          short[] input = new short[]{1, 2};
          short[] result = new short[3];
          System.arraycopy(input, 0, result, 1, 2);
          if (include) result[0] = (short)symbol;
          return result;
        }
        public static byte[] copiedDecimal(int input) {
          byte[] digits = new byte[]{1, 2, 3, 4, 5, 6, 7, 8};
          int count = 1 + (input & 7);
          byte[] result = new byte[count + 2];
          System.arraycopy(digits, 0, result, 0, count);
          if (count == 1) {
            result[count + 1] = result[0];
            result[count] = -1;
            result[count - 1] = 0;
          } else {
            result[count] = result[count - 1];
            result[count - 1] = -1;
          }
          return result;
        }
        public static int[] point(int input) { return new int[]{input, input + 1}; }
        public static int[] copiedPoint(int input) {
          int[] result = new int[2];
          Object alias = result;
          System.arraycopy(new int[]{input, 0}, 0, alias, 0, 2);
          result[1] = input + 1;
          return result;
        }
        public static int copiedRows(int input) {
          int[][] result = new int[][]{point(input)};
          System.arraycopy(new int[][]{new int[]{input, input + 2}}, 0, result, 0, 1);
          return result[0][1];
        }
        static void mutate(Object input) { ((int[])input)[0] = 42; }
        static void mutateBox(Object input) { mutate(((Object[])input)[0]); }
        public static int[] escapedCopy(int input) {
          int[] result = new int[2];
          System.arraycopy(new int[]{input, 0}, 0, result, 0, 2);
          mutate(result);
          result[1] = input;
          return result;
        }
        public static int[] copiedEscape(int input) {
          int[] result = new int[2];
          Object[] holder = new Object[]{result};
          Object[] copy = new Object[1];
          System.arraycopy(holder, 0, copy, 0, 1);
          mutateBox(copy);
          result[1] = input;
          return result;
        }
      """);
    String signs = methodSource(source, "copiedSigns"), decimal = methodSource(source, "copiedDecimal");
    assertContains(signs, "Action.SECOND", "Action.THIRD", "new short[]{1, 2}");
    assertContains(decimal, "Action.NONE", "Action.FIRST", "new byte[]{1, 2, 3, 4, 5, 6, 7, 8}");
    assertContains(methodSource(source, "copiedPoint"), "Point.Y");
    assertMethodsDoNotContain(source, new String[]{"copiedRows", "escapedCopy", "copiedEscape"}, "Point.");
    recompile();
    compareScalars("copiedRows");
    assertSameResults(new String[]{"copiedSigns"}, new Class<?>[]{boolean.class, boolean.class},
      new Object[]{false, false}, new Object[]{false, true}, new Object[]{true, false}, new Object[]{true, true});
    assertSameIntResults(new String[]{"copiedDecimal", "copiedPoint", "escapedCopy", "copiedEscape"},
      Integer.MIN_VALUE, -1, 0, 1, 7, Integer.MAX_VALUE);
  }

  @Test
  public void inferredTableShapesFeedLaterDependentConsumerRequirements() throws Exception {
    String source = subject("""
        static int sink;
        static void touch(int input) { sink = input; }
        static int lookup(int[] table, int key) { return table[0] == key ? 1 : 0; }
        public static int[] lateTable(int input) {
          int[] table = new int[2];
          table[1] = input;
          int choice = 1;
          if (input > 0) { choice = 2; touch(input); }
          sink = lookup(table, choice);
          return table;
        }
      """);
    assertContains(methodSource(source, "lateTable"), "Action.SECOND", "Action.THIRD");
    recompile();
  }

  @Test
  public void knownProducerJoinsNameNeutralAlternativesAtTheRead() throws Exception {
    String source = subject("""
        static int action, mask;
        static int producer(int input) { return input > 0 ? 1 : 2; }
        public static int inferred(int input) {
          int result = action;
          if (input > 0) result = 1;
          return result;
        }
        public static int recovered(int input) {
          int result = -1;
          try { result = producer(7 / input); } catch (ArithmeticException failure) { }
          return result;
        }
        public static int styles(int input) {
          int result = mask;
          switch (input) {
            case 0: result = (input > 1 ? 1 : 0) | (input > 2 ? 2 : 0); break;
            case 1: result = 16;
          }
          return result;
        }
      """);
    assertContains(methodSource(source, "inferred"), "Action.SECOND");
    assertContains(methodSource(source, "recovered"), "Action.NONE");
    assertContains(methodSource(source, "styles"), "Mask.BACK", "Mask.CONFIRM", "Mask.BUILD");
    recompile();
    compareScalars("inferred", "recovered", "styles");
  }

  @Test
  public void compatibleNumericFormatsSurviveDistinctSemanticDomains() throws Exception {
    String source = subject("""
        static int sink;
        static void rgb(int color) { sink = color; }
        static void argb(int color) { sink = color; }
        public static int sharedColor(int input) {
          int color = -65281;
          if (input > 0) { color = -1; sink = input; }
          rgb(color);
          argb(color);
          return color;
        }
      """);
    assertContains(methodSource(source, "sharedColor"), "0xFFFF00FF", "0xFFFFFFFF");
    recompile();
    compareScalars("sharedColor");
  }

  @Test
  public void narrowCountersUseTheirActualStorageOverflowBounds() throws Exception {
    String source = subject("""
        static int sink;
        static int[] weights = {5, 2, 7};
        static void consume(int value) { sink = value; }
        public static int narrowBounded(int input) {
          int choice = -1;
          for (byte i = 0; i < 3; i++) if (weights[i] < input) choice = i;
          consume(choice);
          return choice;
        }
        public static int narrowWrapped(int input) {
          int choice = -1, count = 0;
          for (byte i = 0; i < 200; i++) {
            if (i == input) choice = i;
            if (++count > 260) break;
          }
          consume(choice);
          return choice;
        }
        public static int charWrapped(int input) {
          int choice = -1, count = 0;
          for (char i = 2; i >= 0; i--) {
            if (i == input) choice = i;
            if (++count > 4) break;
          }
          consume(choice);
          return choice;
        }
      """);
    assertContains(methodSource(source, "narrowBounded"), "Action.NONE");
    assertNotContains(methodSource(source, "narrowWrapped"), "Action.");
    assertNotContains(methodSource(source, "charWrapped"), "Action.");
    recompile();
    compareScalars("narrowBounded", "narrowWrapped", "charWrapped");
  }

  @Test
  public void mappedTablePublicationSuppliesItsRowContract() throws Exception {
    String source = subject("""
        static int[][] rows = new int[2][];
        static Object[] rawRows = new Object[2];
        static int sink;
        static void touch(int input) { sink = input; }
        public static int[] publishRows(int input) {
          int[] result = new int[2];
          touch(input);
          result[0] = input;
          result[1] = input + 1;
          int[][] target = rows;
          target[input & 1] = result;
          return result;
        }
        public static int[] rawPublishRows(int input) {
          int[] result = new int[2];
          touch(input);
          result[0] = input;
          result[1] = input + 1;
          Object[] box = new Object[]{result};
          rawRows[input & 1] = box;
          return result;
        }
      """);
    assertContains(methodSource(source, "publishRows"), "Point.X", "Point.Y");
    assertNotContains(methodSource(source, "rawPublishRows"), "Point.");
    recompile();
  }

  private void compareScalars(String... methods) throws Exception {
    assertSameIntResults(methods, Integer.MIN_VALUE, -2, -1, 0, 1, 2, Integer.MAX_VALUE);
  }
}
