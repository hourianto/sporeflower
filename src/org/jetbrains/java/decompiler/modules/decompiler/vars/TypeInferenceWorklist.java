package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.SwitchHeadExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.Deque;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/** Dependencies of type constraints within an expression graph that stays fixed during inference. */
final class TypeInferenceWorklist {
  private final BitSet pending = new BitSet();
  private final BitSet invalid = new BitSet();
  private final BitSet upperBounds = new BitSet();
  private final CheckTypesResult[] constraints;
  private final Map<VarVersionPair, List<Node>> occurrences = new HashMap<>();
  private final Deque<Node> affected = new ArrayDeque<>();
  private int revision;
  // No pending work precedes this position; invalidation may move it backwards.
  private int nextIndex;

  TypeInferenceWorklist(List<Exprent> expressions) {
    constraints = new CheckTypesResult[expressions.size()];
    pending.set(0, expressions.size());
    invalid.set(0, expressions.size());
    Map<Exprent, Node> nodes = new IdentityHashMap<>();
    List<Node> discovered = new ArrayList<>();
    for (int i = 0; i < expressions.size(); i++) {
      node(expressions.get(i), nodes, discovered).positions.add(i);
    }
    for (int i = 0; i < discovered.size(); i++) {
      Node current = discovered.get(i);
      Exprent expression = current.expression;
      VarVersionPair key = expression instanceof VarExprent var ? var.getVarVersionPair()
        : expression instanceof ConstExprent ? new VarVersionPair(expression.id, -1) : null;
      if (key != null) occurrences.computeIfAbsent(key, unused -> new ArrayList<>()).add(current);
      List<Exprent> children = expression.getAllExprents();
      // Switch labels participate in selector bounds but are not ordinary expression children.
      if (expression instanceof SwitchHeadExprent head) {
        for (List<Exprent> labels : head.getCaseValues()) {
          for (Exprent label : labels) if (label != null) children.add(label);
        }
      }
      for (Exprent child : children) node(child, nodes, discovered).parents.add(current);
    }
  }

  private static Node node(Exprent expression, Map<Exprent, Node> nodes, List<Node> discovered) {
    return nodes.computeIfAbsent(expression, key -> {
      Node node = new Node(key);
      discovered.add(node);
      return node;
    });
  }

  int next() {
    int index = pending.nextSetBit(nextIndex);
    if (index >= 0) {
      pending.clear(index);
      nextIndex = index + 1;
    }
    return index;
  }

  boolean needsEvaluation(int index) {
    boolean changed = invalid.get(index);
    invalid.clear(index);
    return changed;
  }

  CheckTypesResult constraints(int index) {
    return constraints[index];
  }

  void cache(int index, CheckTypesResult result) {
    constraints[index] = result;
    upperBounds.set(index, result != null && !result.getUpperBounds().isEmpty());
  }

  void changed(VarVersionPair key) {
    List<Node> readers = occurrences.get(key);
    if (readers == null) return;
    revision++;
    affected.addAll(readers);
    while (!affected.isEmpty()) {
      Node node = affected.removeLast();
      if (node.revision == revision) continue;
      node.revision = revision;
      for (int position : node.positions) {
        invalid.set(position);
        pending.set(position);
        nextIndex = Math.min(nextIndex, position);
      }
      affected.addAll(node.parents);
    }
  }

  void restartUpperBounds() {
    // Unlike lower-bound joins, upper-bound updates across type families are order-sensitive.
    // Replay cached upper constraints in the original order on a lower-bound restart; only
    // readers of changed types need to rebuild constraints and apply their lower bounds.
    pending.or(upperBounds);
    // New work can precede the cursor. Ordinary pops never need to rescan the
    // consumed prefix, but a replay must retain the original constraint order.
    nextIndex = 0;
  }

  private static final class Node {
    final Exprent expression;
    final List<Integer> positions = new ArrayList<>(1);
    final List<Node> parents = new ArrayList<>(1);
    int revision;

    Node(Exprent expression) {
      this.expression = expression;
    }
  }
}
