package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.modules.decompiler.exps.ExprUtil;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdgeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchAllStatement;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Data-flow dependencies of finally bodies skipped by direct continuation edges.
 * A continuation supplies the input of each cleanup in nesting order, then
 * receives the last cleanup's result. Exception inputs still use graph edges.
 */
final class FinallyFlow {
  private final Map<DirectEdge, List<DirectNode>> exits = new HashMap<>();
  private final Map<DirectNode, List<DirectEdge>> inputs = new HashMap<>();
  private final Map<DirectNode, Set<DirectNode>> dependents = new HashMap<>();
  private final Map<DirectNode, Set<Integer>> writes = new HashMap<>();

  FinallyFlow(DirectGraph graph) {
    if (graph.finallyEnds.isEmpty()) return;
    for (DirectNode end : graph.finallyEnds.values()) writes.put(end, new HashSet<>());
    List<Exprent> expressions = new ArrayList<>();
    for (DirectNode node : graph.nodes) {
      if (node.tryFinally == null) continue;
      Set<Integer> localWrites = null;
      for (DirectNode context = node.tryFinally; context != null; context = context.tryFinally) {
        if (context.type == DirectNodeType.FINALLY_END) {
          if (localWrites == null) localWrites = collectWrites(node, expressions);
          writes.get(context).addAll(localWrites);
        }
      }

      for (DirectEdge edge : node.getSuccessors(DirectEdgeType.REGULAR)) {
        for (DirectNode context = node.tryFinally; context != null && !contains(context, edge.getDestination()); context = context.tryFinally) {
          // FINALLY marks the protected body; FINALLY_END marks cleanup already
          // being executed. An abrupt exit from cleanup must not execute it again.
          if (context.type == DirectNodeType.FINALLY) {
            // The semaphore fallback clears its flag on regular exits and leaves
            // their cleanup copies in place. Only exceptional exits execute the
            // guarded handler, so its writes do not precede these continuations.
            if (context.statement instanceof CatchAllStatement statement && statement.getMonitor() != null) continue;
            DirectNode end = graph.finallyEnds.get(context);
            List<DirectNode> preceding = exits.computeIfAbsent(edge, ignored -> new ArrayList<>());
            inputs.computeIfAbsent(context, ignored -> new ArrayList<>()).add(edge);
            dependents.computeIfAbsent(node, ignored -> new HashSet<>()).add(context);
            // An outer cleanup receives the state after the inner cleanups,
            // not just the state at the original continuation's source.
            for (DirectNode inner : preceding) {
              dependents.computeIfAbsent(inner, ignored -> new HashSet<>()).add(context);
            }
            preceding.add(end);
            dependents.computeIfAbsent(end, ignored -> new HashSet<>()).add(edge.getDestination());
          }
        }
      }
    }
  }

  private static Set<Integer> collectWrites(DirectNode node, List<Exprent> expressions) {
    Set<Integer> result = new HashSet<>();
    for (Exprent expression : node.exprents) {
      if (expression == null) continue;
      expressions.clear();
      expression.getAllExprents(true, expressions);
      expressions.add(expression);
      for (Exprent child : expressions) {
        Exprent written = node.type == DirectNodeType.FOREACH_VARDEF ? child : ExprUtil.getWrittenLocal(child);
        if (written instanceof VarExprent var && !var.isStack()) result.add(var.getIndex());
      }
    }
    return result;
  }

  private static boolean contains(DirectNode context, DirectNode node) {
    for (DirectNode current = node; current != null; current = current.tryFinally) {
      if (current == context) return true;
    }
    return false;
  }

  List<DirectNode> exits(DirectEdge edge) {
    return exits.getOrDefault(edge, List.of());
  }

  List<DirectEdge> inputs(DirectNode entry) {
    return inputs.getOrDefault(entry, List.of());
  }

  Set<DirectNode> dependents(DirectNode node) {
    return dependents.getOrDefault(node, Set.of());
  }

  Set<Integer> writes(DirectNode end) {
    return writes.get(end);
  }
}
