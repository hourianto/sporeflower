package org.jetbrains.java.decompiler.modules.decompiler.flow;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.stats.DummyExitStatement;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class DirectNodeTest {
  @BeforeEach
  void setUp() { MinimalFernflowerEnvironment.setup(); }

  @AfterEach
  void tearDown() { DecompilerContext.setCurrentContext(null); }

  @Test
  void wideGraphsKeepBothDirectionsOrderedAndDeduplicateEachEdgeKind() {
    DirectNode head = node(), tail = node();
    List<DirectNode> middle = new ArrayList<>();
    for (int i = 0; i < 2_000; i++) {
      DirectNode next = node();
      middle.add(next);
      for (DirectEdgeType type : DirectEdgeType.TYPES) {
        head.addSuccessor(new DirectEdge(head, next, type));
        next.addSuccessor(new DirectEdge(next, tail, type));
      }
    }
    // Reversed duplicate insertion must not reorder either adjacency list.
    for (int i = middle.size() - 1; i >= 0; i--) {
      DirectNode next = middle.get(i);
      for (DirectEdgeType type : DirectEdgeType.TYPES) {
        head.addSuccessor(new DirectEdge(head, next, type));
        next.addSuccessor(new DirectEdge(next, tail, type));
      }
    }
    for (DirectEdgeType type : DirectEdgeType.TYPES) {
      assertEquals(middle, head.getSuccessors(type).stream().map(DirectEdge::getDestination).toList());
      assertEquals(middle, tail.getPredecessors(type).stream().map(DirectEdge::getSource).toList());
      for (DirectNode next : middle) {
        assertEquals(List.of(new DirectEdge(head, next, type)), next.getPredecessors(type));
        assertEquals(List.of(new DirectEdge(next, tail, type)), next.getSuccessors(type));
      }
    }
  }

  @Test
  void selfLoopsAndReadOnlyViewsPreserveTopology() {
    DirectNode node = node();
    List<DirectEdge> successors = node.getSuccessors(DirectEdgeType.REGULAR);
    DirectEdge edge = DirectEdge.of(node, node);
    node.addSuccessor(edge);
    node.addSuccessor(DirectEdge.of(node, node));
    assertEquals(List.of(edge), successors);
    assertEquals(List.of(edge), node.getPredecessors(DirectEdgeType.REGULAR));
    assertSame(edge, successors.get(0));
    assertThrows(UnsupportedOperationException.class, () -> successors.add(edge));
    assertThrows(UnsupportedOperationException.class, () -> successors.remove(edge));
    assertThrows(UnsupportedOperationException.class, () -> successors.set(0, edge));
    assertFalse(node.hasSuccessors(DirectEdgeType.EXCEPTION));
    assertFalse(node.hasPredecessors(DirectEdgeType.EXCEPTION));
  }

  private static DirectNode node() {
    return DirectNode.forStat(DirectNodeType.DIRECT, new DummyExitStatement(), null);
  }
}
