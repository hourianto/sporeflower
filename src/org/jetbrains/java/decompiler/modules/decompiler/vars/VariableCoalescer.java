package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.StackVarsProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.sforms.SSAUConstructorSparseEx;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.TypeFamily;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.StatementIterator;

import java.util.*;

/** Coalesces printable locals while keeping lexical scope, type constraints and value lifetimes consistent. */
final class VariableCoalescer {
  private final RootStatement root;
  private final StructMethod mt;
  private final VarProcessor varproc;
  private final Set<Integer> semanticParameterSlots;
  private final boolean j2meStrictSlotMerge;
  private final Map<VarVersionPair, Set<VarType>> legacySlotTypeEvidence;
  private final Map<VarVersionPair, Set<VarType>> assignmentUseUpperBounds = new HashMap<>();
  private final Set<VarVersionPair> nullAssignmentDefinitions = new HashSet<>();

  VariableCoalescer(RootStatement root, StructMethod mt, VarProcessor varproc) {
    this.root = root;
    this.mt = mt;
    this.varproc = varproc;
    this.semanticParameterSlots = findSemanticParameterSlots();
    this.j2meStrictSlotMerge = DecompilerContext.getOption(IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE)
      || mt.hasAttribute(StructGeneralAttribute.ATTRIBUTE_STACK_MAP);
    this.legacySlotTypeEvidence = j2meStrictSlotMerge ? collectLegacySlotTypeEvidence() : new HashMap<>();
  }

  private void collectReceiverBound(Exprent expression) {
    Exprent value;
    VarType bound;
    if (expression instanceof InvocationExprent invocation) {
      value = invocation.getInstance();
      bound = new VarType(CodeType.OBJECT, 0, invocation.getClassname());
    } else if (expression instanceof FieldExprent field) {
      value = field.getInstance();
      bound = new VarType(CodeType.OBJECT, 0, field.getClassname());
    } else if (expression instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.RETURN) {
      value = exit.getValue();
      bound = exit.getRetType();
    } else {
      return;
    }
    // Object imposes no narrower reference requirement. Keep the type already
    // inferred from actual values and more specific member/return uses.
    if (!(value instanceof VarExprent variable) || bound.typeFamily != TypeFamily.OBJECT || VarType.VARTYPE_OBJECT.equals(bound)) return;
    Map<VarVersionPair, VarType> bounds = varproc.getVarVersions().getTypeProcessor().getUpperBounds();
    VarVersionPair pair = variable.getVarVersionPair();
    VarType previous = bounds.get(pair);
    if (previous == null || previous.typeFamily != TypeFamily.OBJECT || previous.higherEqualInLatticeThan(bound)) {
      bounds.put(pair, bound);
    }
  }

  static class VarID {
    final VarExprent var;

    VarID(VarExprent var) {
      this.var = var;
    }

    @Override
    public int hashCode() {
      return System.identityHashCode(var);
    }

    @Override
    public boolean equals(Object obj) {
      return obj instanceof VarID varID && var == varID.var;
    }
  }

  private Map<VarID, Set<VarID>> getVarExprentSources() {
    // Do an ssau analysis to find the sources of variables
    SSAUConstructorSparseEx ssau = new SSAUConstructorSparseEx();
    try {
      ssau.splitVariables(root, mt);
    } catch (NullPointerException t) {
      // Can happen when something is wrong with variables ...

      StackVarsProcessor.setVersionsToNull(root);
      return null;
    }

    Map<VarVersionPair, VarID> lookup = new HashMap<>();
    findAllVarExprents(root, lookup);

    Map<VarID, Set<VarID>> sources = new HashMap<>();
    for (VarVersionNode node : ssau.getSsuVersions().nodes) {
      VarID target = lookup.get(node.asPair());
      if (target == null) {
        continue;
      }

      Set<VarID> sourceVars = new HashSet<>();

      for (VarVersionNode predecessor : node.getPredecessors()) {
        VarID source = lookup.get(predecessor.asPair());
        if (source != null) {
          sourceVars.add(source);
        }
      }

      if (node.phantomNode != null) {
        VarID source = lookup.get(node.phantomNode.asPair());
        if (source != null) {
          sourceVars.add(source);
        }
      }

      if (!sourceVars.isEmpty()) {
        sources.put(target, sourceVars);
      }
    }

    StackVarsProcessor.setVersionsToNull(root);

    return sources;
  }

