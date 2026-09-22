package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.TypeFamily;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.*;
import java.util.function.BiConsumer;
import java.util.function.Consumer;

/**
 * Type facts for source-local coalescing, after bytecode type inference.
 *
 * Inferred value types, use requirements and fixed Java bindings have different
 * meanings. A source declaration must accept every value assigned to it, keep
 * uses that currently need no cast, retain a parameter's declared type, and
 * preserve the checked-exception effects of throws.
 * Proposals combine all facts without mutation; only an accepted merge installs
 * them. In particular, a sequence of merges cannot forget an earlier use or
 * narrow a declaration merely because a later lifetime has a narrower type.
 */
final class VariableTypeConstraints {
  private static final VarType RUNTIME_EXCEPTION = new VarType("java/lang/RuntimeException", true);
  private static final VarType ERROR = new VarType("java/lang/Error", true);
  private final VarProcessor processor;
  private final VarTypeProcessor inferred;
  private final boolean strictFrames;
  private final Map<VarVersionPair, Collected> collected = new HashMap<>();
  private final Map<VarVersionPair, Local> locals = new HashMap<>();

  VariableTypeConstraints(VarProcessor processor, StructMethod method) {
    this.processor = processor;
    this.inferred = processor.getVarVersions().getTypeProcessor();
    this.strictFrames = DecompilerContext.getOption(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE)
      || method.hasAttribute(StructGeneralAttribute.ATTRIBUTE_STACK_MAP);
    // SSA may represent all uses of an entry value by its writable phi. The
    // signature still supplies a real candidate even with no VarExprent left.
    for (VarVersionPair parameter : processor.getParams()) {
      VarType declared = processor.getDeclaredParameterType(parameter.var);
      collected.put(parameter, new Collected(declared == null ? processor.getVarType(parameter) : declared));
    }
  }

