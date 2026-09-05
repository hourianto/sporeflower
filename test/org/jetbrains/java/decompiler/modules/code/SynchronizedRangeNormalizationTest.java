package org.jetbrains.java.decompiler.modules.code;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.code.cfg.ControlFlowGraph;
import org.jetbrains.java.decompiler.code.cfg.ExceptionRangeCFG;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.Init;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.*;

public class SynchronizedRangeNormalizationTest {
  @BeforeEach
  public void setUp() {
    MinimalFernflowerEnvironment.setup();
    Init.init();
  }

  @AfterEach
  public void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  public void normalizationPreservesEveryThrowingInstructionsHandlers() throws IOException {
    for (String method : List.of("run", "runCatchAll")) {
      ControlFlowGraph graph = fixtureGraph(method);
      Map<Integer, List<String>> before = throwingCoverage(graph);
      assertTrue(ExceptionDeobfuscator.normalizeSynchronizedRanges(graph));
      assertEquals(before, throwingCoverage(graph), "Only non-throwing instruction coverage may change");
      assertFalse(ExceptionDeobfuscator.normalizeSynchronizedRanges(graph), "Normalization should be idempotent");
    }
  }

  @Test
  public void narrowerCatchIsNotPromotedToCatchAll() throws IOException {
    ControlFlowGraph graph = fixtureGraph("run");
    for (ExceptionRangeCFG range : graph.getExceptions()) {
      range.getExceptionTypes().set(0, "java/lang/Exception");
    }
    Map<Integer, List<String>> before = throwingCoverage(graph);
    List<List<BasicBlock>> ranges = rangeContents(graph);
    assertFalse(ExceptionDeobfuscator.normalizeSynchronizedRanges(graph));
    assertEquals(before, throwingCoverage(graph));
    assertEquals(ranges, rangeContents(graph));
  }

  @Test
  public void handlerCatchingItsOwnRethrowIsNotNormalized() throws IOException {
    ControlFlowGraph graph = fixtureGraph("run");
    ExceptionRangeCFG selfProtected = graph.getExceptions().get(1);
    BasicBlock rethrow = graph.getBlocks().stream()
      .filter(block -> block.getLastInstruction().opcode == CodeConstants.opc_athrow).findFirst().orElseThrow();
    // This is a different program: a rethrow re-enters cleanup indefinitely.
    // A synchronized statement would instead propagate that exception.
    selfProtected.getProtectedRange().add(rethrow);
    rethrow.addSuccessorException(selfProtected.getHandler());
    Map<Integer, List<String>> before = throwingCoverage(graph);
    List<List<BasicBlock>> ranges = rangeContents(graph);
    assertFalse(ExceptionDeobfuscator.normalizeSynchronizedRanges(graph));
    assertEquals(before, throwingCoverage(graph));
    assertEquals(ranges, rangeContents(graph));
  }

  private static ControlFlowGraph fixtureGraph(String methodName) throws IOException {
    Path path = Path.of("testData/classes/jasm/pkg/TestObjectLocalReusedForMonitorThrowable.class");
    try (DataInputFullStream stream = new DataInputFullStream(Files.readAllBytes(path))) {
      StructClass type = StructClass.create(stream, true);
      StructMethod method = type.getMethod(methodName, "()V");
      method.expandData(type);
      return new ControlFlowGraph(method.getInstructionSequence());
    }
  }

  private static List<List<BasicBlock>> rangeContents(ControlFlowGraph graph) {
    return graph.getExceptions().stream().map(range -> List.copyOf(range.getProtectedRange())).toList();
  }

  private static Map<Integer, List<String>> throwingCoverage(ControlFlowGraph graph) {
    Map<Integer, List<String>> result = new LinkedHashMap<>();
    for (BasicBlock block : graph.getBlocks()) {
      for (var instruction : block.getSeq()) {
        if (!instruction.cannotThrow()) {
          result.put(instruction.startOffset, graph.getExceptions().stream()
            .filter(range -> range.getProtectedRange().contains(block))
            .map(range -> range.getHandler().id + ":" +
              (range.getExceptionTypes() == null ? List.of("java/lang/Throwable") : range.getExceptionTypes()))
            .toList());
        }
      }
    }
    return result;
  }
}
