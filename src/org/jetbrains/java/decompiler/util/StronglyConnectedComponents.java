package org.jetbrains.java.decompiler.util;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Deque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Function;

/** Tarjan components in reverse topological order: successors precede their dependants. */
public final class StronglyConnectedComponents<N> {
  private final Function<N, ? extends Iterable<N>> successors;
  private final Map<N, Integer> indices = new HashMap<>();
  private final Map<N, Integer> lowLinks = new HashMap<>();
  private final Deque<N> stack = new ArrayDeque<>();
  private final Set<N> onStack = new HashSet<>();
  private final List<List<N>> components = new ArrayList<>();

  private StronglyConnectedComponents(Function<N, ? extends Iterable<N>> successors) {
    this.successors = successors;
  }

  public static <N> List<List<N>> find(Collection<N> roots, Function<N, ? extends Iterable<N>> successors) {
    StronglyConnectedComponents<N> search = new StronglyConnectedComponents<>(successors);
    for (N root : roots) {
      if (!search.indices.containsKey(root)) search.visit(root);
    }
    return search.components;
  }

  private void visit(N node) {
    int index = indices.size();
    indices.put(node, index);
    lowLinks.put(node, index);
    stack.push(node);
    onStack.add(node);
    for (N successor : successors.apply(node)) {
      if (!indices.containsKey(successor)) {
        visit(successor);
        lowLinks.put(node, Math.min(lowLinks.get(node), lowLinks.get(successor)));
      } else if (onStack.contains(successor)) {
        lowLinks.put(node, Math.min(lowLinks.get(node), indices.get(successor)));
      }
    }
    if (lowLinks.get(node) == index) {
      List<N> component = new ArrayList<>();
      N member;
      do {
        member = stack.pop();
        onStack.remove(member);
        component.add(member);
      } while (!member.equals(node));
      components.add(component);
    }
  }
}