  void collect(Exprent expression) {
    if (expression instanceof VarExprent variable) {
      Collected facts = facts(variable);
      facts.frames.addAll(variable.getStackMapTypes());
      if (variable.getLVT() != null && DecompilerContext.getOption(IFernflowerPreferences.USE_DEBUG_VAR_NAMES)) {
        facts.fixed = variable.getLVT().getVarType();
      }
    }
    VarExprent written = ExprUtil.getWrittenLocal(expression);
    if (written != null) {
      Collected local = facts(written);
      local.hasWrite = true;
      local.onlyNullWrites &= expression instanceof AssignmentExprent assignment && assignment.getCondType() == null
        && assignment.getRight() instanceof ConstExprent constant && constant.isNull();
    }

    // Reuse expression typing contracts, including arguments and return values.
    // Keep separate requirements: unrelated interfaces cannot in general be
    // represented by the single upper bound used during early inference.
    CheckTypesResult bounds = expression.checkExprTypeBounds();
    if (bounds != null) {
      for (CheckTypesResult.ExprentTypePair bound : bounds.getUpperBounds()) require(bound.exprent, bound.type);
    }
    if (expression instanceof FieldExprent field && field.getInstance() != null) {
      require(field.getInstance(), new VarType(CodeType.OBJECT, 0, field.getClassname()));
    } else if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
      require(assignment.getRight(), assignment.getLeft().getExprType());
    } else if (expression instanceof ArrayExprent array && array.getArray().getExprType().arrayDim > 0) {
      require(array.getArray(), array.getArray().getExprType());
    } else if (expression instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.THROW) {
      require(exit.getValue(), facts -> facts.thrown = true);
    }
  }

  private Collected facts(VarExprent variable) {
    return collected.computeIfAbsent(variable.getVarVersionPair(), pair -> new Collected(variable.getVarType()));
  }

  private void require(Exprent expression, VarType type) {
    if (type != null && type.type != CodeType.UNKNOWN) require(expression, facts -> facts.requirements.add(type));
  }

  private void require(Exprent expression, Consumer<Collected> requirement) {
    if (expression instanceof VarExprent variable) {
      requirement.accept(facts(variable));
    } else if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
      require(assignment.getLeft(), requirement);
      require(assignment.getRight(), requirement);
    } else if (expression instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      require(function.getLstOperands().get(1), requirement);
      require(function.getLstOperands().get(2), requirement);
    }
  }

  void finish(BiConsumer<VarVersionPair, VarType> retype) {
    collected.forEach((pair, facts) -> {
      VarType fixed = processor.getParams().contains(pair) ? processor.getDeclaredParameterType(pair.var) : facts.fixed;
      // Object is the printable normalization of a null-only inferred value.
      // Recover NULL from all writes, not just the first declaration assignment.
      VarType value = fixed != null ? fixed : facts.hasWrite && facts.onlyNullWrites ? VarType.VARTYPE_NULL : facts.type;
      Set<VarType> requirements = new HashSet<>(facts.requirements);
      VarType upper = inferred.getUpperBounds().get(pair);
      if (upper != null) requirements.add(upper);
      // Some bytecode already requires casts or primitive conversions. Preserve
      // those existing uses without treating inconsistent inference bounds as
      // a reason to reject even an unchanged declaration type.
      requirements.removeIf(bound -> value == null || bound.typeFamily != value.typeFamily || !bound.higherEqualInLatticeThan(value));
      Set<VarType> thrown = facts.thrown && value != null ? Set.of(value) : Set.of();
      VarType type = facts.type;
      if (fixed == null && !thrown.isEmpty() && !accepts(type, value, requirements, thrown)) {
        // Inference normalizes null-only locals to Object. Establish a legal
        // throwing declaration even when this lifetime never merges with another.
        VarType candidate = declarationType(value, requirements, thrown);
        if (accepts(candidate, value, requirements, thrown)) {
          type = candidate;
          processor.setVarType(pair, type);
          retype.accept(pair, type);
        }
      }
      locals.put(pair, new Local(type, value, fixed, Set.copyOf(requirements), thrown, Set.copyOf(facts.frames)));
    });
    collected.clear();
  }

  Merge propose(VarVersionPair from, VarVersionPair to) {
    Local source = locals.get(from), target = locals.get(to);
    if (source == null || target == null || source.value == null || target.value == null) return null;
    if (!compatibleFamilies(source.value, target.value)) return null;
    if (strictFrames && !compatibleEvidence(source.frames, target.frames)) return null;

    VarType value = VarType.join(source.value, target.value);
    if (value == null) return null;
    // A useful shared superclass can describe sibling implementations without
    // casts. Joining unrelated concrete lifetimes at Object offers no such
    // source-level abstraction; keep that boundary in strict mode.
    if (strictFrames && value.equals(VarType.VARTYPE_OBJECT)
        && !source.value.equals(value) && !target.value.equals(value)) return null;
    if (source.fixed != null && target.fixed != null && !source.fixed.equals(target.fixed)) return null;
    VarType fixed = target.fixed != null ? target.fixed : source.fixed;
    Set<VarType> requirements = union(source.requirements, target.requirements);
    Set<VarType> thrown = union(source.thrown, target.thrown);
    VarType type = fixed != null ? fixed : declarationType(value, requirements, thrown);
    if (!accepts(type, value, requirements, thrown)) return null;
    return new Merge(from, to, new Local(type, value, fixed, requirements, thrown, union(source.frames, target.frames)));
  }

  private static VarType declarationType(VarType value, Set<VarType> requirements, Set<VarType> thrown) {
    if (value.typeFamily != TypeFamily.OBJECT) return value;
    // Throwing null raises an unchecked NPE. A Throwable declaration would
    // instead introduce a checked effect even though no non-null value exists.
    VarType type = value;
    if (value.type == CodeType.NULL) type = thrown.isEmpty() ? VarType.VARTYPE_OBJECT : RUNTIME_EXCEPTION;
    // Choose the widest useful type in the interval between assigned values
    // and required uses. This avoids repeated upcasts to a shared API type.
    // Object alone is not a reason to discard a concrete value's useful type.
    for (VarType bound : requirements) {
      if (!bound.equals(VarType.VARTYPE_OBJECT) && accepts(bound, value, requirements, thrown)
          && (!accepts(type, value, requirements, thrown) || bound.higherInLatticeThan(type))) type = bound;
    }
    return type;
  }

  private static boolean accepts(VarType type, VarType value, Set<VarType> requirements, Set<VarType> thrown) {
    if (type == null || type.type == CodeType.NULL || type.type == CodeType.UNKNOWN || !type.higherEqualInLatticeThan(value)) return false;
    for (VarType bound : requirements) if (!bound.higherEqualInLatticeThan(type)) return false;
    // A throw is not an ordinary Throwable argument: widening its source type
    // can add checked exceptions, changing throws clauses or inducing wrappers.
    // Keep each original checked effect; unchecked declarations add none. This
    // still allows sibling RuntimeExceptions/Errors to share their common type,
    // and null-only lifetimes can use either unchecked hierarchy.
    if (!thrown.isEmpty() && !RUNTIME_EXCEPTION.higherEqualInLatticeThan(type) && !ERROR.higherEqualInLatticeThan(type)) {
      for (VarType original : thrown) if (!original.higherEqualInLatticeThan(type)) return false;
    }
    return true;
  }

  void bind(VarExprent variable) {
    VarVersionPair pair = variable.getVarVersionPair();
    Local local = locals.get(pair);
    if (local != null) {
      VarType type = variable.getVarType();
      locals.put(pair, new Local(type, type, type, local.requirements, local.thrown, local.frames));
    }
  }

  void commit(Merge merge) {
    locals.remove(merge.from);
    locals.put(merge.to, merge.local);
    processor.setVarType(merge.to, merge.type());
    inferred.getLowerBounds().remove(merge.from);
    inferred.getUpperBounds().remove(merge.from);
    // Publish the representable projection for legacy consumers. The full set
    // above remains authoritative throughout both coalescing passes.
    VarType upper = null;
    for (VarType bound : merge.local.requirements) upper = upper == null ? bound : VarType.meet(upper, bound);
    if (upper == null) inferred.getUpperBounds().remove(merge.to);
    else inferred.getUpperBounds().put(merge.to, upper);
  }

  private static boolean compatibleFamilies(VarType first, VarType second) {
    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) return true;
    // Primitive declaration changes can alter implicit narrowing in +=/++.
    // Reference joins are checked against the accumulated use requirements.
    return first.typeFamily == TypeFamily.OBJECT && second.typeFamily == TypeFamily.OBJECT || first.equals(second);
  }

  private static boolean compatibleEvidence(Set<VarType> first, Set<VarType> second) {
    for (VarType a : first) for (VarType b : second) {
      if (a.type == CodeType.UNKNOWN || b.type == CodeType.UNKNOWN) continue;
      if (!compatibleFamilies(a, b)) return false;
      if (a.type == CodeType.NULL || b.type == CodeType.NULL || a.equals(VarType.VARTYPE_OBJECT) || b.equals(VarType.VARTYPE_OBJECT)) continue;
      if (a.arrayDim > 0 || b.arrayDim > 0) {
        if (!a.equals(b)) return false;
      } else if (!a.higherEqualInLatticeThan(b) && !b.higherEqualInLatticeThan(a)) {
        return false;
      }
    }
    return true;
  }

  private static Set<VarType> union(Set<VarType> first, Set<VarType> second) {
    Set<VarType> result = new HashSet<>(first);
    result.addAll(second);
    return Set.copyOf(result);
  }

  record Merge(VarVersionPair from, VarVersionPair to, Local local) {
    VarType type() { return local.type; }
  }

  private record Local(VarType type, VarType value, VarType fixed, Set<VarType> requirements,
                       Set<VarType> thrown, Set<VarType> frames) { }

  private static final class Collected {
    private final VarType type;
    private VarType fixed;
    private final Set<VarType> requirements = new HashSet<>();
    private final Set<VarType> frames = new HashSet<>();
    private boolean hasWrite;
    private boolean onlyNullWrites = true;
    private boolean thrown;

    private Collected(VarType type) { this.type = type; }
  }
}
