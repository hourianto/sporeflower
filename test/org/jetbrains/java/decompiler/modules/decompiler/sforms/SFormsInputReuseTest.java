package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.jetbrains.java.decompiler.util.collections.SFormsFastMapDirect;
import org.junit.jupiter.api.Test;

import java.util.HashSet;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class SFormsInputReuseTest {
  @Test
  void unchangedNormalOutputStillPropagatesChangedExceptionInput() {
    MinimalFernflowerEnvironment.setup();
    CountingSSA ssa = new CountingSSA();
    ssa.factory = new FastSparseSetFactory<>(List.of());
    VarExprent initial = variable(), overwritten = variable(), backedge = variable(), caught = variable();
    DirectNode entry = node(0, assignment(initial));
    DirectNode body = node(1, assignment(overwritten));
    DirectNode handler = node(2, caught);
    DirectNode tail = node(3, assignment(backedge));
    entry.addSuccessor(DirectEdge.of(entry, body));
    body.addSuccessor(DirectEdge.of(body, tail));
    tail.addSuccessor(DirectEdge.of(tail, body));
    body.addSuccessor(DirectEdge.exception(body, handler));
    DirectGraph graph = new DirectGraph();
    graph.first = entry;
    for (DirectNode node : List.of(entry, body, handler, tail)) graph.nodes.addWithKey(node, node.id);

    var updated = new HashSet<String>();
    ssa.ssaStatements(graph, updated, false, null, 1);
    assertEquals(3, ssa.assignments);
    assertFalse(ssa.getPhi().get(caught.getVarVersionPair()).contains(backedge.getVersion()));

    ssa.ssaStatements(graph, updated, false, null, 2);
    // The body overwrites the slot, so its normal output did not change. Its
    // exception state did, and the handler must see the newly discovered input.
    assertEquals(4, ssa.assignments);
    assertTrue(ssa.getPhi().get(caught.getVarVersionPair()).contains(backedge.getVersion()));

    ssa.ssaStatements(graph, updated, false, null, 3);
    assertEquals(4, ssa.assignments, "Stable inputs should not repeat expression processing");
    ssa.ssaStatements(graph, updated, true, null, 4);
    assertEquals(7, ssa.assignments, "The live-variable traversal must still process every node");
  }

  private static VarExprent variable() {
    return new VarExprent(0, VarType.VARTYPE_INT, null);
  }

  private static Exprent assignment(VarExprent variable) {
    return new AssignmentExprent(variable, new ConstExprent(1, false, null), null);
  }

  private static DirectNode node(int id, Exprent exprent) {
    DirectNode node = DirectNode.forStat(DirectNodeType.DIRECT, new BasicBlockStatement(new BasicBlock(id)), null);
    node.exprents.add(exprent);
    return node;
  }

  private static class CountingSSA extends SSAConstructorSparseEx {
    int assignments;

    @Override
    protected void onAssignment(VarVersionPair pair, SFormsFastMapDirect map, boolean live) {
      super.onAssignment(pair, map, live);
      assignments++;
    }
  }
}
