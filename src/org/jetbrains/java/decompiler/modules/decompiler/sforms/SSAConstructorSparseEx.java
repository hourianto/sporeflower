// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.jetbrains.java.decompiler.util.collections.SFormsFastMapDirect;

import java.util.*;

public class SSAConstructorSparseEx extends SFormsConstructor {

  // (var, version), version
  private final Map<VarVersionPair, FastSparseSetFactory.FastSparseSet<Integer>> phi = new HashMap<>();
  private PhiComponents phiComponents;
  private final Map<VarVersionPair, VarVersionPair> directAssignments = new HashMap<>();

  public SSAConstructorSparseEx() {
    super(false);
  }

  @Override
  public void markDirectAssignment(VarVersionPair varVersionPair, VarVersionPair rightPair) {
    this.directAssignments.put(varVersionPair, rightPair);
  }

  @Override
  public VarVersionPair getOrCreatePhantom(VarVersionPair pair) {
    return pair;
  }

  @Override
  public Integer getFieldIndex(FieldExprent field) {
    return -1;
  }

  @Override
  void varReadSingleVersion(
    Statement stat,
    boolean calcLiveVars,
    VarExprent varExprent,
    SFormsFastMapDirect varmap,
    int lastVersion) {
    // simply copy the version
    varExprent.setVersion(lastVersion);
  }

  @Override
  void varReadMultipleVersions(
    Statement stat,
    boolean calcLiveVars,
    VarExprent varExprent,
    SFormsFastMapDirect varMap,
    FastSparseSetFactory.FastSparseSet<Integer> versions) {

    phiComponents = null;
    int varIndex = varExprent.getIndex();
    int currentVersion = varExprent.getVersion();
    VarVersionPair varVersion = varExprent.getVarVersionPair();
    if (currentVersion != 0 && this.phi.containsKey(varVersion)) {
      // keep phi node up to date of all inputs
      this.phi.get(varVersion).union(versions);
    } else {
      // increase version
      int nextVer = this.getNextFreeVersion(varIndex, stat);
      // set version
      varExprent.setVersion(nextVer);

      // create new phi node
      this.phi.put(varExprent.getVarVersionPair(), versions);
    }

    varMap.setCurrentVar(varExprent); // update varMap to the phi version
  }

  public Map<VarVersionPair, FastSparseSetFactory.FastSparseSet<Integer>> getPhi() {
    return this.phi;
  }

  /**
   * Finds values formed exclusively from direct copies and phi joins of
   * {@code source}. Require a path back to the source and reject every value
   * with a non-equivalent dependency. This retains source-anchored loop phis without accepting closed
   * source-free cycles.
   */
  public Set<VarVersionPair> getDirectCopyEquivalentVersions(VarVersionPair source) {
    Map<VarVersionPair, Set<VarVersionPair>> dependencies = new HashMap<>();
    for (Map.Entry<VarVersionPair, VarVersionPair> entry : directAssignments.entrySet()) {
      dependencies.put(entry.getKey(), Set.of(entry.getValue()));
    }
    for (Map.Entry<VarVersionPair, FastSparseSetFactory.FastSparseSet<Integer>> entry : phi.entrySet()) {
      Set<VarVersionPair> inputs = new HashSet<>();
      for (int version : entry.getValue()) {
        inputs.add(new VarVersionPair(entry.getKey().var, version));
      }
      dependencies.putIfAbsent(entry.getKey(), inputs);
    }

    Map<VarVersionPair, Set<VarVersionPair>> dependants = new HashMap<>();
    dependencies.forEach((value, inputs) -> inputs.forEach(input ->
      dependants.computeIfAbsent(input, ignored -> new HashSet<>()).add(value)));

    // First require a path back to the source, then propagate any disqualifying
    // input forward. This accepts anchored loops but rejects source-free cycles
    // and phis with even one unrelated input, without repeated global scans.
    Set<VarVersionPair> candidates = new HashSet<>();
    Deque<VarVersionPair> pending = new ArrayDeque<>();
    candidates.add(source);
    pending.add(source);
    while (!pending.isEmpty()) {
      for (VarVersionPair dependant : dependants.getOrDefault(pending.removeFirst(), Set.of())) {
        if (candidates.add(dependant)) pending.addLast(dependant);
      }
    }
    for (VarVersionPair candidate : candidates) {
      if (!candidate.equals(source) && !candidates.containsAll(dependencies.get(candidate))) {
        pending.addLast(candidate);
      }
    }
    while (!pending.isEmpty()) {
      VarVersionPair rejected = pending.removeFirst();
      if (!rejected.equals(source) && candidates.remove(rejected)) {
        pending.addAll(dependants.getOrDefault(rejected, Set.of()));
      }
    }

    return candidates;
  }

  public PhiComponents getPhiComponents() {
    if (phiComponents == null) {
      phiComponents = new PhiComponents(phi);
    }
    return phiComponents;
  }

  @Override
  void initVersion(VarExprent varExprent, Statement stat) {
    if (varExprent.getVersion() == 0) {
      // get next version
      int nextVersion = this.getNextFreeVersion(varExprent.getIndex(), stat);

      // set version
      varExprent.setVersion(nextVersion);
    }
  }

  @Override
  public void initParameter(int varIndex, SFormsFastMapDirect varMap, boolean isCatchVar)  {
    int version = this.getNextFreeVersion(varIndex, this.root); // == 1

    varMap.setCurrentVar(varIndex, version);
  }
}
