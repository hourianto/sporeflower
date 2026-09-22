package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExprUtil;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.IfExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchAllStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;

import java.util.*;

/**
 * Write/live interference for the final local-coalescing pass. Original bytecode
 * slots are only naming hints: expression substitution can extend a value's
 * lifetime past a later write to its original slot.
 *
 * The graph is built once per merge pass. Coalescing only renames locals and
 * removes declarations, so contracting its interference edges remains safe.
 */
final class VariableInterference {
  private final Map<VarVersionPair, Integer> indices = new HashMap<>();
  private final List<BitSet> conflicts = new ArrayList<>();

  VariableInterference(RootStatement root) {
    this(FlattenStatementsHelper.build(root));
  }

  VariableInterference(DirectGraph graph) {
    Map<DirectNode, Set<DirectNode>> trueSuccessors = new HashMap<>();
    Map<DirectNode, Set<DirectNode>> falseSuccessors = new HashMap<>();
    Map<DirectNode, Set<DirectNode>> predecessors = new HashMap<>();
    Map<DirectNode, BitSet> inputs = new HashMap<>();
    for (DirectNode node : graph.nodes) {
      trueSuccessors.put(node, new HashSet<>());
      if (graph.mapNegIfBranch.containsKey(node.id)) falseSuccessors.put(node, new HashSet<>());
      predecessors.put(node, new HashSet<>());
      inputs.put(node, new BitSet());
    }
    FinallyFlow finallyFlow = new FinallyFlow(graph);
    Map<DirectNode, DirectNode> finallyEntries = new HashMap<>();
    graph.finallyEnds.forEach((entry, end) -> finallyEntries.put(end, entry));
    for (DirectNode node : graph.nodes) {
      for (DirectEdge edge : node.getSuccessors(DirectEdgeType.REGULAR)) {
        Map<DirectNode, Set<DirectNode>> successors = edge.getDestination().id.equals(graph.mapNegIfBranch.get(node.id))
          ? falseSuccessors : trueSuccessors;
        link(node, edge.getDestination(), successors, predecessors);
        // Cleanup reads must stay live at every normal exit as well as on
        // exceptional paths. Do not link the shared cleanup's end to normal
        // continuations: that would make their locals live on exception paths
        // which actually rethrow, often before those locals were initialized.
        for (DirectNode end : finallyFlow.exits(edge)) {
          link(node, finallyEntries.get(end), successors, predecessors);
        }
      }
      for (DirectEdge edge : node.getSuccessors(DirectEdgeType.EXCEPTION)) {
        predecessors.get(edge.getDestination()).add(node);
      }
    }

    Deque<DirectNode> pending = new ArrayDeque<>(graph.nodes);
    Set<DirectNode> queued = new HashSet<>(graph.nodes);
    while (!pending.isEmpty()) {
      DirectNode node = pending.removeLast();
      queued.remove(node);
      BitSet input = transfer(node, outgoing(trueSuccessors.get(node), inputs), outgoing(falseSuccessors.get(node), inputs),
        exceptional(node, inputs), false);
      if (!input.equals(inputs.get(node))) {
        inputs.put(node, input);
        for (DirectNode previous : predecessors.get(node)) {
          if (queued.add(previous)) pending.addLast(previous);
        }
      }
    }
    for (DirectNode node : graph.nodes) {
      transfer(node, outgoing(trueSuccessors.get(node), inputs), outgoing(falseSuccessors.get(node), inputs),
        exceptional(node, inputs), true);
    }
    recordFinallyConflicts(graph, finallyFlow, finallyEntries, inputs);
  }

