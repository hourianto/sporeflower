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
    // Most components are single statements. Inspect edges directly and stop at
    // the first external successor instead of materializing their union.
    Set<Statement> members = lst.size() > 1 ? new HashSet<>(lst) : null;
    for (Statement statement : lst) {
      if (!statement.hasSuccessor(StatEdge.TYPE_REGULAR)) continue;
      for (StatEdge edge : statement.getSuccessorEdgeView(StatEdge.TYPE_REGULAR)) {
        if (members == null ? edge.getDestination() != statement : !members.contains(edge.getDestination())) return false;
      }
    }
    return true;
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
