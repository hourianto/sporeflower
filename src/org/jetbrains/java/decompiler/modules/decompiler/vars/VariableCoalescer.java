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
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.StatementIterator;

import java.util.*;

/** Coalesces printable locals while keeping lexical scope, type constraints and value lifetimes consistent. */
final class VariableCoalescer {
  private final RootStatement root;
  private final StructMethod mt;
  private final VarProcessor varproc;
  private final Set<Integer> semanticParameterSlots;
  private VariableTypeConstraints types;

  VariableCoalescer(RootStatement root, StructMethod mt, VarProcessor varproc) {
    this.root = root;
    this.mt = mt;
    this.varproc = varproc;
    this.semanticParameterSlots = findSemanticParameterSlots();
  }

  private Map<VarExprent, Set<VarExprent>> getVarExprentSources() {
    // Do an ssau analysis to find the sources of variables
    SSAUConstructorSparseEx ssau = new SSAUConstructorSparseEx();
    try {
      ssau.splitVariables(root, mt);
    } catch (NullPointerException t) {
      // Can happen when something is wrong with variables ...

      StackVarsProcessor.setVersionsToNull(root);
      return null;
    }

    Map<VarVersionPair, VarExprent> lookup = new HashMap<>();
    StatementIterator.iterate(root, expression -> {
      if (expression instanceof VarExprent variable) lookup.put(variable.getVarVersionPair(), variable);
      return 0;
    });

    Map<VarExprent, Set<VarExprent>> sources = new IdentityHashMap<>();
    for (VarVersionNode node : ssau.getSsuVersions().nodes) {
      VarExprent target = lookup.get(node.asPair());
      if (target == null) {
        continue;
      }

      Set<VarExprent> sourceVars = Collections.newSetFromMap(new IdentityHashMap<>());

      for (VarVersionNode predecessor : node.getPredecessors()) {
        VarExprent source = lookup.get(predecessor.asPair());
        if (source != null) {
          sourceVars.add(source);
        }
      }

      if (node.phantomNode != null) {
        VarExprent source = lookup.get(node.phantomNode.asPair());
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

  private void compareVarExprentSources(
    Map<VarExprent, Set<VarExprent>> oldSources,
    Map<VarExprent, Set<VarExprent>> newSources
  ) {
    if (newSources == null) return;

    for (var oldEntry : oldSources.entrySet()) {
      Set<VarExprent> oldSet = oldEntry.getValue();
      Set<VarExprent> newSet = newSources.get(oldEntry.getKey());

      // Check if sets match
      if (!Objects.equals(oldSet, newSet)) {
        root.addComment("$VF: Variable merging failed for merge " + oldEntry.getKey() + ". Code has semantic differences!");
      }
    }

    for (var newVar : newSources.keySet()) {
      if (!oldSources.containsKey(newVar)) {
        root.addComment("$VF: Variable merging added a var? " + newVar);
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

    // Declaration placement between the two passes adds no assignments or uses.
    // Keep the complete constraint state across both passes; accepted merges
    // combine that state rather than rediscovering facts from renamed locals.
    boolean collectTypes = types == null;
    if (collectTypes) types = new VariableTypeConstraints(varproc, mt);
    VariableOccurrences occurrences = new VariableOccurrences(root, expression -> {
      if (collectTypes) types.collect(expression);
    });
    if (collectTypes) types.finish(occurrences::setType);

    Map<VarExprent, Set<VarExprent>> sources = null;
    if (DecompilerContext.getOption(IFernflowerPreferences.VERIFY_PRE_POST_VARIABLE_MERGES)) {
      sources = getVarExprentSources();
    }

    // Scope discovery visits every declaration once. Merges contract this
    // pass's occurrence and interference indexes at the same mutation boundary.
    MergePass pass = new MergePass(occurrences);
    new VariableScopes(varproc::getVarOriginalPair, pass::tryMerge, types::bind).process(root, parameters);

    if (sources != null) {
      Map<VarExprent, Set<VarExprent>> newSources = getVarExprentSources();
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
      VariableTypeConstraints.Merge merge = types.propose(from, to);
      if (merge == null) return false;
      if (interference == null) interference = new VariableInterference(root);
      if (!interference.canMerge(from, to) || !occurrences.merge(from, to, merge.type())) return false;
      types.commit(merge);
      interference.merge(from, to);
      return true;
    }
  }

  private boolean canMergeWithExistingVar(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    // A signature binding cannot be renamed to a body local. Entry-copy
    // materialization may still mark a parameter occurrence as a definition.
    if (varproc.getParams().contains(current)) return false;
    // A copy of the current receiver is a distinct source value even when its
    // bytecode slot previously held an assignable parameter or local. Reusing
    // that source variable would retain the old declared type while changing
    // the value's member-resolution semantics.
    if (varproc.isReceiverEquivalent(current) != varproc.isReceiverEquivalent(existing)
        || !Objects.equals(varproc.getSemanticName(current), varproc.getSemanticName(existing))) {
      return false;
    }

    // A parameter binding describes the value supplied by the caller, not every
    // unrelated value an obfuscated method later stores in the same JVM slot.
    if (isOverwrittenSemanticParameter(originalIndex, current, existing)) {
      return false;
    }

    if (!isOverwrittenReceiverSlot(originalIndex, current, existing)) {
      return true;
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

  private boolean isOverwrittenReceiverSlot(int originalIndex, VarVersionPair current, VarVersionPair existing) {
    return originalIndex == 0
      && !mt.hasModifier(CodeConstants.ACC_STATIC)
      && existing.var == 0
      && existing.version == 0
      && (current.var != 0 || current.version != 0);
  }

}
