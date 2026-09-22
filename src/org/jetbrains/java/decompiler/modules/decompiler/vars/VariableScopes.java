package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;

import java.util.*;
import java.util.function.BiPredicate;
import java.util.function.Consumer;
import java.util.function.IntFunction;

/**
 * Visits local declarations in Java scope order. A merge may only rename the
 * current declaration to an earlier visible local and remove its declaration.
 * That invariant lets the walk continue without rebuilding scopes after each
 * attempt: accepted sources never enter the scope, and existing targets survive.
 */
final class VariableScopes {
  private final IntFunction<VarVersionPair> original;
  private final BiPredicate<Exprent, VarVersionPair> tryMerge;
  private final Consumer<VarExprent> binding;

  VariableScopes(IntFunction<VarVersionPair> original, BiPredicate<Exprent, VarVersionPair> tryMerge) {
    this(original, tryMerge, variable -> {});
  }

  VariableScopes(IntFunction<VarVersionPair> original, BiPredicate<Exprent, VarVersionPair> tryMerge, Consumer<VarExprent> binding) {
    this.original = original;
    this.tryMerge = tryMerge;
    this.binding = binding;
  }

  void process(Statement root, Map<Integer, VarVersionPair> parameters) {
    Scope scope = new Scope();
    parameters.forEach(scope::add);
    visit(root, scope, new Scope());
  }

  private void visit(Statement statement, Scope parent, Scope exported) {
    Scope scope = new Scope(parent);
    // Merges remove standalone declarations from their owning lists.
    for (Exprent expression : new ArrayList<>(statement.getVarDefinitions())) {
      declare(expression, scope, exported);
    }

    boolean transparent = switch (statement.type) {
      case BASIC_BLOCK, ROOT -> true;
      // A labeled sequence prints braces. Its hoisted declarations above are
      // outside those braces, but declarations in its children stay inside.
      case SEQUENCE -> !statement.isLabeled();
      default -> false;
    };
    if (statement.getExprents() != null) {
      for (Exprent expression : new ArrayList<>(statement.getExprents())) {
        declare(expression, scope, transparent ? exported : null);
      }
      return;
    }

    if (statement instanceof CatchStatement caught) {
      // Resources are visible in the protected body, not in the catch clauses.
      // Each catch binding belongs only to its own handler, never to the try
      // body or another handler, even when bytecode reuses the same slot.
      Scope protectedScope = new Scope(scope);
      for (Exprent resource : caught.getResources()) bind(resource, protectedScope);
      visit(caught.getFirst(), protectedScope, new Scope());
      for (int i = 0; i < caught.getVars().size(); i++) {
        Scope handlerScope = new Scope(scope);
        bind(caught.getVars().get(i), handlerScope);
        visit(caught.getStats().get(i + 1), handlerScope, new Scope());
      }
      return;
    }
    if (statement instanceof CatchAllStatement caught) {
      visit(caught.getFirst(), scope, new Scope());
      Scope handlerScope = new Scope(scope);
      for (VarExprent binding : caught.getVars()) bind(binding, handlerScope);
      visit(caught.getHandler(), handlerScope, new Scope());
      return;
    }

    for (Exprent expression : statement.getStatExprents()) {
      if (statement instanceof DoStatement loop && loop.getLooptype() == DoStatement.Type.FOR_EACH
          && expression == loop.getInitExprent()) {
        bind(expression, scope);
      } else {
        declare(expression, scope, transparent ? exported : null);
      }
    }
    for (Statement child : statement.getStats()) {
      Scope declarations = new Scope();
      visit(child, scope, declarations);
      if (statement instanceof IfStatement || statement instanceof SwitchStatement || statement instanceof SynchronizedStatement) {
        // These heads execute in the enclosing scope; the bodies do not.
        if (child != statement.getFirst()) continue;
        exported.addAll(declarations);
      } else if (transparent) {
        exported.addAll(declarations);
      }
      scope.addAll(declarations);
    }
  }

  private void bind(Exprent expression, Scope scope) {
    VarExprent variable = declarationVariable(expression);
    if (variable == null) return;
    binding.accept(variable);
    VarVersionPair origin = original.apply(variable.getIndex());
    if (origin != null) scope.add(origin.var, variable.getVarVersionPair());
  }

  private void declare(Exprent expression, Scope scope, Scope exported) {
    VarExprent variable = declarationVariable(expression);
    if (variable == null || !variable.isDefinition()) return;
    VarVersionPair origin = original.apply(variable.getIndex());
    // Synthesized locals need not have an original bytecode slot.
    if (origin == null) return;
    VarVersionPair current = variable.getVarVersionPair();
    List<VarVersionPair> candidates = scope.variables.getOrDefault(origin.var, List.of());
    // Prefer fragments of the same SSA origin, then the nearest preceding
    // lifetime of this slot. An incompatible lifetime must not hide older ones.
    for (boolean exactOrigin : new boolean[]{true, false}) {
      for (int i = candidates.size() - 1; i >= 0; i--) {
        VarVersionPair existing = candidates.get(i);
        if (current.equals(existing) || origin.equals(original.apply(existing.var)) != exactOrigin) continue;
        if (tryMerge.test(expression, existing)) return;
      }
    }
    scope.add(origin.var, current);
    if (exported != null) exported.add(origin.var, current);
  }

  static VarExprent declarationVariable(Exprent expression) {
    return expression instanceof VarExprent variable ? variable
      : expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable ? variable : null;
  }

  private static final class Scope {
    private final Map<Integer, List<VarVersionPair>> variables = new HashMap<>();

    private Scope() { }

    private Scope(Scope parent) {
      variables.putAll(parent.variables);
    }

    private void add(int slot, VarVersionPair variable) {
      List<VarVersionPair> previous = variables.getOrDefault(slot, List.of());
      if (previous.contains(variable)) return;
      // Child scopes share untouched lists with their parents.
      List<VarVersionPair> extended = new ArrayList<>(previous);
      extended.add(variable);
      variables.put(slot, extended);
    }

    private void addAll(Scope other) {
      other.variables.forEach((slot, locals) -> locals.forEach(local -> add(slot, local)));
    }
  }
}
