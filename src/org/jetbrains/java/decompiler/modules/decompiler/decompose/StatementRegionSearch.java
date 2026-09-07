package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement.EdgeDirection;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/**
 * Searches candidate regions in one unchanged statement graph. Discard after a collapse:
 * adjacency and exception ranges are cached, while membership marks belong to one search.
 */
final class StatementRegionSearch {
  private final Map<Statement, Node> nodes = new IdentityHashMap<>();
  private final List<Node> region = new ArrayList<>();
  private final List<Node> handlers = new ArrayList<>();
  private final ArrayDeque<Node> queue = new ArrayDeque<>();
  private long generation;

  List<Statement> findRegion(Statement headStatement, Statement postStatement, int parentSize) {
    generation++;
    region.clear();
    handlers.clear();
    queue.clear();
    Node head = node(headStatement);
    Node boundary = headStatement == postStatement ? null : node(postStatement);
    addHandler(head);

    // Preserve handler discovery order and reconsider earlier handlers when a newly
    // included range makes them eligible. A boundary handler is processed without
    // entering it, so it must not be retried indefinitely.
    for (int index = 0; index < handlers.size();) {
      Node handler = handlers.get(index++);
      if (handler.included == generation || handler.expanded == generation) continue;
      if (!region.isEmpty() && !canIncludeHandler(handler)) continue;
      handler.expanded = generation;
      include(handler, boundary);
      while (!queue.isEmpty()) {
        Node current = queue.removeFirst();
        region.add(current);
        for (Node successor : current.successors()) include(successor, boundary);
        for (Node exception : current.handlers()) addHandler(exception);
      }
      index = 0;
    }

    if (region.size() >= parentSize || region.size() == 1 && !hasSelfLoop(head)) return null;

    // Every handler left outside must protect the entire region. The handler list
    // already contains all outgoing exception targets, in discovery order.
    for (Node handler : handlers) {
      if (handler.included != generation && !handler.covers(region)) return null;
    }
    for (Node member : region) {
      if (member != head) {
        for (Node predecessor : member.predecessors()) {
          if (predecessor.included != generation) return null;
        }
      }
      if (member.statement.isMonitorEnter()) {
        List<StatEdge> exits = member.statement.getAllDirectSuccessorEdges();
        if (exits.size() != 1 || exits.get(0).getType() != StatEdge.TYPE_REGULAR ||
            node(exits.get(0).getDestination()).included != generation) return null;
      }
    }

    List<Statement> result = new ArrayList<>(region.size());
    for (Node member : region) result.add(member.statement);
    return result;
  }

  private void include(Node member, Node boundary) {
    if (member != boundary && member.included != generation) {
      member.included = generation;
      queue.addLast(member);
    }
  }

  private void addHandler(Node handler) {
    if (handler.discovered != generation) {
      handler.discovered = generation;
      handlers.add(handler);
    }
  }

  private boolean canIncludeHandler(Node handler) {
    Node[] protectedNodes = handler.protectedNodes();
    // Keep the original strict-subset rule, including the edge count when a
    // protected statement has multiple edges to the same handler.
    if (region.size() <= protectedNodes.length && region.size() != 1) return false;
    for (Node protectedNode : protectedNodes) {
      if (protectedNode.included != generation) return false;
    }
    return true;
  }

  private static boolean hasSelfLoop(Node head) {
    for (Node predecessor : head.predecessors()) {
      if (predecessor == head) return true;
    }
    return false;
  }

  private Node node(Statement statement) {
    return nodes.computeIfAbsent(statement, key -> new Node(key, nodes.size()));
  }

  private Node[] neighbours(Statement statement, int type, EdgeDirection direction) {
    List<Statement> neighbours = statement.getNeighbours(type, direction);
    Node[] result = new Node[neighbours.size()];
    for (int i = 0; i < result.length; i++) result[i] = node(neighbours.get(i));
    return result;
  }

  private final class Node {
    private final Statement statement;
    private final int index;
    private Node[] successors;
    private Node[] predecessors;
    private Node[] handlers;
    private Node[] protectedNodes;
    private BitSet protectedMembership;
    private long included;
    private long discovered;
    private long expanded;

    private Node(Statement statement, int index) {
      this.statement = statement;
      this.index = index;
    }

    private Node[] successors() {
      if (successors == null) successors = neighbours(statement, StatEdge.TYPE_REGULAR, EdgeDirection.FORWARD);
      return successors;
    }

    private Node[] predecessors() {
      if (predecessors == null) predecessors = neighbours(statement, StatEdge.TYPE_REGULAR, EdgeDirection.BACKWARD);
      return predecessors;
    }

    private Node[] handlers() {
      if (handlers == null) handlers = neighbours(statement, StatEdge.TYPE_EXCEPTION, EdgeDirection.FORWARD);
      return handlers;
    }

    private Node[] protectedNodes() {
      if (protectedNodes == null) protectedNodes = neighbours(statement, StatEdge.TYPE_EXCEPTION, EdgeDirection.BACKWARD);
      return protectedNodes;
    }

    private boolean covers(List<Node> region) {
      if (protectedMembership == null) {
        protectedMembership = new BitSet();
        for (Node member : protectedNodes()) protectedMembership.set(member.index);
      }
      for (Node member : region) {
        if (!protectedMembership.get(member.index)) return false;
      }
      return true;
    }
  }
}
