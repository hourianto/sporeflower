// Copyright 2000-2018 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.GenericDominatorEngine;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.IGraph;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.IGraphNode;
import org.jetbrains.java.decompiler.struct.attr.StructLocalVariableTableAttribute.LocalVariable;
import org.jetbrains.java.decompiler.util.collections.ListStack;
import org.jetbrains.java.decompiler.util.collections.VBStyleCollection;

import java.util.*;

public class VarVersionsGraph {
  public final VBStyleCollection<VarVersionNode, VarVersionPair> nodes = new VBStyleCollection<>();

  private GenericDominatorEngine engine;

  public VarVersionNode createNode(VarVersionPair ver) {
    return this.createNode(ver, null);
  }

  public VarVersionNode createNode(VarVersionPair ver, LocalVariable lvt) {
    VarVersionNode node = new VarVersionNode(ver.var, ver.version, lvt);
    this.nodes.addWithKey(node, ver);
    return node;
  }

  public boolean isDominatorSet(VarVersionNode node, Set<VarVersionNode> domnodes) {
    if (domnodes.size() == 1) {
      return this.engine.isDominator(node, domnodes.iterator().next());
    } else {
      if (domnodes.contains(node)) {
        return true;
      }

      Set<VarVersionNode> seen = new HashSet<>();

      Deque<VarVersionNode> lstNodes = new ArrayDeque<>();
      lstNodes.add(node);

      while (!lstNodes.isEmpty()) {
        VarVersionNode nd = lstNodes.pollFirst();

        if (!seen.add(nd)) {
          continue;
        }

        if (nd.predecessors.isEmpty()) {
          return false;
        }

        for (VarVersionNode pred : nd.predecessors) {
          if (!seen.contains(pred) && !domnodes.contains(pred)) {
            lstNodes.addLast(pred);
          }
        }
      }
    }

    return true;
  }

  public void initDominators() {
    Set<VarVersionNode> roots = new HashSet<>();

    for (VarVersionNode node : this.nodes) {
      if (node.predecessors.isEmpty()) {
        roots.add(node);
      }
    }

    if (ValidationHelper.VALIDATE) {
      ValidationHelper.validateTrue(this.nodes.size() == rootReachability(roots).size(), "Cyclic roots detected");
    }

    this.engine = new GenericDominatorEngine(new IGraph() {
      @Override
      public List<? extends IGraphNode> getReversePostOrderList() {
        return getReversedPostOrder(roots);
      }

      @Override
      public Set<? extends IGraphNode> getRoots() {
        return roots;
      }
    });

    this.engine.initialize();
  }

  /**
   * Returns the set of nodes that are reachable by the given roots.
   */
  public static Set<VarVersionNode> rootReachability(Set<VarVersionNode> roots) {
    Set<VarVersionNode> visited = new HashSet<>();

    ListStack<VarVersionNode> stack = new ListStack<>(roots);

    while (!stack.isEmpty()) {
      VarVersionNode node = stack.pop();

      if (visited.add(node)) {
        stack.addAll(node.successors);
      }
    }

    return visited;
  }

  public boolean areVarsAnalogous(int varBase, int varCheck) {
    Deque<VarVersionNode> stack = new ArrayDeque<>();
    Set<VarVersionNode> visited = new HashSet<>();

    VarVersionNode start = this.nodes.getWithKey(new VarVersionPair(varBase, 1));
    stack.add(start);

    while (!stack.isEmpty()) {
      VarVersionNode node = stack.removeFirst();
      ValidationHelper.validateTrue(
        node.phantomParentNode == null && node.phantomNode == null,
        "`areVarsAnalogous` should not be called after ppmm or operator assignments resugaring");

      if (visited.contains(node)) {
        continue;
      }

      visited.add(node);
      VarVersionNode analog = this.nodes.getWithKey(new VarVersionPair(varCheck, node.version));

      if (analog == null) {
        return false;
      }

      if (node.successors.size() != analog.successors.size()) {
        return false;
      }

      // FIXME: better checking
      for (VarVersionNode dest : node.successors) {
        stack.add(dest);

        VarVersionNode sucAnalog = this.nodes.getWithKey(new VarVersionPair(varCheck, dest.version));

        if (sucAnalog == null) {
          return false;
        }
      }
    }

    return true;
  }

  static List<VarVersionNode> getReversedPostOrder(Collection<VarVersionNode> roots) {
    List<VarVersionNode> order = new ArrayList<>();
    Set<VarVersionNode> visited = new HashSet<>();
    Deque<TraversalFrame> stack = new ArrayDeque<>();
    for (VarVersionNode root : roots) {
      if (!visited.add(root)) continue;
      int start = order.size();
      stack.push(new TraversalFrame(root));
      while (!stack.isEmpty()) {
        TraversalFrame frame = stack.peek();
        if (frame.successors.hasNext()) {
          VarVersionNode successor = frame.successors.next();
          if (visited.add(successor)) stack.push(new TraversalFrame(successor));
        } else {
          order.add(frame.node);
          stack.pop();
        }
      }
      // Preserve both successor order and the old ordering between roots. Appending
      // postorder then reversing this component avoids quadratic front insertions.
      Collections.reverse(order.subList(start, order.size()));
    }
    return order;
  }

  private record TraversalFrame(VarVersionNode node, Iterator<VarVersionNode> successors) {
    private TraversalFrame(VarVersionNode node) {
      this(node, node.successors.iterator());
    }
  }
}
