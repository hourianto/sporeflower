package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionNode;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class FinallyFlowTest {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void cleanupDefinitionReachesAContinuationVisitedEarlier(boolean multipleSources) {
    MinimalFernflowerEnvironment.setup();
    for (SFormsConstructor ssa : List.of(new SSAConstructorSparseEx(), new SSAUConstructorSparseEx())) {
      ssa.factory = new FastSparseSetFactory<>(List.of());
      BasicBlockStatement owner = new BasicBlockStatement(new BasicBlock(10));
      DirectNode entry = DirectNode.forStat(DirectNodeType.FINALLY, owner, null);
      DirectNode end = DirectNode.forStat(DirectNodeType.FINALLY_END, owner, null);
      DirectNode start = node(0, entry);
      DirectNode body = node(1, entry);
      DirectNode continuation = node(2, null);
      DirectNode cleanup = node(3, end);
      VarExprent initial = variable(), written = variable(), read = variable();
      VarExprent cleanupRead = variable();
      body.exprents.add(new AssignmentExprent(initial, new ConstExprent(1, false, null), null));
      cleanup.exprents.add(cleanupRead);
      cleanup.exprents.add(new AssignmentExprent(written, new ConstExprent(2, false, null), null));
      continuation.exprents.add(read);
      body.addSuccessor(DirectEdge.of(body, continuation));
      start.addSuccessor(DirectEdge.of(start, body));
      // Flattening connects the handler to the pre-try state and exception
      // states, but a normal continuation implicitly runs cleanup too.
      start.addSuccessor(DirectEdge.of(start, entry));
      List<DirectNode> order = new ArrayList<>(List.of(start, body));
      if (multipleSources) {
        DirectNode other = node(4, entry);
        other.exprents.add(new AssignmentExprent(variable(), new ConstExprent(3, false, null), null));
        start.addSuccessor(DirectEdge.of(start, other));
        other.addSuccessor(DirectEdge.of(other, continuation));
        order.add(other);
      }
      entry.addSuccessor(DirectEdge.of(entry, cleanup));
      cleanup.addSuccessor(DirectEdge.of(cleanup, end));
      DirectGraph graph = new DirectGraph();
      graph.first = start;
      graph.finallyEnds.put(entry, end);
      // The summary becomes available after its consumer; iteration order must
      // not decide whether cleanup's assignment is considered live.
      order.addAll(List.of(continuation, entry, cleanup, end));
      for (DirectNode node : order) graph.nodes.addWithKey(node, node.id);
      Set<String> updated = new HashSet<>();
      ssa.ssaStatements(graph, updated, false, null, 1);
      assertTrue(updated.contains(continuation.id));
      ssa.ssaStatements(graph, updated, false, null, 2);
      if (ssa instanceof SSAUConstructorSparseEx ssau) {
        assertEquals(Set.of(ssau.getNode(written)), ssau.getNode(read).predecessors);
        assertTrue(reaches(ssau.getNode(cleanupRead), ssau.getNode(initial), new HashSet<>()));
      } else {
        assertEquals(written.getVersion(), read.getVersion());
        assertTrue(cleanupRead.getVersion() == initial.getVersion() ||
          ((SSAConstructorSparseEx)ssa).getPhiComponents().component(cleanupRead.getVarVersionPair()).contains(initial.getVarVersionPair()));
      }
    }
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void nestedCleanupReceivesTheInnerResultEvenWhenVisitedFirst(boolean useSSAU) {
    MinimalFernflowerEnvironment.setup();
    SFormsConstructor ssa = useSSAU ? new SSAUConstructorSparseEx() : new SSAConstructorSparseEx();
    ssa.factory = new FastSparseSetFactory<>(List.of());
    BasicBlockStatement outer = new BasicBlockStatement(new BasicBlock(10));
    BasicBlockStatement inner = new BasicBlockStatement(new BasicBlock(11));
    DirectNode outerEntry = DirectNode.forStat(DirectNodeType.FINALLY, outer, null);
    DirectNode outerEnd = DirectNode.forStat(DirectNodeType.FINALLY_END, outer, null);
    DirectNode innerEntry = DirectNode.forStat(DirectNodeType.FINALLY, inner, outerEntry);
    DirectNode innerEnd = DirectNode.forStat(DirectNodeType.FINALLY_END, inner, outerEntry);
    DirectNode body = node(0, innerEntry), innerBody = node(1, innerEnd), outerBody = node(2, outerEnd), continuation = node(3, null);
    VarExprent initial = variable(), innerWrite = variable(), outerRead = variable(), outerWrite = variable(), read = variable();
    body.exprents.add(new AssignmentExprent(initial, new ConstExprent(1, false, null), null));
    innerBody.exprents.add(new AssignmentExprent(innerWrite, new ConstExprent(2, false, null), null));
    outerBody.exprents.add(outerRead);
    outerBody.exprents.add(new AssignmentExprent(outerWrite, new ConstExprent(3, false, null), null));
    continuation.exprents.add(read);
    body.addSuccessor(DirectEdge.of(body, continuation));
    innerEntry.addSuccessor(DirectEdge.of(innerEntry, innerBody));
    innerBody.addSuccessor(DirectEdge.of(innerBody, innerEnd));
    outerEntry.addSuccessor(DirectEdge.of(outerEntry, outerBody));
    outerBody.addSuccessor(DirectEdge.of(outerBody, outerEnd));
    DirectGraph graph = new DirectGraph();
    graph.first = body;
    graph.finallyEnds.put(innerEntry, innerEnd);
    graph.finallyEnds.put(outerEntry, outerEnd);
    for (DirectNode node : List.of(body, continuation, outerEntry, outerBody, outerEnd, innerEntry, innerBody, innerEnd)) {
      graph.nodes.addWithKey(node, node.id);
    }
    Set<String> updated = new HashSet<>();
    int iteration = 0;
    do {
      assertTrue(++iteration < 10, "Nested cleanup must converge");
      ssa.ssaStatements(graph, updated, false, null, iteration);
    } while (!updated.isEmpty());
    if (ssa instanceof SSAUConstructorSparseEx ssau) {
      assertEquals(Set.of(ssau.getNode(innerWrite)), ssau.getNode(outerRead).predecessors);
      assertEquals(Set.of(ssau.getNode(outerWrite)), ssau.getNode(read).predecessors);
    } else {
      assertEquals(innerWrite.getVersion(), outerRead.getVersion());
      assertEquals(outerWrite.getVersion(), read.getVersion());
    }
  }

  private static boolean reaches(VarVersionNode read, VarVersionNode definition, Set<VarVersionNode> visited) {
    if (read == definition) return true;
    if (!visited.add(read)) return false;
    return read.predecessors.stream().anyMatch(predecessor -> reaches(predecessor, definition, visited));
  }

  private static VarExprent variable() {
    return new VarExprent(0, VarType.VARTYPE_INT, null);
  }

  private static DirectNode node(int id, DirectNode context) {
    return DirectNode.forStat(DirectNodeType.DIRECT, new BasicBlockStatement(new BasicBlock(id)), context);
  }
}