  private void recordFinallyConflicts(DirectGraph graph, FinallyFlow finallyFlow,
                                      Map<DirectNode, DirectNode> finallyEntries, Map<DirectNode, BitSet> inputs) {
    if (graph.finallyEnds.isEmpty()) return;
    // Direct exits skip cleanup writes too. Check those writes against each
    // continuation separately, including the inputs of later outer cleanups.
    // Treating every cleanup write as possible is conservative across branches.
    Map<DirectNode, BitSet> cleanupWrites = new HashMap<>();
    for (DirectNode end : graph.finallyEnds.values()) cleanupWrites.put(end, new BitSet());
    for (DirectNode node : graph.nodes) {
      for (DirectNode context = node.tryFinally; context != null; context = context.tryFinally) {
        if (context.type != DirectNodeType.FINALLY_END) continue;
        VarExprent binding = catchBinding(node);
        if (binding != null) cleanupWrites.get(context).set(index(binding.getVarVersionPair()));
        for (Exprent expression : node.exprents) {
          if (expression == null) continue;
          for (Exprent child : expression.getAllExprents(true, true)) {
            VarExprent written = ExprUtil.getWrittenLocal(child);
            if (node.type == DirectNodeType.FOREACH_VARDEF && child instanceof VarExprent variable) written = variable;
            if (written != null) cleanupWrites.get(context).set(index(written.getVarVersionPair()));
          }
        }
      }
    }
    for (DirectNode node : graph.nodes) {
      for (DirectEdge edge : node.getSuccessors(DirectEdgeType.REGULAR)) {
        List<DirectNode> cleanups = finallyFlow.exits(edge);
        if (cleanups.isEmpty()) continue;
        BitSet live = (BitSet)inputs.get(edge.getDestination()).clone();
        for (int i = cleanups.size() - 1; i >= 0; i--) {
          DirectNode end = cleanups.get(i);
          BitSet writes = cleanupWrites.get(end);
          for (int written = writes.nextSetBit(0); written >= 0; written = writes.nextSetBit(written + 1)) {
            recordWrite(written, live);
          }
          live.or(inputs.get(finallyEntries.get(end)));
        }
      }
    }
  }

  private static void link(DirectNode from, DirectNode to, Map<DirectNode, Set<DirectNode>> successors,
                           Map<DirectNode, Set<DirectNode>> predecessors) {
    successors.get(from).add(to);
    predecessors.get(to).add(from);
  }

  private static BitSet outgoing(Set<DirectNode> successors, Map<DirectNode, BitSet> inputs) {
    if (successors == null) return null;
    BitSet live = new BitSet();
    for (DirectNode next : successors) live.or(inputs.get(next));
    return live;
  }

  private static BitSet exceptional(DirectNode node, Map<DirectNode, BitSet> inputs) {
    BitSet live = new BitSet();
    for (DirectEdge edge : node.getSuccessors(DirectEdgeType.EXCEPTION)) live.or(inputs.get(edge.getDestination()));
    return live;
  }

  private BitSet transfer(DirectNode node, BitSet live, BitSet whenFalse, BitSet exceptional, boolean record) {
    live.or(exceptional);
    if (whenFalse != null) whenFalse.or(exceptional);
    List<Exprent> expressions = node.exprents;
    for (int i = expressions.size() - 1; i >= 0; i--) {
      Exprent expression = expressions.get(i);
      if (i == expressions.size() - 1 && whenFalse != null) {
        live = visitCondition(expression, live, whenFalse, exceptional, record);
      } else if (node.type == DirectNodeType.FOREACH_VARDEF && expression instanceof VarExprent variable) {
        write(variable, live, exceptional, record);
      } else {
        live = visit(expression, live, exceptional, record);
      }
    }
    if (expressions.isEmpty() && whenFalse != null) live.or(whenFalse);
    VarExprent binding = catchBinding(node);
    if (binding != null) write(binding, live, exceptional, record);
    return live;
  }

  // Catch bindings are implicit writes, absent from the direct node's exprents.
  private static VarExprent catchBinding(DirectNode node) {
    if (node.type != DirectNodeType.CATCH) return null;
    Statement parent = node.statement.getParent();
    List<VarExprent> bindings = parent instanceof CatchStatement statement ? statement.getVars()
      : parent instanceof CatchAllStatement statement ? statement.getVars() : List.of();
    if (bindings.isEmpty()) return null;
    int handler = parent.getStats().indexOf(node.statement) - 1;
    return handler >= 0 && handler < bindings.size() ? bindings.get(handler) : null;
  }

