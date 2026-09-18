package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.IfExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdgeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;

/**
 * Reaching definitions at individual reads. Small boolean partitions retain the
 * relationship between a flag and assignments performed on the same path. They
 * are merged when the budget is exceeded; this loses precision, never paths.
 * Definitions have identity: equal-looking expressions at different program
 * points can have different reaching inputs.
 */
final class SemanticLocalFlow {
  static final class Definition {
    final Exprent value;
    final VarVersionPair incoming;
    Definition(Exprent value, VarVersionPair incoming) {
      this.value = value;
      this.incoming = incoming;
    }
  }
  private record State(Map<VarVersionPair, Set<Definition>> values, Map<VarVersionPair, Boolean> flags) {}
  private record Branches(List<State> yes, List<State> no) {}
  private static final int MAX_PARTITIONS = 32;
  private final Map<Exprent, Definition> writes = new IdentityHashMap<>();
  private final Map<VarVersionPair, Definition> incoming = new HashMap<>();
  private final Map<VarVersionPair, Definition> singleDefinitions = new HashMap<>();
  private final Map<Exprent, Set<Definition>> reads = new IdentityHashMap<>();
  private List<State> catchable;

  SemanticLocalFlow(RootStatement root, Set<VarVersionPair> parameters) {
    DirectGraph graph = FlattenStatementsHelper.build(root);
    collectSingleDefinitions(graph, parameters);
    Map<DirectNode, Set<VarVersionPair>> liveFlags = futureFlags(graph);
    Map<DirectNode, List<State>> inputs = new HashMap<>();
    Set<DirectNode> widened = new HashSet<>();
    Deque<DirectNode> pending = new ArrayDeque<>();
    Set<DirectNode> queued = new HashSet<>();
    inputs.put(graph.first, List.of(new State(Map.of(), Map.of())));
    pending.add(graph.first);
    queued.add(graph.first);
    while (!pending.isEmpty()) {
      DirectNode node = pending.removeFirst();
      queued.remove(node);
      List<State> states = inputs.get(node);
      catchable = node.hasSuccessors(DirectEdgeType.EXCEPTION) ? new ArrayList<>(states) : null;
      Branches branches = null;
      for (int i = 0; i < node.exprents.size(); i++) {
        Exprent expression = node.exprents.get(i);
        if (expression == null)
          continue;
        if (i == node.exprents.size() - 1 && graph.mapNegIfBranch.containsKey(node.id)) {
          branches = branch(expression instanceof IfExprent conditional ? conditional.getCondition() : expression, states);
        } else if (node.type == DirectNodeType.FOREACH_VARDEF && expression instanceof VarExprent variable) {
          states = write(variable, expression, states, null);
        } else {
          states = evaluate(expression, states);
        }
      }
      for (DirectEdgeType type : DirectEdgeType.TYPES) {
        for (DirectEdge edge : node.getSuccessors(type)) {
          DirectNode destination = edge.getDestination();
          List<State> output = type == DirectEdgeType.EXCEPTION        ? catchable
            : branches == null                                         ? states
            : destination.id.equals(graph.mapNegIfBranch.get(node.id)) ? branches.no()
                                                                       : branches.yes();
          if (output == null || output.isEmpty())
            continue;
          List<State> combined = new ArrayList<>(inputs.getOrDefault(destination, List.of()));
          for (State state : output) combined.add(retainFlags(state, liveFlags.get(destination)));
          List<State> merged = compact(combined, widened.contains(destination));
          if (merged.size() == 1 && merged.get(0).flags().isEmpty()
            && combined.stream().map(State::flags).distinct().count() > MAX_PARTITIONS)
            widened.add(destination);
          if (!merged.equals(inputs.get(destination))) {
            inputs.put(destination, merged);
            if (queued.add(destination))
              pending.addLast(destination);
          }
        }
      }
    }
    catchable = null;
  }

  /**
   * SSA-derived locals with one write need no per-path map entry. Their sole
   * definition dominates every valid read (incoming parameters are excluded).
   * Most expression temporaries fall into this category; retaining all of them
   * in every boolean partition would make large methods needlessly expensive.
   */
  private void collectSingleDefinitions(DirectGraph graph, Set<VarVersionPair> parameters) {
    Set<VarVersionPair> multiple = new HashSet<>(parameters);
    Set<Exprent> seen = java.util.Collections.newSetFromMap(new IdentityHashMap<>());
    for (DirectNode node : graph.nodes)
      for (Exprent expression : node.exprents) {
        if (expression == null)
          continue;
        walk(expression, child -> {
          if (!seen.add(child))
            return;
          Exprent target = child instanceof AssignmentExprent assignment         ? assignment.getLeft()
            : child instanceof FunctionExprent function && isIncrement(function) ? function.getLstOperands().get(0)
                                                                                 : null;
          if (target instanceof VarExprent variable) {
            Definition definition = writes.computeIfAbsent(child, ignored -> new Definition(child, null));
            if (singleDefinitions.putIfAbsent(variable.getVarVersionPair(), definition) != null)
              multiple.add(variable.getVarVersionPair());
          }
        });
      }
    multiple.forEach(singleDefinitions::remove);
  }

