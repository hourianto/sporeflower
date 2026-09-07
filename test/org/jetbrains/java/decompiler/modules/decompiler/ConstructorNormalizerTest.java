package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.SynchronizedStatement;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

public class ConstructorNormalizerTest {
  private DirectGraph graph;

  @BeforeEach
  public void setUp() {
    MinimalFernflowerEnvironment.setup();
    graph = new DirectGraph();
  }

  @AfterEach
  public void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  public void allocationMustDominateEveryInitialization() {
    DirectNode entry = node();
    DirectNode allocation = node(allocate(1));
    DirectNode initialization = node(init(1), var(1));
    connect(entry, allocation, initialization);
    connect(entry, initialization);
    assertUnchanged(allocation);
  }

  @Test
  public void everyNormalExitMustInitialize() {
    DirectNode allocation = node(allocate(1));
    connect(allocation, node(init(1), var(1)));
    connect(allocation, node());
    assertUnchanged(allocation);
  }

  @Test
  public void cannotMoveAllocationIntoAnotherExceptionRegion() {
    DirectNode allocation = node(allocate(1));
    DirectNode initialization = node(init(1), var(1));
    connect(allocation, initialization);
    initialization.addSuccessor(DirectEdge.exception(initialization, node()));
    assertUnchanged(allocation);
  }

  @Test
  public void sharedExceptionRegionAllowsReconstructionAndKeepsHandlerReads() {
    DirectNode allocation = node(allocate(1));
    DirectNode initialization = node(init(1));
    DirectNode later = node(new ConstExprent(1, false, null));
    DirectNode handler = node(var(1));
    connect(allocation, initialization, later);
    for (DirectNode protectedNode : List.of(allocation, initialization, later)) {
      protectedNode.addSuccessor(DirectEdge.exception(protectedNode, handler));
    }
    assertTrue(ConstructorNormalizer.normalize(graph));
    AssignmentExprent assignment = assertInstanceOf(AssignmentExprent.class, initialization.exprents.get(0));
    assertInstanceOf(NewExprent.class, assignment.getRight());
  }

  @Test
  public void alternativePathsCannotGiveAnAliasDifferentValues() {
    DirectNode allocation = node(allocate(1));
    DirectNode alias = node(new AssignmentExprent(var(2), var(1), null));
    DirectNode unrelated = node(new AssignmentExprent(var(2), new ConstExprent(VarType.VARTYPE_NULL, null, null), null));
    DirectNode initialization = node(init(1), var(2));
    connect(allocation, alias, initialization);
    connect(allocation, unrelated, initialization);
    assertUnchanged(allocation);
  }

  @Test
  public void sameSlotAndClassDoNotIdentifyTwoAllocations() {
    DirectNode entry = node();
    DirectNode first = node(allocate(1));
    DirectNode second = node(allocate(1));
    DirectNode initialization = node(init(1), var(1));
    connect(entry, first, initialization);
    connect(entry, second, initialization);
    assertUnchanged(first);
    assertInstanceOf(NewExprent.class, ((AssignmentExprent)second.exprents.get(0)).getRight());
  }

  @Test
  public void overwrittenAliasesAreNotRestored() {
    DirectNode allocation = node(allocate(1), new AssignmentExprent(var(2), var(1), null),
      new AssignmentExprent(var(1), new ConstExprent(VarType.VARTYPE_NULL, null, null), null));
    DirectNode initialization = node(init(2), var(1), var(2));
    connect(allocation, initialization);
    assertTrue(ConstructorNormalizer.normalize(graph));
    assertEquals(1, allocation.exprents.size());
    assertEquals(3, initialization.exprents.size());
    AssignmentExprent assignment = assertInstanceOf(AssignmentExprent.class, initialization.exprents.get(0));
    assertEquals(2, ((VarExprent)assignment.getLeft()).getIndex());
  }

  @Test
  public void constructorMustBelongToTheAllocatedClass() {
    InvocationExprent wrong = init(1);
    wrong.setClassname("java/lang/String");
    DirectNode allocation = node(allocate(1), wrong, var(1));
    assertUnchanged(allocation);
  }

  @Test
  public void loopCannotExecuteAllocationAgainBeforeInitialization() {
    DirectNode allocation = node(allocate(1));
    connect(allocation, allocation);
    connect(allocation, node(init(1), var(1)));
    assertUnchanged(allocation);
  }

  @Test
  public void doesNotRemoveAliasesFromFixedSizeLoopHeaders() {
    DirectNode allocation = node(allocate(1));
    DirectNode header = DirectNode.forStat(DirectNodeType.INIT, new BasicBlockStatement(new BasicBlock(10)), null);
    Exprent copy = new AssignmentExprent(var(2), var(1), null);
    header.exprents = Arrays.asList(copy);
    graph.nodes.addWithKey(header, header.id);
    connect(allocation, header, node(init(2), var(2)));
    assertUnchanged(allocation);
    assertSame(copy, header.exprents.get(0));
  }

  @Test
  public void monitorScopeSurvivesRemovalOfMonitorExitExpressions() {
    DirectNode allocation = node(allocate(1));
    allocation.statement.setParent(new SynchronizedStatement());
    connect(allocation, node(init(1), var(1)));
    assertUnchanged(allocation);
  }

  private void assertUnchanged(DirectNode allocation) {
    Exprent original = allocation.exprents.get(0);
    assertFalse(ConstructorNormalizer.normalize(graph));
    assertSame(original, allocation.exprents.get(0));
    assertNull(((NewExprent)((AssignmentExprent)original).getRight()).getConstructor());
  }

  private DirectNode node(Exprent... expressions) {
    BasicBlockStatement statement = new BasicBlockStatement(new BasicBlock(graph.nodes.size()));
    DirectNode node = DirectNode.forStat(DirectNodeType.DIRECT, statement, null);
    node.exprents = new ArrayList<>(Arrays.asList(expressions));
    graph.nodes.addWithKey(node, node.id);
    if (graph.first == null) {
      graph.first = node;
    }
    return node;
  }

  private static void connect(DirectNode... nodes) {
    for (int i = 1; i < nodes.length; i++) {
      nodes[i - 1].addSuccessor(DirectEdge.of(nodes[i - 1], nodes[i]));
    }
  }

  private static VarExprent var(int index) {
    return new VarExprent(index, VarType.VARTYPE_OBJECT, null);
  }

  private static AssignmentExprent allocate(int index) {
    return new AssignmentExprent(var(index), new NewExprent(VarType.VARTYPE_OBJECT, List.of(), null), null);
  }

  private static InvocationExprent init(int index) {
    InvocationExprent invocation = new InvocationExprent();
    invocation.setClassname("java/lang/Object");
    invocation.setName("<init>");
    invocation.setFunctype(InvocationExprent.Type.INIT);
    invocation.setInstance(var(index));
    invocation.setDescriptor(MethodDescriptor.parseDescriptor("()V"));
    invocation.setStringDescriptor("()V");
    return invocation;
  }
}
