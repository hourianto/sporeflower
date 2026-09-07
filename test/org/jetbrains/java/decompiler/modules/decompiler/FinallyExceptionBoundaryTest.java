package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.ExceptionTable;
import org.jetbrains.java.decompiler.code.FullInstructionSequence;
import org.jetbrains.java.decompiler.code.Instruction;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.code.cfg.ControlFlowGraph;
import org.jetbrains.java.decompiler.code.cfg.ExceptionRangeCFG;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.code.DeadCodeHelper;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

public class FinallyExceptionBoundaryTest {
  private static final BytecodeVersion JAVA_1 = new BytecodeVersion(BytecodeVersion.MAJOR_1_0_2, 3);

  @BeforeEach
  public void setUp() {
    MinimalFernflowerEnvironment.setup();
  }

  @AfterEach
  public void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  public void fixtureReturnsAcquireDifferentExceptionMemberships() throws Exception {
    Path path = Path.of("testData/classes/jasm/pkg/TestFinallyReturnRange.class");
    try (DataInputFullStream stream = new DataInputFullStream(Files.readAllBytes(path))) {
      StructClass type = StructClass.create(stream, true);
      StructMethod method = type.getMethod("run", "()V");
      method.expandData(type);
      ControlFlowGraph graph = new ControlFlowGraph(method.getInstructionSequence());
      List<BasicBlock> returns = graph.getBlocks().stream()
        .filter(block -> block.getLastInstruction().opcode == CodeConstants.opc_return).toList();
      assertEquals(2, returns.size());
      assertTrue(returns.stream().allMatch(block -> block.getSuccExceptions().isEmpty()));
      DeadCodeHelper.incorporateValueReturns(graph);
      assertEquals(1, returns.get(0).getSuccExceptions().size());
      assertTrue(returns.get(1).getSuccExceptions().isEmpty());
    }
  }

  @Test
  public void removedCleanupOutsideCatchRetainsItsBoundary() throws Exception {
    checkRetainedBoundary(false);
  }

  @Test
  public void preExistingEmptyBoundaryIsNotDiscarded() throws Exception {
    // A previous finally deletion can leave an empty block representing real
    // cleanup outside a catch. Globally ignoring empty blocks loses that fact.
    checkRetainedBoundary(true);
  }

  private static void checkRetainedBoundary(boolean empty) throws Exception {
    ControlFlowGraph graph = graph();
    BasicBlock next = graph.getFirst();
    BasicBlock predecessor = block(graph, CodeConstants.opc_nop);
    BasicBlock cleanup = block(graph, empty ? -1 : CodeConstants.opc_invokestatic);
    BasicBlock outer = block(graph, CodeConstants.opc_return);
    BasicBlock inner = block(graph, CodeConstants.opc_return);
    predecessor.addSuccessor(cleanup);
    cleanup.addSuccessor(next);
    graph.setFirst(predecessor);
    protect(graph, outer, predecessor, cleanup, next);
    protect(graph, inner, predecessor, next);

    Class<?> areaType = Class.forName(FinallyProcessor.class.getName() + "$Area");
    Constructor<?> constructor = areaType.getDeclaredConstructor(BasicBlock.class, Set.class, BasicBlock.class, Set.class);
    constructor.setAccessible(true);
    Object area = constructor.newInstance(cleanup, Set.of(cleanup), next, Set.of());
    Method delete = FinallyProcessor.class.getDeclaredMethod("deleteArea", ControlFlowGraph.class, areaType);
    delete.setAccessible(true);
    delete.invoke(null, graph, area);

    BasicBlock boundary = predecessor.getSuccs().get(0);
    assertNotSame(next, boundary);
    assertTrue(boundary.getSeq().isEmpty());
    assertEquals(List.of(next), boundary.getSuccs());
    assertEquals(List.of(outer), boundary.getSuccExceptions());
    assertTrue(graph.getExceptionRange(outer, boundary).getProtectedRange().contains(boundary));
    DeadCodeHelper.removeEmptyBlocks(graph);
    assertSame(boundary, predecessor.getSuccs().get(0), "The real exception boundary must survive empty-block removal");
  }

  @Test
  public void identicalReturnsCanHaveDifferentIncorporatedCoverage() throws Exception {
    ControlFlowGraph graph = graph();
    BasicBlock first = graph.getFirst();
    BasicBlock second = block(graph, CodeConstants.opc_return);
    BasicBlock handler = block(graph, CodeConstants.opc_return);
    protect(graph, handler, first);
    assertSame(first, continuation(graph, first, second));
  }

  @Test
  public void identicalThrowingContinuationsMustKeepTheirHandlers() throws Exception {
    ControlFlowGraph graph = graph();
    BasicBlock first = block(graph, CodeConstants.opc_invokestatic);
    BasicBlock second = block(graph, CodeConstants.opc_invokestatic);
    BasicBlock handler = block(graph, CodeConstants.opc_return);
    protect(graph, handler, first);
    assertNull(continuation(graph, first, second));
    protect(graph, handler, second);
    assertSame(first, continuation(graph, first, second));
  }

  @Test
  public void conditionalSuccessorOrderIsSignificant() throws Exception {
    ControlFlowGraph graph = graph();
    BasicBlock first = block(graph, CodeConstants.opc_ifeq);
    BasicBlock second = block(graph, CodeConstants.opc_ifeq);
    BasicBlock yes = block(graph, CodeConstants.opc_return);
    BasicBlock no = block(graph, CodeConstants.opc_return);
    first.addSuccessor(yes);
    first.addSuccessor(no);
    second.addSuccessor(no);
    second.addSuccessor(yes);
    assertNull(continuation(graph, first, second));
  }

  private static BasicBlock continuation(ControlFlowGraph graph, BasicBlock first, BasicBlock second) throws Exception {
    Method method = FinallyProcessor.class.getDeclaredMethod("getUniqueNext", ControlFlowGraph.class, Collection.class, boolean.class);
    method.setAccessible(true);
    return (BasicBlock)method.invoke(null, graph, List.of(
      new BasicBlock[]{first, first, first}, new BasicBlock[]{second, second, second}), true);
  }

  private static ControlFlowGraph graph() {
    return new ControlFlowGraph(new FullInstructionSequence(
      List.of(instruction(CodeConstants.opc_return, 0)), Map.of(0, 0), ExceptionTable.EMPTY));
  }

  private static BasicBlock block(ControlFlowGraph graph, int opcode) {
    BasicBlock block = new BasicBlock(++graph.last_id);
    if (opcode >= 0) {
      block.getSeq().addInstruction(instruction(opcode, block.id));
      block.getInstrOldOffsets().add(block.id);
    }
    graph.getBlocks().addWithKey(block, block.id);
    return block;
  }

  private static Instruction instruction(int opcode, int offset) {
    int group = opcode == CodeConstants.opc_return ? CodeConstants.GROUP_RETURN : CodeConstants.GROUP_GENERAL;
    return Instruction.create(opcode, false, group, JAVA_1,
      opcode == CodeConstants.opc_invokestatic || opcode == CodeConstants.opc_ifeq ? new int[]{1} : null, offset, 1);
  }

  private static void protect(ControlFlowGraph graph, BasicBlock handler, BasicBlock... blocks) {
    graph.getExceptions().add(new ExceptionRangeCFG(new ArrayList<>(List.of(blocks)), handler, "java/lang/Exception"));
    for (BasicBlock block : blocks) {
      block.addSuccessorException(handler);
    }
  }
}
