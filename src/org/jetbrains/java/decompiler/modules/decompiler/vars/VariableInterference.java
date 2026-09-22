package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.flow.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;

import java.util.*;

/** Write/live conflicts, contracted as printable locals coalesce. */
final class VariableInterference {
  private final Map<VarVersionPair, Integer> indices = new HashMap<>();
  private final List<BitSet> conflicts = new ArrayList<>();

  VariableInterference(RootStatement root) {
    this(FlattenStatementsHelper.build(root));
  }

  VariableInterference(DirectGraph graph) {
    LocalLiveness liveness = new LocalLiveness(graph);
    indices.putAll(liveness.indices());
    for (int i = 0; i < indices.size(); i++) conflicts.add(new BitSet());
    liveness.observeWrites(this::recordWrite);
  }

  private void recordWrite(int written, BitSet live) {
    for (int other = live.nextSetBit(0); other >= 0; other = live.nextSetBit(other + 1)) {
      if (other == written) continue;
      conflicts.get(written).set(other);
      conflicts.get(other).set(written);
    }
  }

  private int index(VarVersionPair variable) {
    return indices.computeIfAbsent(variable, ignored -> {
      conflicts.add(new BitSet());
      return conflicts.size() - 1;
    });
  }

  boolean canMerge(VarVersionPair from, VarVersionPair to) {
    Integer source = indices.get(from), target = indices.get(to);
    return source == null || target == null || !conflicts.get(source).get(target);
  }

  void merge(VarVersionPair from, VarVersionPair to) {
    int source = index(from), target = index(to);
    BitSet combined = conflicts.get(target);
    combined.or(conflicts.get(source));
    combined.clear(source);
    combined.clear(target);
    for (int other = combined.nextSetBit(0); other >= 0; other = combined.nextSetBit(other + 1)) {
      conflicts.get(other).clear(source);
      conflicts.get(other).set(target);
    }
    conflicts.get(source).clear();
    indices.remove(from);
  }
}
