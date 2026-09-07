// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.CounterContainer;
import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.sforms.SSAConstructorSparseEx;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.SequenceStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarTypeProcessor.FinalType;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DotExporter;

import java.util.*;
import java.util.Map.Entry;

public class VarVersionsProcessor {
  private final StructMethod method;
  private Map<Integer, VarVersionPair> mapOriginalVarIndices = Collections.emptyMap();
  private Set<VarVersionPair> receiverEquivalentVars = Set.of();
  private final VarTypeProcessor typeProcessor;

  public VarVersionsProcessor(StructMethod mt, MethodDescriptor md) {
    method = mt;
    typeProcessor = new VarTypeProcessor(mt, md);
  }

  // FIXME: This introduces bugs!! (see the FizzBuzz in TestDoublePopAfterJump & TestCompoundAssignmentReplace)
  //  consider:
  //    x = x + 1
  //    use(x);
  //    ...
  //    use(x);
  //  after splitting:
  //    a = x + 1
  //    use(a);
  //    ...
  //    use(a);
  //  after inline 1 of these:
  //    a = x + 1
  //    use(x + 1);
  //    ...
  //    use(a);
  //  merge variables again:
  //    x = x + 1
  //    use(x + 1);
  //    ...
  //    use(x);
  public void setVarVersions(RootStatement root, VarVersionsProcessor previousVersionsProcessor) {
    SSAConstructorSparseEx ssa = new SSAConstructorSparseEx();
    ssa.splitVariables(root, method);

    DirectGraph graph = ssa.getDirectGraph();

    DotExporter.toDotFile(graph, method, "setVarVersions");

    Set<VarVersionPair> receiverEquivalentVersions = Set.of();
    if (CodeConstants.INIT_NAME.equals(method.getName()) && !method.hasModifier(CodeConstants.ACC_STATIC)) {
      receiverEquivalentVersions = ssa.getDirectCopyEquivalentVersions(new VarVersionPair(0, 1));
    }

    Map<VarVersionPair, Integer> phiVersions = mergePhiVersions(ssa);
    Integer receiverVersion = method.hasModifier(CodeConstants.ACC_STATIC) ? null : phiVersions.remove(new VarVersionPair(0, 1));
    updateVersions(graph, phiVersions);
    if (receiverVersion != null) {
      materializeReceiver(root, receiverVersion);
      graph = FlattenStatementsHelper.build(root);
    }
    receiverEquivalentVersions = mergeReceiverEquivalentVersions(receiverEquivalentVersions, phiVersions);

    typeProcessor.calculateVarTypes(root, graph);

    eliminateNonJavaTypes(typeProcessor);

    setNewVarIndices(typeProcessor, graph, previousVersionsProcessor, receiverEquivalentVersions);
  }

  private static Set<VarVersionPair> mergeReceiverEquivalentVersions(
    Set<VarVersionPair> equivalentVersions,
    Map<VarVersionPair, Integer> mergedVersions
  ) {
    if (equivalentVersions.isEmpty()) {
      return Set.of();
    }

    Set<VarVersionPair> result = new HashSet<>();
    Map<VarVersionPair, Set<VarVersionPair>> collapsed = new HashMap<>();
    for (Map.Entry<VarVersionPair, Integer> entry : mergedVersions.entrySet()) {
      VarVersionPair target = new VarVersionPair(entry.getKey().var, entry.getValue());
      collapsed.computeIfAbsent(target, ignored -> new HashSet<>()).add(entry.getKey());
    }
    for (VarVersionPair equivalent : equivalentVersions) {
      if (!mergedVersions.containsKey(equivalent)) {
        result.add(equivalent);
      }
    }
    for (Map.Entry<VarVersionPair, Set<VarVersionPair>> entry : collapsed.entrySet()) {
      if (equivalentVersions.containsAll(entry.getValue())) {
        result.add(entry.getKey());
      }
    }
    return result;
  }

