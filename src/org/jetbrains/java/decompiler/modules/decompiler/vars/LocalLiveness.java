package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExprUtil;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchAllStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;

import java.util.*;
import java.util.function.BiConsumer;

/**
 * Local-value demand on the reconstructed control-flow graph. This is shared by
 * coalescing and code movement: an expression's lexical position is not proof
 * that its writes execute on paths through breaks, handlers or short circuits.
 * Exclusions model expressions moved out of the caller without changing its IR.
 */
public final class LocalLiveness {
  private final Map<VarVersionPair, Integer> indices = new HashMap<>();
  private final DirectGraph graph;
  private final Set<Exprent> excluded;
  private final Map<DirectNode, Set<DirectNode>> trueSuccessors = new HashMap<>();
  private final Map<DirectNode, Set<DirectNode>> falseSuccessors = new HashMap<>();
  private final Map<DirectNode, BitSet> inputs = new HashMap<>();
  private final FinallyFlow finallyFlow;
  private final Map<DirectNode, DirectNode> finallyEntries = new HashMap<>();
  private BiConsumer<Integer, BitSet> observer;

  public static Set<VarVersionPair> incomingReads(RootStatement root, Set<Exprent> excluded) {
    return incomingReads(FlattenStatementsHelper.build(root), excluded);
  }

  static Set<VarVersionPair> incomingReads(DirectGraph graph, Set<Exprent> excluded) {
    LocalLiveness flow = new LocalLiveness(graph, excluded);
    BitSet live = flow.inputs.get(flow.graph.first);
    Set<VarVersionPair> reads = new HashSet<>();
    flow.indices.forEach((variable, index) -> { if (live.get(index)) reads.add(variable); });
    return reads;
  }

  LocalLiveness(DirectGraph graph) {
    this(graph, Set.of());
  }

  private LocalLiveness(DirectGraph graph, Set<Exprent> excluded) {
    this.graph = graph;
    this.excluded = excluded;
    Map<DirectNode, Set<DirectNode>> predecessors = new HashMap<>();
    for (DirectNode node : graph.nodes) {
      trueSuccessors.put(node, new HashSet<>());
      if (graph.mapNegIfBranch.containsKey(node.id)) falseSuccessors.put(node, new HashSet<>());
      predecessors.put(node, new HashSet<>());
      inputs.put(node, new BitSet());
    }
    finallyFlow = new FinallyFlow(graph);
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
  }

  Map<VarVersionPair, Integer> indices() { return Map.copyOf(indices); }

  void observeWrites(BiConsumer<Integer, BitSet> observer) {
    this.observer = observer;
    try {
      for (DirectNode node : graph.nodes) {
        transfer(node, outgoing(trueSuccessors.get(node), inputs), outgoing(falseSuccessors.get(node), inputs),
          exceptional(node, inputs), true);
      }
      recordFinallyConflicts();
    } finally {
      this.observer = null;
    }
  }

  private void recordFinallyConflicts() {
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
    Reads flow = new Reads(exceptional, record);
    List<Exprent> expressions = node.exprents;
    for (int i = expressions.size() - 1; i >= 0; i--) {
      Exprent expression = expressions.get(i);
      if (i == expressions.size() - 1 && whenFalse != null) {
        live = flow.condition(expression, live, whenFalse);
      } else if (node.type == DirectNodeType.FOREACH_VARDEF && expression instanceof VarExprent variable) {
        write(variable, live, exceptional, record);
      } else {
        live = flow.visit(expression, live);
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

  private final class Reads extends ExpressionFlow<BitSet> {
    private final BitSet exceptional;
    private final boolean record;

    private Reads(BitSet exceptional, boolean record) {
      this.exceptional = exceptional;
      this.record = record;
    }

    protected BitSet copy(BitSet state) { return (BitSet)state.clone(); }
    protected BitSet join(BitSet first, BitSet second) {
      BitSet result = copy(first);
      result.or(second);
      return result;
    }
    protected boolean skip(Exprent expression) { return excluded.contains(expression); }
    protected BitSet read(VarExprent variable, BitSet state) {
      state.set(index(variable.getVarVersionPair()));
      return state;
    }
    protected BitSet write(VarExprent variable, Exprent expression, BitSet state) {
      LocalLiveness.this.write(variable, state, exceptional, record);
      return state;
    }
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
    observer.accept(written, live);
  }

  private int index(VarVersionPair variable) {
    return indices.computeIfAbsent(variable, ignored -> indices.size());
  }
}
