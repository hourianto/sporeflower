// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.util.collections.VBStyleCollection;

import java.util.List;
import java.util.Set;
import java.util.function.Function;

/** Immediate dominators over a reverse-postordered graph, including forests with multiple roots. */
final class DominatorTree<N> {
  private final VBStyleCollection<N, N> dominators = new VBStyleCollection<>();

  DominatorTree(List<? extends N> nodes, Set<? extends N> roots,
                Function<N, ? extends Iterable<? extends N>> predecessors) {
    for (N node : nodes) dominators.addWithKey(null, node);
    boolean changed;
    do {
      changed = false;
      for (N node : nodes) {
        N common = null;
        if (!roots.contains(node)) {
          for (N predecessor : predecessors.apply(node)) {
            if (dominators.getWithKey(predecessor) != null) {
              common = intersect(common, predecessor);
              if (common == null) break; // A join between different dominator trees.
            }
          }
        }
        N parent = common == null ? node : common;
        changed |= !parent.equals(dominators.putWithKey(parent, node));
      }
    } while (changed);
  }

  private N intersect(N first, N second) {
    if (first == null) return second;
    int left = dominators.getIndexByKey(first);
    int right = dominators.getIndexByKey(second);
    while (left != right) {
      if (left > right) {
        N parent = dominators.getWithKey(first);
        if (parent.equals(first)) return null;
        first = parent;
        left = dominators.getIndexByKey(first);
      } else {
        N parent = dominators.getWithKey(second);
        if (parent.equals(second)) return null;
        second = parent;
        right = dominators.getIndexByKey(second);
      }
    }
    return first;
  }

  boolean isDominator(N node, N ancestor) {
    while (!node.equals(ancestor)) {
      N parent = dominators.getWithKey(node);
      if (parent == null) throw new IllegalArgumentException("Node outside dominator graph: " + node);
      if (parent.equals(node)) return false;
      node = parent;
    }
    return true;
  }

  VBStyleCollection<N, N> orderedDominators() {
    return dominators;
  }
}