  private Map<VarVersionPair, Integer> mergePhiVersions(SSAConstructorSparseEx ssa) {
    Map<VarVersionPair, Integer> phiVersions = new HashMap<>();
    VarVersionPair receiver = method.hasModifier(CodeConstants.ACC_STATIC) ? null : new VarVersionPair(0, 1);
    for (Set<VarVersionPair> component : ssa.getPhiComponents().groups()) {
      // The JVM receiver can feed a writable phi, but Java's this cannot be its
      // representative. Keep the entry value separate and copy it into that phi.
      int min = Integer.MAX_VALUE;
      for (VarVersionPair pair : component) {
        if (!pair.equals(receiver)) {
          min = Math.min(min, pair.version);
        }
      }
      for (VarVersionPair pair : component) {
        phiVersions.put(pair, min);
      }
    }
    return phiVersions;
  }

  private static void materializeReceiver(RootStatement root, int version) {
    VarProcessor processor = DecompilerContext.getVarProcessor();
    VarExprent receiver = new VarExprent(0, VarType.VARTYPE_UNKNOWN, processor);
    receiver.setVersion(1);
    VarExprent local = new VarExprent(0, VarType.VARTYPE_UNKNOWN, processor);
    local.setVersion(version);
    BasicBlockStatement entry = BasicBlockStatement.create();
    entry.getExprents().add(new AssignmentExprent(local, receiver, null));

    // This is a method-entry edge, outside loops and protected regions. Do not
    // use replaceStatement: backedges must still target the old first statement,
    // otherwise they would reset the writable receiver on every iteration.
    Statement first = root.getFirst();
    SequenceStatement sequence = new SequenceStatement(List.of(entry, first));
    root.getStats().removeWithKey(first.id);
    root.getStats().addWithKey(sequence, sequence.id);
    root.setFirst(sequence);
    sequence.setParent(root);
    sequence.setAllParent();
    entry.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, entry, first));
  }

  private static void updateVersions(DirectGraph graph, final Map<VarVersionPair, Integer> versions) {
    graph.iterateExprents(exprent -> {
      List<Exprent> lst = exprent.getAllExprents(true);
      lst.add(exprent);

      for (Exprent expr : lst) {
        if (expr instanceof VarExprent) {
          VarExprent var = (VarExprent)expr;
          Integer version = versions.get(new VarVersionPair(var));
          if (version != null) {
            var.setVersion(version);
          }
        }
      }

      return 0;
    });
  }

  private static void eliminateNonJavaTypes(VarTypeProcessor typeProcessor) {
    Map<VarVersionPair, VarType> mapExprentMaxTypes = typeProcessor.getUpperBounds();
    Map<VarVersionPair, VarType> mapExprentMinTypes = typeProcessor.getLowerBounds();

    for (VarVersionPair paar : new ArrayList<>(mapExprentMinTypes.keySet())) {
      VarType type = mapExprentMinTypes.get(paar);
      VarType maxType = mapExprentMaxTypes.get(paar);

      if (type.type == CodeType.BYTECHAR || type.type == CodeType.SHORTCHAR) {
        if (maxType != null && maxType.type == CodeType.CHAR) {
          type = VarType.VARTYPE_CHAR;
        }
        else if (maxType != null && maxType.type == CodeType.INT) {
          // Keep plain istore values wide when no bytecode narrowing conversion was observed.
          type = VarType.VARTYPE_INT;
        }
        else {
          type = type.type == CodeType.BYTECHAR ? VarType.VARTYPE_BYTE : VarType.VARTYPE_SHORT;
        }
        mapExprentMinTypes.put(paar, type);
        //} else if(type.type == CodeType.CHAR && (maxType == null || maxType.type == CodeType.INT)) { // when possible, lift char to int
        //	mapExprentMinTypes.put(paar, VarType.VARTYPE_INT);
      }
      else if (type.type == CodeType.NULL) {
        mapExprentMinTypes.put(paar, typeProcessor.getReferenceTypeForNull(paar));
      }
    }
  }

  private void setNewVarIndices(
    VarTypeProcessor typeProcessor,
    DirectGraph graph,
    VarVersionsProcessor previousVersionsProcessor,
    Set<VarVersionPair> receiverEquivalentVersions
  ) {
    final Map<VarVersionPair, VarType> mapExprentMaxTypes = typeProcessor.getUpperBounds();
    Map<VarVersionPair, VarType> mapExprentMinTypes = typeProcessor.getLowerBounds();
    Map<VarVersionPair, FinalType> mapFinalVars = typeProcessor.getMapFinalVars();

    CounterContainer counters = DecompilerContext.getCounterContainer();

    final Map<VarVersionPair, Integer> mapVarPaar = new HashMap<>();
    Set<VarVersionPair> receiverEquivalentVars = new HashSet<>();
    Map<Integer, VarVersionPair> mapOriginalVarIndices = new HashMap<>();
    mapOriginalVarIndices.putAll(this.mapOriginalVarIndices);

    // map var-version pairs on new var indexes
    List<VarVersionPair> vvps = new ArrayList<>(mapExprentMinTypes.keySet());
    Collections.sort(vvps, (o1, o2) -> o1.var != o2.var ?  o1.var - o2.var : o1.version - o2.version);

    VarProcessor varProcessor = DecompilerContext.getVarProcessor();

    for (VarVersionPair pair : vvps) {

      // '>= 0' captures all real variables, as constants are set to version -1
      if (pair.version >= 0) {
        // Some decompiler-inserted mutable locals must remain one Java local after SSA.
        // Otherwise their generated reads can be detached from the writes that guard them.
        boolean pinnedSyntheticLocal = varProcessor != null && varProcessor.isSyntheticLocalPinned(pair.var);
        int newIndex = pair.version == 1 || pinnedSyntheticLocal ? pair.var : counters.getCounterAndIncrement(CounterContainer.VAR_COUNTER);

        VarVersionPair newVar = new VarVersionPair(newIndex, 0);

        mapExprentMinTypes.put(newVar, mapExprentMinTypes.get(pair));
        mapExprentMaxTypes.put(newVar, mapExprentMaxTypes.get(pair));

        if (mapFinalVars.containsKey(pair)) {
          mapFinalVars.put(newVar, mapFinalVars.remove(pair));
        }

        mapVarPaar.put(pair, newIndex);
        mapOriginalVarIndices.put(newIndex, pair);
        if (receiverEquivalentVersions.contains(pair)) {
          receiverEquivalentVars.add(newVar);
        }
      }
    }

    // set new vars
    graph.iterateExprents(exprent -> {
      List<Exprent> lst = exprent.getAllExprents(true);
      lst.add(exprent);

      for (Exprent expr : lst) {
        if (expr instanceof VarExprent) {
          VarExprent newVar = (VarExprent)expr;
          Integer newVarIndex = mapVarPaar.get(new VarVersionPair(newVar));
          if (newVarIndex != null) {
            newVar.setIndex(newVarIndex);
            newVar.setVersion(0);
          }
        }
        else if (expr instanceof ConstExprent) {
          VarType maxType = mapExprentMaxTypes.get(new VarVersionPair(expr.id, -1));
          if (maxType != null) {
            ((ConstExprent)expr).setConstType(maxType);
          }
        }
      }

      return 0;
    });

    if (previousVersionsProcessor != null) {
      Map<Integer, VarVersionPair> oldIndices = previousVersionsProcessor.getMapOriginalVarIndices();
      this.mapOriginalVarIndices = new HashMap<>(mapOriginalVarIndices.size());
      for (Entry<Integer, VarVersionPair> entry : mapOriginalVarIndices.entrySet()) {
        VarVersionPair value = entry.getValue();
        VarVersionPair oldValue = oldIndices.get(value.var);
        value = oldValue != null ? oldValue : value;
        this.mapOriginalVarIndices.put(entry.getKey(), value);
      }
    }
    else {
      this.mapOriginalVarIndices = mapOriginalVarIndices;
    }
    this.receiverEquivalentVars = Set.copyOf(receiverEquivalentVars);
  }

  public VarType getVarType(VarVersionPair pair) {
    return typeProcessor.getVarType(pair);
  }

  public void setVarType(VarVersionPair pair, VarType type) {
    typeProcessor.setVarType(pair, type);
  }

  public FinalType getVarFinal(VarVersionPair pair) {
    FinalType fin = typeProcessor.getMapFinalVars().get(pair);
    return fin == null ? FinalType.FINAL : fin;
  }

  public void setVarFinal(VarVersionPair pair, FinalType finalType) {
    typeProcessor.getMapFinalVars().put(pair, finalType);
  }

  public Map<Integer, VarVersionPair> getMapOriginalVarIndices() {
    return mapOriginalVarIndices;
  }

  public VarTypeProcessor getTypeProcessor() {
    return typeProcessor;
  }

  public boolean isReceiverEquivalent(VarVersionPair pair) {
    return receiverEquivalentVars.contains(pair);
  }
}