  private static void findAllVarExprents(Statement stat, Map<VarVersionPair, VarID> lookup) {
    for (Exprent exprent : stat.getVarDefinitions()) {
      if (exprent instanceof VarExprent varExprent) {
        lookup.put(new VarVersionPair(varExprent), new VarID(varExprent));
      }
    }
    List<Exprent> lst = stat.getExprents();
    if (lst != null) {
      for (Exprent exprent : lst) {
        for (Exprent exp : exprent.getAllExprents(true, true)) {
          if (exp instanceof VarExprent varExprent) {
            lookup.put(new VarVersionPair(varExprent), new VarID(varExprent));
          }
        }
      }
    }

    for (Statement subStat : stat.getStats()) {
      findAllVarExprents(subStat, lookup);
    }
  }

  private void compareVarExprentSources(
    Map<VarID, Set<VarID>> oldSources,
    Map<VarID, Set<VarID>> newSources
  ) {
    if (newSources == null) return;

    for (var oldEntry : oldSources.entrySet()) {
      Set<VarID> oldSet = oldEntry.getValue();
      Set<VarID> newSet = newSources.get(oldEntry.getKey());

      // Check if sets match
      if (!Objects.equals(oldSet, newSet)) {
        root.addComment("$VF: Variable merging failed for merge " + oldEntry.getKey().var + ". Code has semantic differences!");
      }
    }

    for (var newVar : newSources.keySet()) {
      if (!oldSources.containsKey(newVar)) {
        root.addComment("$VF: Variable merging added a var? " + newVar.var);
      }
    }
  }

  void coalesce() {
    Map<Integer, VarVersionPair> parameters = new HashMap<>();
    MethodDescriptor md = MethodDescriptor.parseDescriptor(mt.getDescriptor());

    int index = 0;
    // this var
    if (!mt.hasModifier(CodeConstants.ACC_STATIC)) {
      VarVersionPair receiver = new VarVersionPair(index, 0);
      parameters.put(index, receiver);
      index++;
    }

    for (VarType var : md.params) {
      VarVersionPair parameter = new VarVersionPair(index, 0);
      parameters.put(index, parameter);
      index += var.stackSize;
    }

    nullAssignmentDefinitions.clear();
    assignmentUseUpperBounds.clear();
    VariableOccurrences occurrences = new VariableOccurrences(root, exprent -> {
      collectReceiverBound(exprent);
      VarVersionPair nullDefinition = getNullAssignmentDefinition(exprent);
      if (nullDefinition != null) nullAssignmentDefinitions.add(nullDefinition);
      if (exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
        collectAssignmentUseUpperBound(assignment.getRight(), assignment.getLeft().getExprType());
      }
    });

    Map<VarID, Set<VarID>> sources = null;
    if (DecompilerContext.getOption(IFernflowerPreferences.VERIFY_PRE_POST_VARIABLE_MERGES)) {
      sources = getVarExprentSources();
    }

    // Scope discovery visits every declaration once. Merges contract this
    // pass's occurrence and interference indexes at the same mutation boundary.
    MergePass pass = new MergePass(occurrences);
    new VariableScopes(varproc::getVarOriginalPair, pass::tryMerge).process(root, parameters);

    if (sources != null) {
      Map<VarID, Set<VarID>> newSources = getVarExprentSources();
      compareVarExprentSources(sources, newSources);
    }
  }

  private final class MergePass {
    private final VariableOccurrences occurrences;
    private VariableInterference interference;

    private MergePass(VariableOccurrences occurrences) {
      this.occurrences = occurrences;
    }

