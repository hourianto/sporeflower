package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.flow.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class VariableInterferenceTest {
  @BeforeEach
  void setUp() {
    MinimalFernflowerEnvironment.setup();
  }

  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  void keepsAnOldValueReadAfterAnotherDefinition() {
    VariableInterference flow = flow(node(assign(1, constant()), assign(2, variable(1)), variable(1), variable(2)));
    assertFalse(flow.canMerge(pair(1), pair(2)));
  }

  @Test
  void allowsDisjointLifetimesAndReadsBeforeTheAssignment() {
    VariableInterference flow = flow(node(assign(1, constant()), assign(2, variable(1)), variable(2)));
    assertTrue(flow.canMerge(pair(1), pair(2)));
    flow = flow(node(assign(1, constant()), variable(1), assign(2, constant()), variable(2), assign(1, constant()), variable(1)));
    assertTrue(flow.canMerge(pair(1), pair(2)), "A later overwrite kills the old value");
  }

  @Test
  void followsSuccessorsAndLoopBackedges() {
    DirectNode header = node(variable(1)), body = node(assign(2, constant()), variable(2));
    connect(header, body);
    connect(body, header);
    assertFalse(flow(header, body).canMerge(pair(1), pair(2)));
  }

  @Test
  void preservesHandlerReadsAcrossEveryNestedWrite() {
    DirectNode body = node(assign(1, constant()), assign(2, assign(1, constant())));
    DirectNode handler = node(variable(1));
    body.addSuccessor(DirectEdge.exception(body, handler));
    assertFalse(flow(body, handler).canMerge(pair(1), pair(2)));
  }

  @Test
  void conditionalDefinitionsDoNotKillTheBypassPath() {
    FunctionExprent conditional = function(FunctionExprent.FunctionType.BOOLEAN_AND, variable(0), assign(1, constant()));
    assertFalse(flow(node(assign(2, constant()), conditional, variable(1))).canMerge(pair(1), pair(2)));
    FunctionExprent ternary = function(FunctionExprent.FunctionType.TERNARY, variable(0), assign(1, constant()), constant());
    assertFalse(flow(node(assign(2, constant()), ternary, variable(1))).canMerge(pair(1), pair(2)));
  }

  @Test
  void mutuallyExclusiveLifetimesMayShareALocal() {
    DirectNode start = node(variable(0));
    DirectNode left = node(assign(1, constant()), variable(1));
    DirectNode right = node(assign(2, constant()), variable(2));
    connect(start, left);
    connect(start, right);
    assertTrue(flow(start, left, right).canMerge(pair(1), pair(2)));
  }

  @Test
  void incrementReadsThePreviousValue() {
    FunctionExprent increment = function(FunctionExprent.FunctionType.IPP, variable(1));
    assertFalse(flow(node(assign(2, constant()), increment)).canMerge(pair(1), pair(2)));
  }

  @Test
  void contractionPreservesConflictsOfBothVariables() {
    VariableInterference flow = flow(node(assign(1, constant()), assign(3, constant()), variable(1), variable(3),
      assign(2, constant()), variable(2)));
    assertTrue(flow.canMerge(pair(1), pair(2)));
    assertTrue(flow.canMerge(pair(2), pair(3)));
    flow.merge(pair(1), pair(2));
    assertFalse(flow.canMerge(pair(2), pair(3)));
    assertFalse(flow.canMerge(pair(3), pair(2)));
  }

  @Test
  void cleanupWritesInterfereWithValuesReadByTheNormalContinuation() {
    BasicBlockStatement owner = BasicBlockStatement.create();
    DirectNode entry = DirectNode.forStat(DirectNodeType.FINALLY, owner, null);
    DirectNode end = DirectNode.forStat(DirectNodeType.FINALLY_END, owner, null);
    DirectNode body = DirectNode.forStat(DirectNodeType.DIRECT, BasicBlockStatement.create(), entry);
    DirectNode cleanup = DirectNode.forStat(DirectNodeType.DIRECT, BasicBlockStatement.create(), end);
    cleanup.exprents.add(assign(2, constant()));
    DirectNode continuation = node(variable(1));
    connect(body, continuation);
    connect(entry, cleanup);
    connect(cleanup, end);
    DirectGraph graph = graph(body, continuation, entry, cleanup, end);
    graph.finallyEnds.put(entry, end);
    assertFalse(new VariableInterference(graph).canMerge(pair(1), pair(2)));
  }

  @Test
  void exceptionalCleanupDoesNotReachTheNormalReturn() {
    BasicBlockStatement owner = BasicBlockStatement.create();
    DirectNode entry = DirectNode.forStat(DirectNodeType.FINALLY, owner, null);
    DirectNode end = DirectNode.forStat(DirectNodeType.FINALLY_END, owner, null);
    DirectNode start = node(assign(1, constant()));
    DirectNode body = DirectNode.forStat(DirectNodeType.DIRECT, BasicBlockStatement.create(), entry);
    body.exprents.add(assign(2, constant()));
    DirectNode cleanup = DirectNode.forStat(DirectNodeType.DIRECT, BasicBlockStatement.create(), end);
    DirectNode continuation = node(variable(2));
    connect(start, body);
    connect(body, continuation);
    body.addSuccessor(DirectEdge.exception(body, entry));
    connect(entry, cleanup);
    connect(cleanup, end);
    DirectGraph graph = graph(start, body, continuation, entry, cleanup, end);
    graph.finallyEnds.put(entry, end);
    assertTrue(new VariableInterference(graph).canMerge(pair(1), pair(2)),
      "The return value is not live on the exception path, which rethrows");
  }

  @Test
  void catchBindingInsideCleanupIsAlsoAWrite() {
    BasicBlockStatement owner = BasicBlockStatement.create();
    DirectNode entry = DirectNode.forStat(DirectNodeType.FINALLY, owner, null);
    DirectNode end = DirectNode.forStat(DirectNodeType.FINALLY_END, owner, null);
    DirectNode body = DirectNode.forStat(DirectNodeType.DIRECT, BasicBlockStatement.create(), entry);
    BasicBlockStatement protectedBody = BasicBlockStatement.create(), handler = BasicBlockStatement.create();
    CatchStatement caught = new CatchStatement() { };
    caught.getStats().addWithKey(protectedBody, protectedBody.id);
    caught.getStats().addWithKey(handler, handler.id);
    handler.setParent(caught);
    caught.getVars().add(variable(2));
    DirectNode binding = DirectNode.forStat(DirectNodeType.CATCH, handler, end);
    DirectNode continuation = node(variable(1));
    connect(body, continuation);
    connect(entry, binding);
    connect(binding, end);
    DirectGraph graph = graph(body, continuation, entry, binding, end);
    graph.finallyEnds.put(entry, end);
    assertFalse(new VariableInterference(graph).canMerge(pair(1), pair(2)));
  }

  private static VariableInterference flow(DirectNode... nodes) {
    return new VariableInterference(graph(nodes));
  }

  private static DirectGraph graph(DirectNode... nodes) {
    DirectGraph graph = new DirectGraph();
    graph.first = nodes[0];
    for (DirectNode node : nodes) graph.nodes.addWithKey(node, node.id);
    return graph;
  }

  private static DirectNode node(Exprent... expressions) {
    DirectNode node = DirectNode.forStat(DirectNodeType.DIRECT, BasicBlockStatement.create(), null);
    node.exprents.addAll(List.of(expressions));
    return node;
  }

  private static void connect(DirectNode from, DirectNode to) {
    from.addSuccessor(DirectEdge.of(from, to));
  }

  private static VarVersionPair pair(int index) {
    return new VarVersionPair(index, 0);
  }

  private static VarExprent variable(int index) {
    return new VarExprent(index, VarType.VARTYPE_INT, null);
  }

  private static ConstExprent constant() {
    return new ConstExprent(7, false, null);
  }

  private static AssignmentExprent assign(int index, Exprent value) {
    return new AssignmentExprent(variable(index), value, null);
  }

  private static FunctionExprent function(FunctionExprent.FunctionType type, Exprent... operands) {
    return new FunctionExprent(type, List.of(operands), null);
  }
}
