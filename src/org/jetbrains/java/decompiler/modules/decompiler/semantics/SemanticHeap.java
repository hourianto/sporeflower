package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ArrayExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticFacts.*;

/** Allocation identities and all possible stores through local array aliases. */
final class SemanticHeap {
  record Store(ArrayExprent access, Exprent value) {}
  private record Origins(Set<NewExprent> allocations, boolean external) {
    static final Origins EMPTY = new Origins(Set.of(), false);
    static final Origins EXTERNAL = new Origins(Set.of(), true);
    Origins merge(Origins other) {
      Set<NewExprent> result = Collections.newSetFromMap(new IdentityHashMap<>());
      result.addAll(allocations);
      result.addAll(other.allocations);
      return new Origins(result, external || other.external);
    }
  }
  private final Map<Exprent, Origins> origins = new IdentityHashMap<>();
  private final SemanticLocalFlow locals;
  private final SemanticAnalysis analysis;
  private final Map<NewExprent, List<ArrayExprent>> publications = new IdentityHashMap<>();
  private final Map<NewExprent, Set<NewExprent>> holders = new IdentityHashMap<>();
  private final Map<NewExprent, List<Store>> stores = new IdentityHashMap<>();
  private final Map<NewExprent, List<InvocationExprent>> containerUses = new IdentityHashMap<>();
  private final Map<NewExprent, Set<NewExprent>> contents = new IdentityHashMap<>();
  private final Set<NewExprent> publishedContents = Collections.newSetFromMap(new IdentityHashMap<>());
  private final Set<NewExprent> escaped = Collections.newSetFromMap(new IdentityHashMap<>());

  SemanticHeap(SemanticAnalysis analysis, List<Exprent> roots) {
    this.analysis = analysis;
    locals = analysis.locals;
    List<Exprent> expressions = new ArrayList<>();
    Set<Exprent> seen = Collections.newSetFromMap(new IdentityHashMap<>());
    for (Exprent root : roots)
      walk(root, expression -> {
        if (seen.add(expression))
          expressions.add(expression);
      });
    // Stores also participate in reference provenance: reading an Object[] can
    // recover an array alias even after its static type was widened to Object.
    List<AssignmentExprent> arrayWrites =
      expressions.stream()
        .filter(expression -> expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof ArrayExprent)
        .map(expression -> (AssignmentExprent) expression)
        .toList();
    boolean changed;
    do {
      changed = false;
      for (Exprent expression : expressions) {
        if (expression.getExprType().arrayDim == 0 && expression.getExprType().type != VarType.VARTYPE_OBJECT.type)
          continue;
        Origins value = compute(expression, arrayWrites);
        Origins previous = origins.getOrDefault(expression, Origins.EMPTY);
        value = previous.merge(value);
        if (!value.equals(previous)) {
          origins.put(expression, value);
          changed = true;
        }
      }
    } while (changed);
    for (Exprent expression : expressions) {
      if (expression instanceof AssignmentExprent assignment) {
        if (assignment.getLeft() instanceof ArrayExprent array) {
          Origins target = get(array.getArray());
          for (NewExprent allocation : target.allocations()) {
            stores.computeIfAbsent(allocation, ignored -> new ArrayList<>())
              .add(new Store(array, assignment.getCondType() == null ? assignment.getRight() : assignment));
            contain(allocation, assignment.getRight());
          }
          if (target.external()) {
            for (NewExprent allocation : get(assignment.getRight()).allocations())
              publications.computeIfAbsent(allocation, ignored -> new ArrayList<>()).add(array);
          }
        } else if (assignment.getLeft() instanceof FieldExprent field) {
          escapeUnlessBound(assignment.getRight(), analysis.mappings.contract(fieldKey(field), "field", -1));
        }
      } else if (expression instanceof InvocationExprent invocation) {
        if (invocation.getInstance() != null && isContainer(invocation.getInstance())) {
          for (NewExprent allocation : get(invocation.getInstance()).allocations()) {
            containerUses.computeIfAbsent(allocation, ignored -> new ArrayList<>()).add(invocation);
          }
        }
        if (isArrayCopy(invocation)) {
          // arraycopy does not mutate or retain its source array. Reference
          // contents can be published through the destination; the destination
          // itself receives opaque writes until its copy layout is established.
          publishedContents.addAll(get(invocation.getLstParameters().get(0)).allocations());
          escape(invocation.getLstParameters().get(2));
          continue;
        }
        for (int i = 0; i < invocation.getLstParameters().size(); i++) {
          Exprent argument = invocation.getLstParameters().get(i);
          escapeUnlessBound(argument, analysis.mappings.contract(invocationKey(invocation), "parameter", i));
        }
      }
    }
    for (Exprent expression : expressions) {
      if (expression instanceof NewExprent allocation)
        for (Exprent element : allocation.getLstArrayElements()) contain(allocation, element);
    }
    for (NewExprent allocation : publishedContents) escaped.addAll(contents.getOrDefault(allocation, Set.of()));
  }

  private static boolean isArrayCopy(InvocationExprent invocation) {
    return invocation.isStatic() && invocation.getClassname().equals("java/lang/System") && invocation.getName().equals("arraycopy")
      && invocation.getStringDescriptor().equals("(Ljava/lang/Object;ILjava/lang/Object;II)V");
  }

