// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.util.collections.VBStyleCollection;

import java.util.Set;

/** Statement graph adapter retaining all predecessor edge kinds. */
public class DominatorEngine {
  private final Statement statement;
  private DominatorTree<Integer> tree;

  public DominatorEngine(Statement statement) {
    this.statement = statement;
  }

  public void initialize() {
    tree = new DominatorTree<>(statement.getReversePostOrderList().stream().map(stat -> stat.id).toList(),
      Set.of(statement.getFirst().id),
      id -> statement.getStats().getWithKey(id).getAllPredecessorEdges().stream().map(edge -> edge.getSource().id).toList());
  }

  public VBStyleCollection<Integer, Integer> getOrderedIDoms() {
    return tree.orderedDominators();
  }

  public boolean isDominator(Integer node, Integer dom) {
    return tree.isDominator(node, dom);
  }
}
