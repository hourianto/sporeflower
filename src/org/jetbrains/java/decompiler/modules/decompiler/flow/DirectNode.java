// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.flow;

import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;

import java.util.*;


public class DirectNode {
  public final DirectNodeType type;

  public final String id;

  public BasicBlockStatement block;

  public final Statement statement;

  public List<Exprent> exprents = new ArrayList<>();

  private final EdgeList[] successors = new EdgeList[DirectEdgeType.TYPES.length];
  private final EdgeList[] predecessors = new EdgeList[DirectEdgeType.TYPES.length];
  public final DirectNode tryFinally;

  private DirectNode(DirectNodeType type, Statement statement, DirectNode tryFinally) {
    this.type = type;
    this.statement = statement;
    this.tryFinally = tryFinally;
    this.id = type.makeId(statement.id);
  }

  public static DirectNode forStat(
    DirectNodeType type,
    Statement statement,
    DirectNode tryFinally
  ) {
    return new DirectNode(type, statement, tryFinally);
  }

  public boolean hasSuccessors(DirectEdgeType type) {
    List<DirectEdge> edges = peekEdges(type, true);
    return edges != null && !edges.isEmpty();
  }

  /** Read-only edges in insertion order; topology changes go through addSuccessor. */
  public List<DirectEdge> getSuccessors(DirectEdgeType type) {
    return getEdges(type, true);
  }

  public boolean hasPredecessors(DirectEdgeType type) {
    List<DirectEdge> edges = peekEdges(type, false);
    return edges != null && !edges.isEmpty();
  }

  /** Read-only incoming edges, maintained together with the source's successors. */
  public List<DirectEdge> getPredecessors(DirectEdgeType type) {
    return getEdges(type, false);
  }

  private List<DirectEdge> peekEdges(DirectEdgeType type, boolean successors) {
    return (successors ? this.successors : this.predecessors)[type.ordinal()];
  }

  private EdgeList getEdges(DirectEdgeType type, boolean successors) {
    EdgeList[] edges = successors ? this.successors : this.predecessors;
    EdgeList result = edges[type.ordinal()];
    if (result != null) {
      return result;
    }

    result = new EdgeList();
    edges[type.ordinal()] = result;
    return result;
  }

  public void addSuccessor(DirectEdge edge) {
    ValidationHelper.validateTrue(edge.getSource() == this, "Source node mismatch");
    getEdges(edge.getType(), true).addIfAbsent(edge);
    edge.getDestination().getEdges(edge.getType(), false).addIfAbsent(edge);
  }

  /** Small degrees need only a list; large fan-in/out must not scan it for every insertion. */
  private static final class EdgeList extends AbstractList<DirectEdge> implements RandomAccess {
    private final List<DirectEdge> edges = new ArrayList<>(2);
    private Set<DirectEdge> membership;

    private void addIfAbsent(DirectEdge edge) {
      if (membership == null && edges.size() >= 8) {
        membership = new HashSet<>(edges);
      }
      if (membership == null ? edges.contains(edge) : !membership.add(edge)) return;
      edges.add(edge);
      modCount++;
    }

    @Override
    public DirectEdge get(int index) {
      return edges.get(index);
    }

    @Override
    public int size() {
      return edges.size();
    }

    @Override
    public boolean contains(Object edge) {
      return membership == null ? edges.contains(edge) : membership.contains(edge);
    }
  }

  @Override
  public boolean equals(Object o) {
    if (this == o) {
      return true;
    }

    if (o == null || getClass() != o.getClass()) {
      return false;
    }

    DirectNode that = (DirectNode) o;
    return type == that.type && id.equals(that.id);
  }

  @Override
  public int hashCode() {
    return 31 * (31 + type.hashCode()) + id.hashCode();
  }

  @Override
  public String toString() {
    return id;
  }
}
