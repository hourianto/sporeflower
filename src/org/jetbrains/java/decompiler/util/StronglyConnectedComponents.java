package org.jetbrains.java.decompiler.util;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Deque;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;

/** Tarjan components in reverse topological order: successors precede their dependants. */
public final class StronglyConnectedComponents<N> {
  private final Function<N, ? extends Iterable<N>> successors;
  private final Map<N, Node<N>> nodes = new HashMap<>();
  private final Deque<Node<N>> stack = new ArrayDeque<>();
  private final List<List<N>> components = new ArrayList<>();

  private StronglyConnectedComponents(Function<N, ? extends Iterable<N>> successors) {
    this.successors = successors;
  }

  public static <N> List<List<N>> find(Collection<N> roots, Function<N, ? extends Iterable<N>> successors) {
    StronglyConnectedComponents<N> search = new StronglyConnectedComponents<>(successors);
    for (N root : roots) {
      if (!search.nodes.containsKey(root)) search.visit(root);
    }
    return search.components;
  }

  private Node<N> visit(N value) {
    Node<N> node = new Node<>(value, nodes.size());
    nodes.put(value, node);
    stack.push(node);
    for (N successor : successors.apply(value)) {
      Node<N> next = nodes.get(successor);
      if (next == null) {
        next = visit(successor);
        node.lowLink = Math.min(node.lowLink, next.lowLink);
      } else if (next.onStack) {
        node.lowLink = Math.min(node.lowLink, next.index);
      }
    }
    if (node.lowLink == node.index) {
      List<N> component = new ArrayList<>();
      Node<N> member;
      do {
        member = stack.pop();
        member.onStack = false;
        component.add(member.value);
      } while (member != node);
      components.add(component);
    }
    return node;
  }

  // Keep Tarjan's state together: each edge needs only one lookup, and popping
  // a completed component does not need another lookup for every member.
  private static final class Node<N> {
    final N value;
    final int index;
    int lowLink;
    boolean onStack = true;

    Node(N value, int index) {
      this.value = value;
      this.index = index;
      this.lowLink = index;
    }
  }
}