    private boolean tryMerge(Exprent declaration, VarVersionPair to) {
      VarExprent variable = VariableScopes.declarationVariable(declaration);
      VarVersionPair from = variable.getVarVersionPair();
      int slot = varproc.getVarOriginalIndex(from.var);
      if (!canMergeWithExistingVar(slot, from, to)) return false;
      VarType merged = compatibleType(from, to, getNullAssignmentMergeType(declaration, from, to));
      if (merged == null) return false;
      if (interference == null) interference = new VariableInterference(root);
      if (!interference.canMerge(from, to) || !occurrences.merge(from, to, merged)) return false;
      varproc.setVarType(to, merged);
      mergeNullAssignmentDefinitions(from, to, merged);
      mergeLegacySlotTypeEvidence(from, to);
      mergeAssignmentUseUpperBounds(from, to);
      interference.merge(from, to);
      return true;
    }
  }

  private VarType getNullAssignmentMergeType(Exprent exp, VarVersionPair source, VarVersionPair target) {
    if (isNullAssignmentDefinition(exp)) {
      // A null-only local is assignable to the existing reference type. Do not let
      // its normalized Object type widen a concrete array/object slot merge.
      VarType targetType = varproc.getVarType(target);
      return targetType != null && targetType.typeFamily == TypeFamily.OBJECT ? targetType : null;
    }

    return getExistingNullAssignmentMergeType(source, target);
  }

  private VarType getExistingNullAssignmentMergeType(VarVersionPair source, VarVersionPair target) {
    if (!nullAssignmentDefinitions.contains(target)) {
      return null;
    }

    VarType sourceType = varproc.getVarType(source);
    return isSpecificReferenceType(sourceType) && satisfiesAssignmentUseUpperBounds(sourceType, target) ? sourceType : null;
  }

  private void collectAssignmentUseUpperBound(Exprent exprent, VarType upperBound) {
    if (upperBound == null || upperBound.type == CodeType.UNKNOWN) {
      return;
    }

    if (exprent instanceof VarExprent var) {
      assignmentUseUpperBounds.computeIfAbsent(new VarVersionPair(var), key -> new HashSet<>()).add(upperBound);
    }
    else if (exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
      collectAssignmentUseUpperBound(assignment.getRight(), upperBound);
    }
  }

  private static boolean isNullAssignmentDefinition(Exprent exprent) {
    return getNullAssignmentDefinition(exprent) != null;
  }

  private static VarVersionPair getNullAssignmentDefinition(Exprent exprent) {
    return exprent instanceof AssignmentExprent assignment &&
      assignment.getLeft() instanceof VarExprent var &&
      var.isDefinition() &&
      assignment.getRight() instanceof ConstExprent constExpr &&
      constExpr.isNull()
      ? new VarVersionPair(var)
      : null;
  }

  private void mergeNullAssignmentDefinitions(VarVersionPair from, VarVersionPair to, VarType merged) {
    boolean mergedIsStillOnlyNull = VarType.VARTYPE_OBJECT.equals(merged) &&
      nullAssignmentDefinitions.contains(from) &&
      nullAssignmentDefinitions.contains(to);
    nullAssignmentDefinitions.remove(from);
    nullAssignmentDefinitions.remove(to);
    if (mergedIsStillOnlyNull) {
      nullAssignmentDefinitions.add(to);
    }
  }

  private void mergeAssignmentUseUpperBounds(VarVersionPair from, VarVersionPair to) {
    Set<VarType> fromTypes = assignmentUseUpperBounds.remove(from);
    if (fromTypes == null || fromTypes.isEmpty()) {
      return;
    }

    assignmentUseUpperBounds.computeIfAbsent(to, key -> new HashSet<>()).addAll(fromTypes);
  }

  private boolean satisfiesAssignmentUseUpperBounds(VarType mergedType, VarVersionPair pair) {
    Set<VarType> upperBounds = assignmentUseUpperBounds.get(pair);
    if (upperBounds == null || upperBounds.isEmpty()) {
      return true;
    }

    for (VarType upperBound : upperBounds) {
      if (!upperBound.higherEqualInLatticeThan(mergedType)) {
        return false;
      }
    }
    return true;
  }