  private Origins compute(Exprent expression, List<AssignmentExprent> arrayWrites) {
    if (expression instanceof NewExprent creation)
      return new Origins(Set.of(creation), false);
    if (expression instanceof ConstExprent constant && constant.getValue() == null)
      return Origins.EMPTY;
    if (expression instanceof VarExprent variable) {
      Set<SemanticLocalFlow.Definition> sources = locals.sources(variable);
      if (sources.isEmpty())
        return Origins.EXTERNAL;
      Origins result = Origins.EMPTY;
      for (SemanticLocalFlow.Definition source : sources)
        result = result.merge(source.value == null ? Origins.EXTERNAL : get(source.value));
      return result;
    }
    if (expression instanceof ArrayExprent array) {
      Origins base = get(array.getArray());
      Origins result = base.external() ? Origins.EXTERNAL : Origins.EMPTY;
      for (NewExprent allocation : base.allocations()) {
        for (Exprent element : allocation.getLstArrayElements()) result = result.merge(get(element));
        for (AssignmentExprent store : arrayWrites) {
          if (get(((ArrayExprent) store.getLeft()).getArray()).allocations().contains(allocation))
            result = result.merge(get(store.getRight()));
        }
      }
      return result;
    }
    if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null)
      return get(assignment.getRight());
    if (expression instanceof FunctionExprent function) {
      if (function.getFuncType() == FunctionExprent.FunctionType.CAST)
        return get(function.getLstOperands().get(0));
      if (function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
        return get(function.getLstOperands().get(1)).merge(get(function.getLstOperands().get(2)));
      }
    }
    return Origins.EXTERNAL;
  }

  private Origins get(Exprent expression) {
    return origins.getOrDefault(expression, Origins.EMPTY);
  }
  private void escape(Exprent expression) {
    escaped.addAll(get(expression).allocations());
  }

  private void escapeUnlessBound(Exprent expression, SemanticContract contract) {
    for (NewExprent allocation : get(expression).allocations()) {
      if (allocation.getNewType().arrayDim > 0 && contract.array() == null || isContainer(allocation) && contract.container() == null)
        escaped.add(allocation);
    }
  }

  private void contain(NewExprent holder, Exprent value) {
    for (NewExprent allocation : get(value).allocations()) {
      contents.computeIfAbsent(holder, ignored -> Collections.newSetFromMap(new IdentityHashMap<>())).add(allocation);
      holders.computeIfAbsent(allocation, ignored -> Collections.newSetFromMap(new IdentityHashMap<>())).add(holder);
    }
  }

  boolean isLocal(Exprent expression) {
    Origins source = get(expression);
    return !source.external() && !source.allocations().isEmpty()
      && source.allocations().stream().noneMatch(allocation -> escapes(allocation, Collections.newSetFromMap(new IdentityHashMap<>())));
  }

  private boolean escapes(NewExprent allocation, Set<NewExprent> visiting) {
    if (!visiting.add(allocation))
      return false;
    if (escaped.contains(allocation))
      return true;
    // Publication into a mapped table is a schema boundary, just like a mapped
    // field or method parameter. Resolve at the store, after its receiver's
    // reaching definitions are known; an untyped holder still constitutes an escape.
    for (ArrayExprent publication : publications.getOrDefault(allocation, List.of())) {
      SemanticFacts contract = analysis.factsOf(publication);
      if (contract.unknown() || contract.arrays().isEmpty())
        return true;
    }
    for (NewExprent holder : holders.getOrDefault(allocation, Set.of()))
      if (escapes(holder, visiting))
        return true;
    return false;
  }

  static boolean isContainer(Exprent expression) {
    return expression.getExprType().arrayDim == 0
      && Set.of("java/util/Vector", "java/util/Hashtable")
           .contains(expression.getExprType().value == null ? "" : expression.getExprType().value);
  }

  List<InvocationExprent> containerUses(NewExprent allocation) {
    return containerUses.getOrDefault(allocation, List.of());
  }

  List<Store> stores(NewExprent allocation) {
    return stores.getOrDefault(allocation, List.of());
  }

  SemanticFacts facts(NewExprent allocation) {
    if (escapes(allocation, Collections.newSetFromMap(new IdentityHashMap<>())))
      return UNKNOWN;
    if (isContainer(allocation))
      return BOTTOM;
    SemanticFacts result = BOTTOM;
    for (Exprent element : allocation.getLstArrayElements())
      result = result.merge(lift(analysis.factsOf(element), element.getExprType().arrayDim, allocation.getNewType().arrayDim));
    for (Store store : stores(allocation))
      result = result.merge(lift(analysis.factsOf(store.value()), store.value().getExprType().arrayDim, allocation.getNewType().arrayDim));
    // A partly unknown collection does not establish a uniform row shape. An
    // explicit boundary may supply one later, without inheriting partial guesses.
    return result.unknown() ? UNKNOWN : result;
  }

  private static SemanticFacts lift(SemanticFacts element, int rank, int arrayRank) {
    // Object arrays can contain themselves. Only lift statically compatible
    // dimensions, so cyclic stores cannot invent arbitrarily deep shapes.
    if (!element.isEmpty() && rank != arrayRank - 1)
      return UNKNOWN;
    // A fresh literal array can adopt a boundary contract. As one row of a
    // collection, though, it is not evidence for a uniform shape: a typed
    // sibling must not become its default through inference feedback.
    if (element.isEmpty())
      return rank > 0 ? UNKNOWN : BOTTOM;
    Set<ArraySemantics> arrays = new HashSet<>();
    for (ArraySemantics shape : element.arrays()) arrays.add(shape.outer());
    // A scalar with a domain may be one column of a heterogeneous record.
    // It is not evidence for a default domain on every element of the array.
    return new SemanticFacts(Set.of(), arrays, Set.of(), Set.of(),
      element.unknown() || !element.domains().isEmpty() || !element.dependent().isEmpty() || !element.containers().isEmpty());
  }
}
