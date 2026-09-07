package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.util.StronglyConnectedComponents;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class DominatorTreeTest {
  @Test
  void diamondAndBackEdgeHaveTheSameCommonDominator() {
    Map<Integer, List<Integer>> predecessors = Map.of(
      0, List.of(), 1, List.of(0, 3), 2, List.of(0), 3, List.of(1, 2), 4, List.of(3));
    DominatorTree<Integer> tree = new DominatorTree<>(List.of(0, 2, 1, 3, 4), Set.of(0), predecessors::get);
    assertTrue(tree.isDominator(4, 0));
    assertTrue(tree.isDominator(4, 3));
    assertFalse(tree.isDominator(3, 1));
    assertFalse(tree.isDominator(3, 2));
  }

  @Test
  void multipleRootsDoNotDominateTheirSharedContinuation() {
    Map<Integer, List<Integer>> predecessors = Map.of(
      0, List.of(), 1, List.of(), 2, List.of(0, 1), 3, List.of(2));
    DominatorTree<Integer> tree = new DominatorTree<>(List.of(0, 1, 2, 3), Set.of(0, 1), predecessors::get);
    assertFalse(tree.isDominator(3, 0));
    assertFalse(tree.isDominator(3, 1));
    assertTrue(tree.isDominator(3, 2));
  }

  @Test
  void componentsAreUniqueAndDependenciesComeFirst() {
    Map<Integer, List<Integer>> edges = Map.of(
      0, List.of(1, 2), 1, List.of(0, 3), 2, List.of(3), 3, List.of(3), 4, List.of());
    List<List<Integer>> components = StronglyConnectedComponents.find(List.of(0, 4), edges::get);
    assertEquals(List.of(List.of(3), List.of(2), List.of(1, 0), List.of(4)), components);
  }
}