  /**
   * Forget a flag after its last possible use, before forming join partitions.
   * Retaining dead flags makes independent earlier branches multiply forever.
   * This is a conservative liveness approximation (writes do not kill it), and
   * includes exception successors, so a flag needed by a handler stays live.
   */
  private static Map<DirectNode, Set<VarVersionPair>> futureFlags(DirectGraph graph) {
    Map<DirectNode, Set<VarVersionPair>> live = new HashMap<>();
    for (DirectNode node : graph.nodes) {
      Set<VarVersionPair> variables = new HashSet<>();
      for (Exprent expression : node.exprents)
        if (expression != null)
          walk(expression, child -> {
            if (child instanceof VarExprent variable && variable.getExprType().equals(VarType.VARTYPE_BOOLEAN))
              variables.add(variable.getVarVersionPair());
          });
      live.put(node, variables);
    }
    Deque<DirectNode> pending = new ArrayDeque<>(graph.nodes);
    Set<DirectNode> queued = new HashSet<>(graph.nodes);
    while (!pending.isEmpty()) {
      DirectNode node = pending.removeFirst();
      queued.remove(node);
      for (DirectEdgeType type : DirectEdgeType.TYPES)
        for (DirectEdge edge : node.getPredecessors(type)) {
          DirectNode previous = edge.getSource();
          if (live.get(previous).addAll(live.get(node)) && queued.add(previous))
            pending.addLast(previous);
        }
    }
    return live;
  }

  private static State retainFlags(State state, Set<VarVersionPair> live) {
    if (live.containsAll(state.flags().keySet()))
      return state;
    Map<VarVersionPair, Boolean> flags = new HashMap<>(state.flags());
    flags.keySet().retainAll(live);
    return new State(state.values(), Map.copyOf(flags));
  }

  Set<Definition> sources(Exprent expression) {
    return reads.getOrDefault(expression, Set.of());
  }

  private Set<Definition> values(State state, VarExprent variable) {
    VarVersionPair key = variable.getVarVersionPair();
    return values(state, key);
  }

  private Set<Definition> values(State state, VarVersionPair key) {
    Definition single = singleDefinitions.get(key);
    if (single != null)
      return Set.of(single);
    Set<Definition> defined = state.values().get(key);
    return defined != null ? defined : Set.of(incoming.computeIfAbsent(key, ignored -> new Definition(null, key)));
  }

  private void observe(VarExprent variable, List<State> states) {
    Set<Definition> sources = reads.computeIfAbsent(variable, ignored -> new LinkedHashSet<>());
    for (State state : states) sources.addAll(values(state, variable));
  }

