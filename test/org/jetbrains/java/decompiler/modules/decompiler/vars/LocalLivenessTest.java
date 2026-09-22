package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.flow.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class LocalLivenessTest {
  @BeforeEach void setUp() { MinimalFernflowerEnvironment.setup(); }
  @AfterEach void tearDown() { DecompilerContext.setCurrentContext(null); }

  @Test
  void aJumpPastAnOverwriteKeepsTheCallersValueLive() {
    DirectNode branch = node(variable(0)), overwrite = node(assign(1, constant())), use = node(variable(1));
    connect(branch, overwrite);
    connect(branch, use);
    connect(overwrite, use);
    assertEquals(Set.of(pair(0), pair(1)), LocalLiveness.incomingReads(graph(branch, overwrite, use), Set.of()));
  }

  @Test
  void excludedWritesCannotSupplyTheCallersRemainingReads() {
    AssignmentExprent moved = assign(1, variable(2));
    DirectNode entry = node(moved, variable(1));
    DirectGraph graph = graph(entry);
    assertEquals(Set.of(pair(2)), LocalLiveness.incomingReads(graph, Set.of()));
    assertEquals(Set.of(pair(1)), LocalLiveness.incomingReads(graph, Set.of(moved)));
    assertSame(moved, entry.exprents.get(0), "Analysis must not mutate the caller");
  }

  @Test
  void aDefiniteLaterOverwriteReleasesTheIncomingValue() {
    AssignmentExprent moved = assign(1, variable(2));
    assertEquals(Set.of(), LocalLiveness.incomingReads(graph(node(moved, assign(1, constant()), variable(1))), Set.of(moved)));
  }

  @Test
  void compoundAssignmentsReadBeforeTheirRightHandSideWrites() {
    AssignmentExprent compound = new AssignmentExprent(variable(1), assign(1, constant()), FunctionExprent.FunctionType.ADD, null);
    assertEquals(Set.of(pair(1)), LocalLiveness.incomingReads(graph(node(compound)), Set.of()));
  }

  @Test
  void handlerReadsSurviveAnOverwriteInTheProtectedBlock() {
    DirectNode body = node(assign(1, constant())), handler = node(variable(1));
    body.addSuccessor(DirectEdge.exception(body, handler));
    assertEquals(Set.of(pair(1)), LocalLiveness.incomingReads(graph(body, handler), Set.of()));
  }

  private static DirectGraph graph(DirectNode... nodes) {
    DirectGraph graph = new DirectGraph();
    graph.first = nodes[0];
    for (DirectNode node : nodes) graph.nodes.addWithKey(node, node.id);
    return graph;
  }

  private static DirectNode node(Exprent... expressions) {
    BasicBlockStatement block = BasicBlockStatement.create();
    DirectNode node = DirectNode.forStat(DirectNodeType.DIRECT, block, null);
    node.exprents = List.of(expressions);
    return node;
  }

  private static void connect(DirectNode from, DirectNode to) { from.addSuccessor(DirectEdge.of(from, to)); }
  private static VarVersionPair pair(int index) { return new VarVersionPair(index, 0); }
  private static VarExprent variable(int index) { return new VarExprent(index, VarType.VARTYPE_INT, null); }
  private static ConstExprent constant() { return new ConstExprent(VarType.VARTYPE_INT, 7, null); }
  private static AssignmentExprent assign(int index, Exprent value) { return new AssignmentExprent(variable(index), value, null); }
}
