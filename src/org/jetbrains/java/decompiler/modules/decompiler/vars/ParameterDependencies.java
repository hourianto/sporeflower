package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.ExpressionFlow;

import java.util.*;
import java.util.function.IntFunction;
import java.util.function.Predicate;

/**
 * Readability preference for optional reuse of named parameters. The current
 * locals still separate independent definitions; phi-connected writes already
 * share a binding. Follow assignment reads through these locals without
 * interpreting operators, inspecting callees, or rewriting expressions.
 *
 * A dependency supports keeping an update under its parameter's name, not a
 * claim about its units or meaning. Type, scope and interference checks remain
 * responsible for correctness. Missing dependencies simply offer no preference.
 */
final class ParameterDependencies {
  private final Set<VarVersionPair> parameters;
  private final Set<VarVersionPair> named = new HashSet<>();
  private final IntFunction<VarVersionPair> original;
  private final Map<VarVersionPair, VarVersionPair> namedOrigins = new HashMap<>();
  private final Map<VarVersionPair, Set<VarVersionPair>> dependants = new HashMap<>();
  private final Map<VarVersionPair, BitSet> inputs = new HashMap<>();

  ParameterDependencies(Collection<VarVersionPair> parameters, Predicate<VarVersionPair> hasName,
                        IntFunction<VarVersionPair> original) {
    this.parameters = Set.copyOf(parameters);
    this.original = original;
    for (VarVersionPair parameter : parameters) {
      if (!hasName.test(parameter)) continue;
      named.add(parameter);
      BitSet input = new BitSet();
      input.set(parameter.var);
      inputs.put(parameter, input);
      VarVersionPair origin = original.apply(parameter.var);
      if (origin != null) namedOrigins.put(origin, parameter);
    }
  }

  void collect(Exprent expression) {
    if (named.isEmpty()) return;
    if (expression instanceof VarExprent variable) {
      // Later SSA rounds can split fragments of an already established binding.
      VarVersionPair parameter = namedOrigins.get(original.apply(variable.getIndex()));
      if (parameter != null) link(parameter, variable.getVarVersionPair());
    } else if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable) {
      VarVersionPair target = variable.getVarVersionPair();
      // Signature bindings anchor the analysis; their writes cannot transfer
      // one parameter's identity into another parameter.
      if (parameters.contains(target)) return;
      new ExpressionFlow<Void>() {
        protected Void copy(Void state) { return null; }
        protected Void join(Void first, Void second) { return null; }
        protected Void read(VarExprent variable, Void state) {
          link(variable.getVarVersionPair(), target);
          return null;
        }
        protected Void write(VarExprent variable, Exprent expression, Void state) { return null; }
      }.visit(assignment.getRight(), null);
    }
  }

  private void link(VarVersionPair input, VarVersionPair output) {
    if (!input.equals(output)) dependants.computeIfAbsent(input, ignored -> new HashSet<>()).add(output);
  }

  void finish() {
    Deque<VarVersionPair> pending = new ArrayDeque<>(named);
    Set<VarVersionPair> queued = new HashSet<>(named);
    while (!pending.isEmpty()) {
      VarVersionPair source = pending.removeFirst();
      queued.remove(source);
      for (VarVersionPair target : dependants.getOrDefault(source, Set.of())) {
        BitSet received = inputs.computeIfAbsent(target, ignored -> new BitSet());
        int before = received.cardinality();
        received.or(inputs.get(source));
        if (received.cardinality() != before && queued.add(target)) pending.addLast(target);
      }
    }
    dependants.clear();
    namedOrigins.clear();
  }

  boolean isNamed(VarVersionPair parameter) { return named.contains(parameter); }

  boolean supports(VarVersionPair variable, VarVersionPair parameter) {
    BitSet input = inputs.get(variable);
    return isNamed(parameter) && input != null && input.get(parameter.var);
  }

  void merge(VarVersionPair from, VarVersionPair to) {
    // An optional merge must not let one related lifetime lend a parameter's
    // name to unrelated lifetimes. Keep only support shared by every member.
    BitSet source = inputs.remove(from), target = inputs.get(to);
    if (source == null || target == null) inputs.remove(to);
    else target.and(source);
  }
}
