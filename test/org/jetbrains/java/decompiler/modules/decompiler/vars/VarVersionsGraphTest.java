package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class VarVersionsGraphTest {
  @Test
  void reversePostorderPreservesRootsBranchesCyclesAndSharedSuccessors() {
    var graph = new VarVersionsGraph();
    List<VarVersionNode> n = nodes(graph, 6);
    edge(n.get(0), n.get(1));
    edge(n.get(0), n.get(2));
    edge(n.get(1), n.get(3));
    edge(n.get(2), n.get(3));
    edge(n.get(3), n.get(0));
    edge(n.get(4), n.get(3));
    edge(n.get(4), n.get(5));
    assertEquals(List.of(n.get(0), n.get(2), n.get(1), n.get(3), n.get(4), n.get(5)),
      VarVersionsGraph.getReversedPostOrder(List.of(n.get(0), n.get(4))));
    assertEquals(List.of(), VarVersionsGraph.getReversedPostOrder(List.of()));
  }

  @Test
  void deepAndWideGraphsRemainIterativeAndVisitEveryNodeOnce() {
    var graph = new VarVersionsGraph();
    List<VarVersionNode> n = nodes(graph, 10_000);
    for (int i = 1; i < n.size(); i++) edge(n.get(i - 1), n.get(i));
    assertEquals(n, VarVersionsGraph.getReversedPostOrder(List.of(n.get(0))));
    for (int i = 2; i < n.size(); i++) edge(n.get(0), n.get(i));
    var order = VarVersionsGraph.getReversedPostOrder(List.of(n.get(0)));
    assertEquals(n.size(), order.size());
    assertEquals(new HashSet<>(n), new HashSet<>(order));
    assertEquals(n.get(0), order.get(0));
  }

  @Test
  void dominatorsRespectAlternativeRootsAndLoopBackedges() {
    var graph = new VarVersionsGraph();
    List<VarVersionNode> n = nodes(graph, 6);
    edge(n.get(0), n.get(2));
    edge(n.get(1), n.get(2));
    edge(n.get(2), n.get(3));
    edge(n.get(2), n.get(4));
    edge(n.get(3), n.get(5));
    edge(n.get(4), n.get(5));
    edge(n.get(5), n.get(2));
    graph.initDominators();
    assertTrue(graph.isDominatorSet(n.get(5), Set.of(n.get(2))));
    assertFalse(graph.isDominatorSet(n.get(5), Set.of(n.get(0))));
    assertFalse(graph.isDominatorSet(n.get(5), Set.of(n.get(1))));
    assertFalse(graph.isDominatorSet(n.get(5), Set.of(n.get(3))));
    assertTrue(graph.isDominatorSet(n.get(5), Set.of(n.get(3), n.get(4))));
  }

  private static List<VarVersionNode> nodes(VarVersionsGraph graph, int size) {
    List<VarVersionNode> nodes = new ArrayList<>();
    for (int i = 0; i < size; i++) nodes.add(graph.createNode(new VarVersionPair(0, i)));
    return nodes;
  }

  private static void edge(VarVersionNode from, VarVersionNode to) {
    from.successors.add(to);
    to.predecessors.add(from);
  }
}
