package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.code.Instruction;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.CounterContainer;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;

import java.util.*;
import java.util.function.Consumer;

// Turns all SSA assigned variables with versions > 1 into new variable indices.
// Basically what VarVersionsProcessor does but much simpler.
public final class SimpleSSAReassign {
  public static Map<Instruction, Integer> reassignSSAForm(SSAConstructorSparseEx ssa, RootStatement root) {
    List<VarExprent> variables = new ArrayList<>();
    findAllVars(root, variables::add);
    Map<VarVersionPair, Integer> representatives = ssa.getPhiComponents().representatives();
    Map<VarVersionPair, Integer> newIndices = new HashMap<>();
    int nextIndex = DecompilerContext.getCounterContainer().getCounter(CounterContainer.VAR_COUNTER);
    for (VarExprent variable : variables) {
      VarVersionPair pair = variable.getVarVersionPair();
      int version = representatives.getOrDefault(pair, pair.version);
      if (pair.var < VarExprent.STACK_BASE && version > 1) {
        VarVersionPair representative = new VarVersionPair(pair.var, version);
        if (!newIndices.containsKey(representative)) {
          newIndices.put(representative, ++nextIndex);
        }
      }
    }

    Map<Instruction, Integer> rewriteMap = new HashMap<>();
    for (VarExprent variable : variables) {
      VarVersionPair pair = variable.getVarVersionPair();
      Integer index = newIndices.get(new VarVersionPair(pair.var, representatives.getOrDefault(pair, pair.version)));
      if (index != null) {
        variable.setIndex(index);
        variable.setVersion(1);
        // Leave original instructions intact; finally analysis applies this map
        // to its private bytecode copy.
        if (variable.getBackingInstr() != null) {
          rewriteMap.put(variable.getBackingInstr(), index);
        }
      }
    }

    return rewriteMap;
  }

  private static void findAllVars(Statement stat, Consumer<VarExprent> action) {
    if (stat.getExprents() == null) {
      for (Statement st : stat.getStats()) {
        findAllVars(st, action);
      }

      for (Exprent exprent : stat.getStatExprents()) {
        findAllVars(exprent, action);
      }
    }
    else {
      for (Exprent exprent : stat.getExprents()) {
        findAllVars(exprent, action);
      }
    }
  }

  private static void findAllVars(Exprent exprent, Consumer<VarExprent> action) {
    List<Exprent> lst = exprent.getAllExprents(true, true);

    for (Exprent expr : lst) {
      if (expr instanceof VarExprent) {
        action.accept((VarExprent)expr);
      }
    }
  }
}
