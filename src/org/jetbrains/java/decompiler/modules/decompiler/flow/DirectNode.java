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

  private final List<DirectEdge>[] successors = edgeBuckets();
  private final List<DirectEdge>[] predecessors = edgeBuckets();
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
  public List<DirectEdge> getSuccessors(DirectEdgeType type) {
    return getEdges(type, true);
  }

  public boolean hasPredecessors(DirectEdgeType type) {
    List<DirectEdge> edges = peekEdges(type, false);
    return edges != null && !edges.isEmpty();
  }
  public List<DirectEdge> getPredecessors(DirectEdgeType type) {
    return getEdges(type, false);
  }

  @SuppressWarnings("unchecked")
  private static List<DirectEdge>[] edgeBuckets() {
    // The array is private edge storage and all writes go through getEdges(),
    // which only stores List<DirectEdge> instances. The unchecked cast is safe
    // despite Java erasing the generic list element type at runtime.
    return (List<DirectEdge>[])new List<?>[DirectEdgeType.TYPES.length];
  }

  private List<DirectEdge> peekEdges(DirectEdgeType type, boolean successors) {
    return (successors ? this.successors : this.predecessors)[type.ordinal()];
  }

  private List<DirectEdge> getEdges(DirectEdgeType type, boolean successors) {
    List<DirectEdge>[] edges = successors ? this.successors : this.predecessors;
    List<DirectEdge> result = edges[type.ordinal()];
    if (result != null) {
      return result;
    }

    result = new ArrayList<>(2);
    edges[type.ordinal()] = result;
    return result;
  }

  public void addSuccessor(DirectEdge edge) {
    ValidationHelper.validateTrue(edge.getSource() == this, "Source node mismatch");
    if (!getSuccessors(edge.getType()).contains(edge)) {
      getSuccessors(edge.getType()).add(edge);
    }

    if (!edge.getDestination().getPredecessors(edge.getType()).contains(edge)) {
      edge.getDestination().getPredecessors(edge.getType()).add(edge);
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
    return Objects.hash(type, id);
  }

  @Override
  public String toString() {
    return id;
  }
}