  private BitSet visit(Exprent expression, BitSet live, BitSet exceptional, boolean record) {
    if (expression == null) return live;
    if (expression instanceof VarExprent variable) {
      // A standalone declaration does not assign a value or read an old one.
      if (!variable.isDefinition()) live.set(index(variable.getVarVersionPair()));
      return live;
    }
    if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable) {
      write(variable, live, exceptional, record);
      live = visit(assignment.getRight(), live, exceptional, record);
      if (assignment.getCondType() != null) live.set(index(variable.getVarVersionPair()));
      return live;
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      if (function.getFuncType().isPPMM() && operands.get(0) instanceof VarExprent variable) {
        write(variable, live, exceptional, record);
        live.set(index(variable.getVarVersionPair()));
        return live;
      }
      switch (function.getFuncType()) {
        case TERNARY, BOOLEAN_AND, BOOLEAN_OR, BOOL_NOT:
          return visitCondition(function, live, live, exceptional, record);
      }
    }
    List<Exprent> children = expression.getAllExprents();
    for (int i = children.size() - 1; i >= 0; i--) live = visit(children.get(i), live, exceptional, record);
    return live;
  }

  /**
   * Propagate each outcome's demand through the expression that selects it.
   * For {@code p && (next = read()) != null}, a use of next in the true body
   * must not make it live on the path that skips read(). The same rule applies
   * inside ternaries and loop headers, not just at statement boundaries.
   * The supplied continuations are shared by branches and must not be mutated.
   */
  private BitSet visitCondition(Exprent expression, BitSet whenTrue, BitSet whenFalse, BitSet exceptional, boolean record) {
    if (expression instanceof IfExprent conditional) {
      return visitCondition(conditional.getCondition(), whenTrue, whenFalse, exceptional, record);
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      switch (function.getFuncType()) {
        case BOOLEAN_AND: {
          BitSet right = visitCondition(operands.get(1), whenTrue, whenFalse, exceptional, record);
          return visitCondition(operands.get(0), right, whenFalse, exceptional, record);
        }
        case BOOLEAN_OR: {
          BitSet right = visitCondition(operands.get(1), whenTrue, whenFalse, exceptional, record);
          return visitCondition(operands.get(0), whenTrue, right, exceptional, record);
        }
        case BOOL_NOT:
          return visitCondition(operands.get(0), whenFalse, whenTrue, exceptional, record);
        case TERNARY: {
          BitSet left = visitCondition(operands.get(1), whenTrue, whenFalse, exceptional, record);
          BitSet right = visitCondition(operands.get(2), whenTrue, whenFalse, exceptional, record);
          return visitCondition(operands.get(0), left, right, exceptional, record);
        }
      }
    }
    if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null
        && assignment.getLeft() instanceof VarExprent variable) {
      BitSet positive = (BitSet)whenTrue.clone(), negative = (BitSet)whenFalse.clone();
      write(variable, positive, exceptional, record);
      write(variable, negative, exceptional, record);
      return visitCondition(assignment.getRight(), positive, negative, exceptional, record);
    }
    BitSet live = (BitSet)whenTrue.clone();
    live.or(whenFalse);
    return visit(expression, live, exceptional, record);
  }

  private void write(VarExprent variable, BitSet live, BitSet exceptional, boolean record) {
    int written = index(variable.getVarVersionPair());
    if (record) recordWrite(written, live);
    live.clear(written);
    // A handler can observe the value before any write in its protected node.
    // Preserve that demand between nested writes as well as between statements.
    live.or(exceptional);
  }

  private void recordWrite(int written, BitSet live) {
    for (int other = live.nextSetBit(0); other >= 0; other = live.nextSetBit(other + 1)) {
      if (other == written) continue;
      conflicts.get(written).set(other);
      conflicts.get(other).set(written);
    }
  }

  private int index(VarVersionPair variable) {
    return indices.computeIfAbsent(variable, ignored -> {
      conflicts.add(new BitSet());
      return conflicts.size() - 1;
    });
  }

  boolean canMerge(VarVersionPair from, VarVersionPair to) {
    Integer source = indices.get(from), target = indices.get(to);
    return source == null || target == null || !conflicts.get(source).get(target);
  }

  void merge(VarVersionPair from, VarVersionPair to) {
    int source = index(from), target = index(to);
    BitSet combined = conflicts.get(target);
    combined.or(conflicts.get(source));
    combined.clear(source);
    combined.clear(target);
    for (int other = combined.nextSetBit(0); other >= 0; other = combined.nextSetBit(other + 1)) {
      conflicts.get(other).clear(source);
      conflicts.get(other).set(target);
    }
    conflicts.get(source).clear();
    indices.remove(from);
  }
}
