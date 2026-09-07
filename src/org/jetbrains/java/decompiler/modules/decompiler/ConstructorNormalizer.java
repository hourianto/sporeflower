package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.main.decompiler.CancelationManager;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.MonitorExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdgeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.SynchronizedStatement;
import org.jetbrains.java.decompiler.struct.gen.CodeType;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Combines a particular uninitialized allocation with its constructor calls, before SSA.
 * Variable slots carry aliases of that allocation; they are not themselves allocation identities.
 * Analysis is read-only until every normal path and every incoming edge has been checked.
 * Path-dependent aliases and transfers across exception/monitor boundaries are left unresolved.
 * As with ordinary constructor resugaring, Java combines allocation and initialization: this
 * does not model allocation failure or class-initialization timing before separately computed arguments.
 */
public final class ConstructorNormalizer {
  private ConstructorNormalizer() { }

  public static boolean normalize(RootStatement root) {
    return normalize(FlattenStatementsHelper.build(root));
  }

  static boolean normalize(DirectGraph graph) {
    boolean[] changed = {false};
    for (DirectNode node : graph.nodes) {
      for (int i = 0; i < node.exprents.size(); i++) {
        Exprent expression = node.exprents.get(i);
        if (expression != null) {
          Exprent replacement = normalizeNested(expression, changed);
          if (replacement != expression) {
            node.exprents.set(i, replacement);
            changed[0] = true;
          }
        }
      }
    }

    // Edits change expression lists, but not graph edges. Revisit the current
    // index after removal; later nodes are inspected using their updated lists.
    Liveness liveness = null;
    for (DirectNode node : graph.nodes) {
      int index = 0;
      while (index < node.exprents.size()) {
        List<VarExprent> targets = new ArrayList<>();
        Exprent value = assignmentValue(node.exprents.get(index), targets);
        if (node.type == DirectNodeType.DIRECT && !targets.isEmpty() && isAllocation(value)) {
          if (liveness == null) {
            liveness = new Liveness(graph);
          }
          Plan plan = new Plan(graph, liveness, new Location(node, index), (NewExprent)value, targets);
          if (plan.analyze()) {
            plan.apply();
            changed[0] = true;
            continue;
          }
        }
        index++;
      }
    }
    return changed[0];
  }

  private static boolean isAllocation(Exprent expression) {
    return expression instanceof NewExprent allocation && allocation.getNewType().type == CodeType.OBJECT &&
      allocation.getNewType().arrayDim == 0 && allocation.getConstructor() == null;
  }

  private static Exprent assignmentValue(Exprent expression, List<VarExprent> targets) {
    while (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null &&
           assignment.getLeft() instanceof VarExprent variable) {
      targets.add(variable);
      expression = assignment.getRight();
    }
    return expression;
  }

  private static Exprent receiver(InvocationExprent invocation) {
    Exprent receiver = invocation.getInstance();
    while (receiver instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.CAST) {
      receiver = function.getLstOperands().get(0);
    }
    return receiver;
  }

  private static Exprent normalizeNested(Exprent expression, boolean[] changed) {
    for (Exprent child : expression.getAllExprents()) {
      Exprent replacement = normalizeNested(child, changed);
      if (replacement != child) {
        expression.replaceExprent(child, replacement);
      }
    }
    if (expression instanceof InvocationExprent invocation && invocation.getFunctype() == InvocationExprent.Type.INIT) {
      Exprent receiver = receiver(invocation);
      List<VarExprent> targets = new ArrayList<>();
      Exprent value = assignmentValue(receiver, targets);
      Set<Integer> aliases = new HashSet<>();
      targets.forEach(target -> aliases.add(target.getIndex()));
      if (isAllocation(value) && ((NewExprent)value).getNewType().value.equals(invocation.getClassname()) &&
          invocation.getLstParameters().stream().noneMatch(parameter -> usesAlias(parameter, aliases))) {
        ((NewExprent)value).setConstructor(invocation);
        invocation.setInstance(null);
        changed[0] = true;
        // Keep the entire assignment chain: more than one alias can remain live.
        return receiver;
      }
    }
    return expression;
  }

  private record Location(DirectNode node, int index) {
    private Exprent expression() {
      return node.exprents.get(index);
    }
  }

  private record Initialization(Location location, Set<Integer> aliases) { }

  private static final class Plan {
    private final DirectGraph graph;
    private final Liveness liveness;
    private final Location origin;
    private final NewExprent allocation;
    private final Map<Integer, VarExprent> variables = new LinkedHashMap<>();
    private final Set<Location> copies = new HashSet<>();
    private final List<Initialization> initializations = new ArrayList<>();
    private final Map<DirectNode, Set<Integer>> inputs = new HashMap<>();
    private final Set<DirectNode> traversed = new HashSet<>();
    private final Set<DirectNode> throwing = new HashSet<>();
    private final Set<DirectNode> handlers = new HashSet<>();
    private final ArrayDeque<DirectNode> pending = new ArrayDeque<>();