  private List<State> evaluate(Exprent expression, List<State> states) {
    if (states.isEmpty())
      return states;
    if (expression instanceof VarExprent variable) {
      observe(variable, states);
      return states;
    }
    if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable) {
      if (assignment.getCondType() != null)
        observe(variable, states);
      if (variable.getExprType().equals(VarType.VARTYPE_BOOLEAN) && assignment.getCondType() == null) {
        Branches result = branch(assignment.getRight(), states);
        List<State> outputs = new ArrayList<>(write(variable, assignment, result.yes(), true));
        outputs.addAll(write(variable, assignment, result.no(), false));
        return compact(outputs, false);
      }
      states = evaluate(assignment.getRight(), states);
      return write(variable, assignment, states, null);
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      if (function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
        Branches choices = branch(operands.get(0), states);
        List<State> result = new ArrayList<>(evaluate(operands.get(1), choices.yes()));
        result.addAll(evaluate(operands.get(2), choices.no()));
        return compact(result, false);
      }
      if (function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND
        || function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_OR) {
        Branches result = branch(expression, states);
        List<State> merged = new ArrayList<>(result.yes());
        merged.addAll(result.no());
        return compact(merged, false);
      }
      if (isIncrement(function) && operands.get(0) instanceof VarExprent variable) {
        observe(variable, states);
        return write(variable, expression, states, null);
      }
    }
    for (Exprent child : expression.getAllExprents()) states = evaluate(child, states);
    return states;
  }

  private List<State> write(VarExprent variable, Exprent expression, List<State> states, Boolean flag) {
    Definition definition = writes.computeIfAbsent(expression, ignored -> new Definition(expression, null));
    Set<Definition> source = Set.of(definition);
    // A simple assignment's LHS denotes the new definition. A compound LHS and
    // ++/-- operand denote the old value and were observed before this write.
    if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null)
      reads.computeIfAbsent(variable, ignored -> new LinkedHashSet<>()).add(definition);
    List<State> result = new ArrayList<>();
    for (State state : states) {
      Map<VarVersionPair, Set<Definition>> values = state.values();
      if (!singleDefinitions.containsKey(variable.getVarVersionPair())) {
        values = new HashMap<>(values);
        values.put(variable.getVarVersionPair(), source);
        values = Map.copyOf(values);
      }
      Map<VarVersionPair, Boolean> flags = new HashMap<>(state.flags());
      if (flag == null)
        flags.remove(variable.getVarVersionPair());
      else
        flags.put(variable.getVarVersionPair(), flag);
      result.add(new State(values, Map.copyOf(flags)));
    }
    // Any intervening operation can throw. Retain both sides of each write for
    // exceptional successors, including a write before a later flag assignment.
    if (catchable != null)
      catchable.addAll(result);
    return result;
  }

  private Branches branch(Exprent expression, List<State> states) {
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      if (function.getFuncType() == FunctionExprent.FunctionType.BOOL_NOT) {
        Branches result = branch(operands.get(0), states);
        return new Branches(result.no(), result.yes());
      }
      if (function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND) {
        Branches left = branch(operands.get(0), states), right = branch(operands.get(1), left.yes());
        List<State> no = new ArrayList<>(left.no());
        no.addAll(right.no());
        return new Branches(right.yes(), compact(no, false));
      }
      if (function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_OR) {
        Branches left = branch(operands.get(0), states), right = branch(operands.get(1), left.no());
        List<State> yes = new ArrayList<>(left.yes());
        yes.addAll(right.yes());
        return new Branches(compact(yes, false), right.no());
      }
      if (function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
        Branches condition = branch(operands.get(0), states);
        Branches left = branch(operands.get(1), condition.yes()), right = branch(operands.get(2), condition.no());
        List<State> yes = new ArrayList<>(left.yes()), no = new ArrayList<>(left.no());
        yes.addAll(right.yes());
        no.addAll(right.no());
        return new Branches(compact(yes, false), compact(no, false));
      }
    }
    states = evaluate(expression, states);
    List<State> yes = new ArrayList<>(), no = new ArrayList<>();
    for (State state : states) {
      Boolean truth = truth(expression, state);
      if (truth == null || truth)
        yes.add(refine(expression, state, true));
      if (truth == null || !truth)
        no.add(refine(expression, state, false));
    }
    return new Branches(yes, no);
  }

  private Boolean truth(Exprent expression, State state) {
    Long constant = literal(expression);
    if (constant != null && (constant == 0 || constant == 1))
      return constant == 1;
    if (expression instanceof VarExprent variable && variable.getExprType().equals(VarType.VARTYPE_BOOLEAN))
      return state.flags().get(variable.getVarVersionPair());
    if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable)
      return state.flags().get(variable.getVarVersionPair());
    if (expression instanceof FunctionExprent function
      && (function.getFuncType() == FunctionExprent.FunctionType.EQ || function.getFuncType() == FunctionExprent.FunctionType.NE)) {
      Boolean left = truth(function.getLstOperands().get(0), state), right = truth(function.getLstOperands().get(1), state);
      if (left != null && right != null)
        return left.equals(right) == (function.getFuncType() == FunctionExprent.FunctionType.EQ);
    }
    return null;
  }

  private State refine(Exprent expression, State state, boolean truth) {
    if (expression instanceof VarExprent variable && variable.getExprType().equals(VarType.VARTYPE_BOOLEAN)) {
      Map<VarVersionPair, Boolean> flags = new HashMap<>(state.flags());
      flags.put(variable.getVarVersionPair(), truth);
      return new State(state.values(), Map.copyOf(flags));
    }
    if (expression instanceof FunctionExprent function
      && (function.getFuncType() == FunctionExprent.FunctionType.EQ || function.getFuncType() == FunctionExprent.FunctionType.NE)) {
      Exprent left = function.getLstOperands().get(0), right = function.getLstOperands().get(1);
      Boolean known = truth(right, state);
      if (known == null) {
        known = truth(left, state);
        left = right;
      }
      if (known != null)
        return refine(left, state, known == (truth == (function.getFuncType() == FunctionExprent.FunctionType.EQ)));
    }
    return state;
  }

  private List<State> compact(List<State> states, boolean collapse) {
    if (states.isEmpty())
      return List.of();
    Set<Map<VarVersionPair, Boolean>> partitions = new LinkedHashSet<>();
    for (State state : states) partitions.add(state.flags());
    collapse |= partitions.size() > MAX_PARTITIONS;
    Map<Map<VarVersionPair, Boolean>, State> result = new LinkedHashMap<>();
    for (State state : states) {
      Map<VarVersionPair, Boolean> key = collapse ? Map.of() : state.flags();
      State previous = result.get(key);
      if (previous == null) {
        result.put(key, new State(state.values(), key));
        continue;
      }
      Set<VarVersionPair> variables = new HashSet<>(previous.values().keySet());
      variables.addAll(state.values().keySet());
      Map<VarVersionPair, Set<Definition>> merged = new HashMap<>();
      for (VarVersionPair variable : variables) {
        Set<Definition> alternatives = new LinkedHashSet<>(values(previous, variable));
        alternatives.addAll(values(state, variable));
        merged.put(variable, Set.copyOf(alternatives));
      }
      result.put(key, new State(Map.copyOf(merged), key));
    }
    return List.copyOf(result.values());
  }
}