  private boolean canMergeWithExistingVar(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    // A copy of the current receiver is a distinct source value even when its
    // bytecode slot previously held an assignable parameter or local. Reusing
    // that source variable would retain the old declared type while changing
    // the value's member-resolution semantics.
    if (varproc.isReceiverEquivalent(current) != varproc.isReceiverEquivalent(existing)) {
      return false;
    }

    // A parameter binding describes the value supplied by the caller, not every
    // unrelated value an obfuscated method later stores in the same JVM slot.
    if (isOverwrittenSemanticParameter(originalIndex, current, existing)) {
      return false;
    }

    if (!isOverwrittenReceiverSlot(originalIndex, current, existing)) {
      return !isIncompatibleOverwrittenParameterSlot(originalIndex, current, existing);
    }

    // Slot 0 can be reassigned in obfuscated bytecode. Keep those locals distinct from the Java receiver.
    return current.equals(existing);
  }

  private boolean isOverwrittenSemanticParameter(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    return !current.equals(existing)
      && varproc.getParams().contains(existing)
      && semanticParameterSlots.contains(originalIndex);
  }

  private Set<Integer> findSemanticParameterSlots() {
    SemanticMappings mappings = DecompilerContext.getContextProperty(DecompilerContext.SEMANTIC_MAPPINGS);
    if (mappings == null) {
      return Set.of();
    }

    Set<Integer> slots = new HashSet<>();
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(mt.getDescriptor());
    MemberKey method = new MemberKey(mt.getClassQualifiedName(), mt.getName(), mt.getDescriptor());
    int slot = mt.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      if (mappings.hasParameterSemantics(method, parameter)) {
        slots.add(slot);
      }
      slot += descriptor.params[parameter].stackSize;
    }
    return Set.copyOf(slots);
  }

  private boolean isIncompatibleOverwrittenParameterSlot(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    if (current.equals(existing) || !varproc.getParams().contains(existing)) {
      return false;
    }

    VarType parameterType = varproc.getParameterTypeByOriginalIndex(originalIndex);
    VarType currentType = varproc.getVarType(current);
    return parameterType != null &&
      currentType != null &&
      currentType.type != CodeType.UNKNOWN &&
      !isAssignableToDeclaredParameter(parameterType, currentType);
  }

  private static boolean isAssignableToDeclaredParameter(VarType parameterType, VarType currentType) {
    if (parameterType.type == CodeType.BOOLEAN || currentType.type == CodeType.BOOLEAN) {
      return parameterType.type == currentType.type;
    }

    return parameterType.higherCrossFamilyThan(currentType, true);
  }

  private Map<VarVersionPair, Set<VarType>> collectLegacySlotTypeEvidence() {
    Map<VarVersionPair, Set<VarType>> observedTypes = new HashMap<>();
    StatementIterator.iterate(root, exprent -> {
      if (exprent instanceof VarExprent var && var.getVersion() >= 0 && !var.getStackMapTypes().isEmpty()) {
        observedTypes.computeIfAbsent(new VarVersionPair(var), k -> new HashSet<>()).addAll(var.getStackMapTypes());
      }
      return 0;
    });
    return observedTypes;
  }

  private void mergeLegacySlotTypeEvidence(VarVersionPair from, VarVersionPair to) {
    if (!j2meStrictSlotMerge || legacySlotTypeEvidence.isEmpty()) {
      return;
    }

    Set<VarType> fromTypes = legacySlotTypeEvidence.remove(from);
    if (fromTypes == null || fromTypes.isEmpty()) {
      return;
    }

    legacySlotTypeEvidence.computeIfAbsent(to, key -> new HashSet<>()).addAll(fromTypes);
  }

  private boolean hasIncompatibleLegacySlotTypes(VarVersionPair from, VarVersionPair to) {
    if (legacySlotTypeEvidence.isEmpty()) {
      return false;
    }

    Set<VarType> fromTypes = legacySlotTypeEvidence.get(from);
    Set<VarType> toTypes = legacySlotTypeEvidence.get(to);
    if (fromTypes == null || fromTypes.isEmpty() || toTypes == null || toTypes.isEmpty()) {
      return false;
    }

    for (VarType fromType : fromTypes) {
      for (VarType toType : toTypes) {
        if (!areSlotTypesCompatible(fromType, toType)) {
          return true;
        }
      }
    }

    return false;
  }

  private boolean areSlotTypesCompatible(VarType first, VarType second) {
    if (first == null || second == null) {
      return true;
    }

    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) {
      return true;
    }

    boolean firstReference = first.typeFamily == TypeFamily.OBJECT;
    boolean secondReference = second.typeFamily == TypeFamily.OBJECT;

    if (first.type == CodeType.NULL || second.type == CodeType.NULL) {
      return firstReference && secondReference;
    }

    if (firstReference != secondReference) {
      return false;
    }

    if (!firstReference) {
      return first.type == second.type;
    }

    if (first.equals(VarType.VARTYPE_OBJECT) || second.equals(VarType.VARTYPE_OBJECT)) {
      return true;
    }

    if (first.arrayDim != second.arrayDim) {
      return false;
    }

    if (first.arrayDim > 0) {
      return first.equals(second);
    }

    if (first.equals(second)) {
      return true;
    }

    return DecompilerContext.getStructContext().instanceOf(first.value, second.value)
      || DecompilerContext.getStructContext().instanceOf(second.value, first.value);
  }

  private VarType compatibleType(VarVersionPair from, VarVersionPair to, VarType mergedTypeOverride) {
    // Keep named packed fields separate from the word they came from and from
    // unrelated slot reuse. Copies carrying the same field identity may merge.
    if (!Objects.equals(varproc.getSemanticName(from), varproc.getSemanticName(to))) {
      return null;
    }
    if (j2meStrictSlotMerge && hasIncompatibleLegacySlotTypes(from, to)) {
      return null;
    }

    VarType mergedType = mergedTypeOverride != null ? mergedTypeOverride : getMergedType(from, to);
    if (mergedType == null) {
      return null;
    }

    VarType fromType = varproc.getVarType(from);
    VarType toType = varproc.getVarType(to);

    // A missing/broad frame must not erase a boundary already established by
    // inference. Merging unrelated reference lifetimes into Object introduces casts
    // and makes slot reuse harder to reconstruct without providing a Java-level alias.
    if (j2meStrictSlotMerge && !areSlotTypesCompatible(fromType, toType)) {
      return null;
    }

    if (!sameOrUnknownTypeFamily(fromType, toType)) {
      return null;
    }

    if (hasConflictingConcretePrimitiveTypes(fromType, toType)) {
      return null;
    }

    return isLatticeCompatible(mergedType, fromType)
      && isLatticeCompatible(mergedType, toType)
      && (mergedTypeOverride == null || satisfiesUpperBounds(mergedType, from, to)) ? mergedType : null;
  }

  private boolean satisfiesUpperBounds(VarType mergedType, VarVersionPair from, VarVersionPair to) {
    Map<VarVersionPair, VarType> upperBounds = varproc.getVarVersions().getTypeProcessor().getUpperBounds();
    VarType fromMax = upperBounds.get(from);
    VarType toMax = upperBounds.get(to);

    return (fromMax == null || fromMax.higherEqualInLatticeThan(mergedType)) &&
           (toMax == null || toMax.higherEqualInLatticeThan(mergedType));
  }

  private static boolean hasConflictingConcretePrimitiveTypes(VarType first, VarType second) {
    if (first == null || second == null) {
      return false;
    }

    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) {
      return false;
    }

    if (first.typeFamily == TypeFamily.OBJECT || second.typeFamily == TypeFamily.OBJECT) {
      return false;
    }

    return first.type != second.type;
  }

  private static boolean sameOrUnknownTypeFamily(VarType first, VarType second) {
    if (first == null || second == null) {
      return true;
    }

    if (first.type == CodeType.UNKNOWN || second.type == CodeType.UNKNOWN) {
      return true;
    }

    return first.typeFamily == second.typeFamily;
  }

  private static boolean isLatticeCompatible(VarType mergedType, VarType varType) {
    if (varType == null || varType.type == CodeType.UNKNOWN) {
      return true;
    }

    return mergedType.higherEqualInLatticeThan(varType) || varType.higherEqualInLatticeThan(mergedType);
  }

  private boolean isOverwrittenReceiverSlot(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    return originalIndex == 0
      && !mt.hasModifier(CodeConstants.ACC_STATIC)
      && existing.var == 0
      && existing.version == 0
      && (current.var != 0 || current.version != 0);
  }

  private VarType getMergedType(VarVersionPair from, VarVersionPair to) {
    Map<VarVersionPair, VarType> minTypes = varproc.getVarVersions().getTypeProcessor().getLowerBounds();
    Map<VarVersionPair, VarType> maxTypes = varproc.getVarVersions().getTypeProcessor().getUpperBounds();

    return getMergedType(minTypes.get(from), minTypes.get(to), maxTypes.get(from), maxTypes.get(to));
  }

  private static boolean isSpecificReferenceType(VarType type) {
    return type != null &&
      type.typeFamily == TypeFamily.OBJECT &&
      type.type != CodeType.NULL &&
      type.type != CodeType.UNKNOWN &&
      !type.equals(VarType.VARTYPE_OBJECT);
  }

  private VarType getMergedType(VarType fromMin, VarType toMin, VarType fromMax, VarType toMax) {
    if (!j2meStrictSlotMerge && fromMin != null && fromMin.equals(toMin)) {
      return fromMin; // Short circuit this for simplicities sake
    }

    VarType type = fromMin == null ? toMin : (toMin == null ? fromMin : VarType.join(fromMin, toMin));
    if (type == null || fromMin == null || toMin == null) {
      return null; // no common supertype, skip the remapping
    }

    if (type.type == CodeType.OBJECT) {
      VarType merged = null;
      if (toMax != null) { // The target var is used in direct invocations
        if (fromMax != null) {
          // Max types are the highest class that this variable is used as a direct instance of without any casts.
          // This will pull up the to var type if the from requires a higher class type.
          // EXA: Collection -> List
          if (DecompilerContext.getStructContext().instanceOf(fromMax.value, toMax.value)) {
            merged = fromMax;
          }
        } else {
          // Pull to up to from: List -> ArrayList
          if (DecompilerContext.getStructContext().instanceOf(fromMin.value, toMax.value)) {
            merged = fromMin;
          }
        }
      } else {
        if (fromMax != null) {
          if (DecompilerContext.getStructContext().instanceOf(fromMax.value, toMin.value)) {
            merged = fromMax;
          }
        } else {
          if (DecompilerContext.getStructContext().instanceOf(toMin.value, fromMin.value)) {
            merged = toMin;
          }

          if (merged == null && DecompilerContext.getStructContext().instanceOf(fromMin.value, toMin.value)) {
            merged = toMin;
          }
        }
      }

      if (merged == null) {
        return null;
      }

      if (j2meStrictSlotMerge && !merged.higherEqualInLatticeThan(type)) {
        merged = type;
      }

      if (fromMax != null && !fromMax.higherEqualInLatticeThan(merged)) {
        return null;
      }

      if (toMax != null && !toMax.higherEqualInLatticeThan(merged)) {
        return null;
      }

      return merged;
    } else {
      // Both nonnull at this point
      if (!fromMin.equals(toMin) && !fromMin.higherInLatticeThan(toMin)) {
        // If type we're merging into the old type isn't a strict superset of the old type, we cannot merge
        return null;
      }

      // Keep primitive merges within all known upper-bound constraints.
      // This prevents cross-family remaps like Object-slot -> int-slot reuse from collapsing into one local.
      if (fromMax != null && !fromMax.higherEqualInLatticeThan(type)) {
        return null;
      }

      if (toMax != null && !toMax.higherEqualInLatticeThan(type)) {
        return null;
      }

      return type;
    }
  }

}
