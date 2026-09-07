// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;

public class GenericDominatorEngine {
  private final IGraph graph;
  private DominatorTree<IGraphNode> tree;

  public GenericDominatorEngine(IGraph graph) {
    this.graph = graph;
  }

  public void initialize() {
    ValidationHelper.assertTrue(tree == null, "Dominator Engine was already initialized");
    tree = new DominatorTree<>(graph.getReversePostOrderList(), graph.getRoots(), IGraphNode::getPredecessors);
  }

  public boolean isDominator(IGraphNode node, IGraphNode dom) {
    if (tree == null) throw new IllegalStateException("GenericDominatorEngine not initialized!");
    return tree.isDominator(node, dom);
  }
}
