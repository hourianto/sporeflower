package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.junit.jupiter.api.Test;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class PhiComponentsTest {
  @Test
  void transitiveGroupsFlattenEveryInputAndKeepSlotsSeparate() {
    Map<VarVersionPair, List<Integer>> phis = new LinkedHashMap<>();
    phis.put(pair(0, 9), List.of(8, 7));
    phis.put(pair(0, 5), List.of(7, 3));
    phis.put(pair(0, 2), List.of(3, 1));
    phis.put(pair(1, 9), List.of(8));
    PhiComponents components = new PhiComponents(phis);
    assertEquals(2, components.groups().size());
    for (int version : List.of(1, 2, 3, 5, 7, 8, 9)) {
      assertEquals(1, components.representatives().get(pair(0, version)));
      assertEquals(7, components.component(pair(0, version)).size());
    }
    assertEquals(8, components.representatives().get(pair(1, 9)));
    assertTrue(components.component(pair(2, 9)).isEmpty());
  }

  @Test
  void copyEquivalenceAcceptsAnchoredLoopsAndRejectsContaminatedAndClosedCycles() {
    SSAConstructorSparseEx ssa = new SSAConstructorSparseEx();
    VarVersionPair source = pair(0, 1);
    ssa.markDirectAssignment(pair(1, 1), source);
    phi(ssa, pair(1, 2), 1, 3);
    ssa.markDirectAssignment(pair(1, 3), pair(1, 2));
    // This loop has a source path, but also an unrelated incoming definition.
    ssa.markDirectAssignment(pair(2, 1), source);
    phi(ssa, pair(2, 2), 1, 3, 4);
    ssa.markDirectAssignment(pair(2, 3), pair(2, 2));
    // No path from this cycle to the source.
    ssa.markDirectAssignment(pair(3, 1), pair(3, 2));
    ssa.markDirectAssignment(pair(3, 2), pair(3, 1));
    assertEquals(Set.of(source, pair(1, 1), pair(1, 2), pair(1, 3), pair(2, 1)),
      ssa.getDirectCopyEquivalentVersions(source));
  }

  private static void phi(SSAConstructorSparseEx ssa, VarVersionPair output, Integer... inputs) {
    FastSparseSetFactory<Integer> factory = new FastSparseSetFactory<>(List.of(inputs));
    var set = factory.createEmptySet();
    for (int input : inputs) set.add(input);
    ssa.getPhi().put(output, set);
  }

  private static VarVersionPair pair(int slot, int version) {
    return new VarVersionPair(slot, version);
  }
}