    private Plan(DirectGraph graph, Liveness liveness, Location origin, NewExprent allocation, List<VarExprent> targets) {
      this.graph = graph;
      this.liveness = liveness;
      this.origin = origin;
      this.allocation = allocation;
      remember(targets);
      for (DirectEdge edge : origin.node.getSuccessors(DirectEdgeType.EXCEPTION)) {
        handlers.add(edge.getDestination());
      }
      inputs.put(origin.node, Set.copyOf(variables.keySet()));
      pending.add(origin.node);
    }

    private void remember(List<VarExprent> targets) {
      for (VarExprent target : targets) {
        variables.put(target.getIndex(), target);
      }
    }

    private boolean analyze() {
      while (!pending.isEmpty()) {
        CancelationManager.checkCanceled();
        DirectNode node = pending.removeFirst();
        if (!sameExceptionRegion(node)) {
          return false;
        }
        Set<Integer> aliases = new HashSet<>(inputs.get(node));
        int start = node == origin.node ? origin.index + 1 : 0;
        boolean completed = false;
        for (int i = start; i < node.exprents.size(); i++) {
          Location location = new Location(node, i);
          Exprent expression = location.expression();
          if (expression == null) {
            continue;
          }
          if (expression instanceof MonitorExprent) {
            return false;
          }
          if (expression instanceof InvocationExprent invocation && invocation.getFunctype() == InvocationExprent.Type.INIT &&
              receiver(invocation) instanceof VarExprent variable && aliases.contains(variable.getIndex())) {
            if (node.type != DirectNodeType.DIRECT || !allocation.getNewType().value.equals(invocation.getClassname()) ||
                invocation.getLstParameters().stream().anyMatch(parameter -> usesAlias(parameter, aliases))) {
              return false;
            }
            initializations.add(new Initialization(location, Set.copyOf(aliases)));
            completed = true;
            break;
          }

          List<VarExprent> targets = new ArrayList<>();
          Exprent value = assignmentValue(expression, targets);
          if (!targets.isEmpty() && value instanceof VarExprent variable && aliases.contains(variable.getIndex())) {
            if (node.type != DirectNodeType.DIRECT) {
              return false; // Loop/header lists have cardinality and scope rules of their own.
            }
            remember(targets);
            for (VarExprent target : targets) {
              aliases.add(target.getIndex());
            }
            copies.add(location);
          } else {
            // Only plain local stores kill an alias. Nested or conditional uses
            // need expression-level control flow and are deliberately left alone.
            if (usesAlias(value, aliases)) {
              return false;
            }
            for (VarExprent target : targets) {
              aliases.remove(target.getIndex());
            }
          }
          if (expression instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.THROW) {
            // An argument computation can fail explicitly, just as an invocation
            // can throw. That path never produces an initialized value.
            throwing.add(node);
            completed = true;
            break;
          }
        }
        if (completed) {
          continue;
        }
        if (aliases.isEmpty() || !node.hasSuccessors(DirectEdgeType.REGULAR)) {
          return false;
        }
        traversed.add(node);
        for (DirectEdge edge : node.getSuccessors(DirectEdgeType.REGULAR)) {
          DirectNode next = edge.getDestination();
          if (next == origin.node) {
            return false; // Another execution of this new instruction is a different object.
          }
          Set<Integer> previous = inputs.putIfAbsent(next, Set.copyOf(aliases));
          if (previous == null) {
            pending.add(next);
          } else if (!previous.equals(aliases)) {
            return false; // A path-dependent alias cannot be assigned unconditionally at init.
          }
        }
      }
      if (initializations.isEmpty()) {
        return false;
      }

      // The region must have a single entry after this allocation. In particular,
      // an init cannot also be reached from an alternative allocation or a handler.
      for (DirectNode node : inputs.keySet()) {
        if (node == origin.node) {
          continue;
        }
        if (node == graph.first || node.hasPredecessors(DirectEdgeType.EXCEPTION)) {
          return false;
        }
        for (DirectEdge edge : node.getPredecessors(DirectEdgeType.REGULAR)) {
          if (!traversed.contains(edge.getSource())) {
            return false;
          }
        }
      }

      // Loops may compute arguments, but a closed cycle with no completion is
      // not a completed allocation. Every normal exit must initialize; throwing
      // paths are abrupt completions and need no constructor call.
      Set<DirectNode> reachesCompletion = new HashSet<>(throwing);
      ArrayDeque<DirectNode> backwards = new ArrayDeque<>(throwing);
      for (Initialization initialization : initializations) {
        if (reachesCompletion.add(initialization.location.node)) {
          backwards.add(initialization.location.node);
        }
      }
      while (!backwards.isEmpty()) {
        for (DirectEdge edge : backwards.removeFirst().getPredecessors(DirectEdgeType.REGULAR)) {
          DirectNode previous = edge.getSource();
          if (traversed.contains(previous) && reachesCompletion.add(previous)) {
            backwards.add(previous);
          }
        }
      }
      return reachesCompletion.containsAll(inputs.keySet());
    }

