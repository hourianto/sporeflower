package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Connected phi versions, independent of the policy used to render their representative. */
public final class PhiComponents {
  private final List<Set<VarVersionPair>> groups = new ArrayList<>();
  private final Map<VarVersionPair, Set<VarVersionPair>> components = new HashMap<>();
  private final Map<VarVersionPair, Integer> representatives = new HashMap<>();

  PhiComponents(Map<VarVersionPair, ? extends Iterable<Integer>> phis) {
    Map<VarVersionPair, Node> nodes = new HashMap<>();
    phis.forEach((output, inputs) -> {
      Node left = nodes.computeIfAbsent(output, ignored -> new Node());
      for (int version : inputs) {
        union(left, nodes.computeIfAbsent(new VarVersionPair(output.var, version), ignored -> new Node()));
      }
    });
    Map<Node, Set<VarVersionPair>> groupsByRoot = new HashMap<>();
    nodes.forEach((pair, node) -> groupsByRoot.computeIfAbsent(find(node), ignored -> new HashSet<>()).add(pair));
    for (Set<VarVersionPair> group : groupsByRoot.values()) {
      Set<VarVersionPair> members = Set.copyOf(group);
      groups.add(members);
      int minimum = members.stream().mapToInt(pair -> pair.version).min().orElseThrow();
      for (VarVersionPair pair : members) {
        components.put(pair, members);
        representatives.put(pair, minimum);
      }
    }
  }

  public List<Set<VarVersionPair>> groups() {
    return List.copyOf(groups);
  }

  public Set<VarVersionPair> component(VarVersionPair pair) {
    return components.getOrDefault(pair, Set.of());
  }

  public Map<VarVersionPair, Integer> representatives() {
    return Map.copyOf(representatives);
  }

  private static Node find(Node node) {
    if (node.parent != node) {
      node.parent = find(node.parent);
    }
    return node.parent;
  }

  private static void union(Node first, Node second) {
    first = find(first);
    second = find(second);
    if (first == second) return;
    if (first.rank < second.rank) {
      first.parent = second;
    } else {
      second.parent = first;
      if (first.rank == second.rank) first.rank++;
    }
  }

  private static final class Node {
    private Node parent = this;
    private int rank;
  }
}
