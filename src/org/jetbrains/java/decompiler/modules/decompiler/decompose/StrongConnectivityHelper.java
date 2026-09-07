package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement.EdgeDirection;
import org.jetbrains.java.decompiler.util.StronglyConnectedComponents;

import java.util.*;

/** Statement-specific entry and exit selection around the shared SCC algorithm. */
public final class StrongConnectivityHelper {
  private final List<List<Statement>> components;

  public StrongConnectivityHelper(Statement stat) {
    List<Statement> roots = new ArrayList<>();
    roots.add(stat.getFirst());
    for (Statement child : stat.getStats()) {
      if (child.getPredecessorEdges(Statement.STATEDGE_DIRECT_ALL).isEmpty()) roots.add(child);
    }
    roots.addAll(stat.getStats());
    components = StronglyConnectedComponents.find(roots,
      node -> node.getNeighbours(StatEdge.TYPE_REGULAR, EdgeDirection.FORWARD));
  }

  // Returns true if the component has no outgoing edges that aren't accounted for by the component itself
  public static boolean isExitComponent(List<? extends Statement> lst) {
    Set<Statement> set = new HashSet<>();

    for (Statement stat : lst) {
      set.addAll(stat.getNeighbours(StatEdge.TYPE_REGULAR, EdgeDirection.FORWARD));
    }

    for (Statement stat : lst) {
      set.remove(stat);
    }

    return set.isEmpty();
  }

  public static List<Statement> getExitReps(List<? extends List<Statement>> lst) {
    List<Statement> res = new ArrayList<>();

    for (List<Statement> comp : lst) {
      if (isExitComponent(comp)) {
        res.add(comp.get(0));
      }
    }

    return res;
  }

  public List<List<Statement>> getComponents() {
    return this.components;
  }
}