    private boolean sameExceptionRegion(DirectNode node) {
      if (monitorScope(node.statement) != monitorScope(origin.node.statement) ||
          node.tryFinally != origin.node.tryFinally || node.type == DirectNodeType.TRY ||
          node.type == DirectNodeType.COMBINED_CATCH || node.type == DirectNodeType.CATCH ||
          node.type == DirectNodeType.FINALLY || node.type == DirectNodeType.FINALLY_END) {
        return false;
      }
      List<DirectEdge> edges = node.getSuccessors(DirectEdgeType.EXCEPTION);
      return edges.size() == handlers.size() && edges.stream().allMatch(edge -> handlers.contains(edge.getDestination()));
    }

    private void apply() {
      Map<Location, List<Exprent>> replacements = new HashMap<>();
      replacements.put(origin, List.of());
      for (Location copy : copies) {
        replacements.put(copy, List.of());
      }
      for (Initialization initialization : initializations) {
        InvocationExprent invocation = (InvocationExprent)initialization.location.expression();
        NewExprent constructed = (NewExprent)allocation.copy();
        constructed.setConstructor(invocation);
        invocation.setInstance(null);
        constructed.addBytecodeOffsets(invocation.bytecode);
        Set<Integer> live = liveness.after(initialization.location);
        List<VarExprent> aliases = variables.values().stream()
          .filter(variable -> initialization.aliases.contains(variable.getIndex()) && live.contains(variable.getIndex())).toList();
        if (aliases.isEmpty()) {
          replacements.put(initialization.location, List.of(constructed));
          continue;
        }
        VarExprent target = aliases.get(aliases.size() - 1);
        for (VarExprent alias : aliases) {
          if (!alias.isStack()) {
            target = alias;
          }
        }
        List<Exprent> expressions = new ArrayList<>();
        expressions.add(new AssignmentExprent(target.copy(), constructed, origin.expression().bytecode));
        for (VarExprent alias : aliases) {
          if (alias != target) {
            expressions.add(new AssignmentExprent(alias.copy(), target.copy(), null));
          }
        }
        replacements.put(initialization.location, expressions);
      }
      // Descending indices keep locations valid, including allocation and init
      // in the same block. Only actual statement lists support insertion/removal.
      for (DirectNode node : inputs.keySet()) {
        for (int i = node.exprents.size() - 1; i >= 0; i--) {
          List<Exprent> replacement = replacements.get(new Location(node, i));
          if (replacement != null) {
            node.exprents.remove(i);
            node.exprents.addAll(i, replacement);
          }
        }
      }
    }
  }

  private static Statement monitorScope(Statement statement) {
    for (Statement parent = statement.getParent(); parent != null; statement = parent, parent = parent.getParent()) {
      // The header evaluates before entering the monitor. The body remains
      // protected even after simplification removes explicit monitorexit exprents.
      if (parent instanceof SynchronizedStatement && parent.getFirst() != statement) {
        return parent;
      }
    }
    return null;
  }

  /** Conservative local liveness, including handler reads before any intervening store. */
  private static final class Liveness {
    private final Map<DirectNode, Set<Integer>> inputs = new HashMap<>();

    private Liveness(DirectGraph graph) {
      ArrayDeque<DirectNode> pending = new ArrayDeque<>(graph.nodes);
      Set<DirectNode> queued = new HashSet<>(graph.nodes);
      while (!pending.isEmpty()) {
        CancelationManager.checkCanceled();
        DirectNode node = pending.removeFirst();
        queued.remove(node);
        Set<Integer> live = beforeSuffix(node, 0);
        if (!live.equals(inputs.put(node, live))) {
          for (DirectEdgeType type : DirectEdgeType.TYPES) {
            for (DirectEdge edge : node.getPredecessors(type)) {
              if (queued.add(edge.getSource())) {
                pending.add(edge.getSource());
              }
            }
          }
        }
      }
    }

    private Set<Integer> after(Location location) {
      return beforeSuffix(location.node, location.index + 1);
    }

    private Set<Integer> beforeSuffix(DirectNode node, int start) {
      Set<Integer> live = successorInputs(node, DirectEdgeType.REGULAR);
      Set<Integer> exceptional = successorInputs(node, DirectEdgeType.EXCEPTION);
      for (int i = node.exprents.size() - 1; i >= start; i--) {
        List<VarExprent> targets = new ArrayList<>();
        Exprent value = assignmentValue(node.exprents.get(i), targets);
        for (VarExprent target : targets) {
          live.remove(target.getIndex());
        }
        if (value != null) {
          value.getAllVariables().forEach(pair -> live.add(pair.var));
        }
        live.addAll(exceptional);
      }
      return live;
    }

    private Set<Integer> successorInputs(DirectNode node, DirectEdgeType type) {
      Set<Integer> result = new HashSet<>();
      for (DirectEdge edge : node.getSuccessors(type)) {
        result.addAll(inputs.getOrDefault(edge.getDestination(), Set.of()));
      }
      return result;
    }
  }

  private static boolean usesAlias(Exprent expression, Set<Integer> aliases) {
    return expression != null && expression.getAllVariables().stream().anyMatch(pair -> aliases.contains(pair.var));
  }
}
