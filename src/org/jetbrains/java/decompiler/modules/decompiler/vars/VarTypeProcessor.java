// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.TypeFamily;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedList;
import java.util.List;
import java.util.Map;
import java.util.Set;

public class VarTypeProcessor {
  public enum FinalType {
    NON_FINAL, EXPLICIT_FINAL, FINAL
  }

  private TypeInferenceWorklist worklist;
  private final StructMethod method;
  private final MethodDescriptor methodDescriptor;
  private final Map<VarVersionPair, VarType> lowerBounds = new HashMap<>();
  private final Map<VarVersionPair, VarType> upperBounds = new HashMap<>();
  private final Set<VarVersionPair> stackMapVars = new HashSet<>();
  private final Map<VarVersionPair, FinalType> mapFinalVars = new HashMap<>();

  public VarTypeProcessor(StructMethod mt, MethodDescriptor md) {
    method = mt;
    methodDescriptor = md;
  }

  public void calculateVarTypes(RootStatement root, DirectGraph graph) {
    setInitVars(root, graph);

    resetExprentTypes(graph);

    List<Exprent> expressions = new ArrayList<>();
    graph.iterateExprents(exprent -> {
      expressions.addAll(exprent.getAllExprents(true));
      expressions.add(exprent);
      return 0;
    });
    inferTypes(expressions);

    for (VarVersionPair p : lowerBounds.keySet()) {
      VarType lower = lowerBounds.get(p);
      VarType upper = upperBounds.get(p);

      if (upper != null) {
        if (lower.typeFamily != TypeFamily.OBJECT && lower.higherInLatticeThan(upper)) {
          ValidationHelper.validateTrue(false, "lower bound " + lower + " > upper bound " + upper + " for var " + p);
        }
      }
    }

    ValidationHelper.validateVars(graph, root, var -> var.getVarType() != VarType.VARTYPE_UNKNOWN, "Var type not set!");
  }

  private void setInitVars(RootStatement root, DirectGraph graph) {
    boolean thisVar = !method.hasModifier(CodeConstants.ACC_STATIC);

    MethodDescriptor md = methodDescriptor;

    if (thisVar) {
      StructClass cl = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS);
      ValidationHelper.assertTrue(cl != null, "Current class name should not be null");
      VarType clType = new VarType(CodeType.OBJECT, 0, cl.qualifiedName);
      lowerBounds.put(new VarVersionPair(0, 1), clType);
      upperBounds.put(new VarVersionPair(0, 1), clType);
    }

    int varIndex = 0;
    for (int i = 0; i < md.params.length; i++) {
      lowerBounds.put(new VarVersionPair(varIndex + (thisVar ? 1 : 0), 1), md.params[i]);
      upperBounds.put(new VarVersionPair(varIndex + (thisVar ? 1 : 0), 1), md.params[i]);
      varIndex += md.params[i].stackSize;
    }

    // Implicitly defined variables
    LinkedList<Statement> stack = new LinkedList<>();
    stack.add(root);

    while (!stack.isEmpty()) {
      Statement stat = stack.removeFirst();

      List<VarExprent> vars = stat.getImplicitlyDefinedVars();

      if (vars != null) {
        for (VarExprent var : vars) {
          lowerBounds.put(new VarVersionPair(var.getIndex(), 1), var.getVarType());
          // Avoid forcing non-catch implicit definitions to BOTTOM too early.
          if (stat instanceof CatchStatement || stat instanceof CatchAllStatement) {
            upperBounds.put(new VarVersionPair(var.getIndex(), 1), var.getVarType());
          }
        }
      }

      stack.addAll(stat.getStats());
    }

    applyLegacyStackMapTypes(graph);
  }

  private void applyLegacyStackMapTypes(DirectGraph graph) {
    Map<VarVersionPair, Set<VarType>> stackMapTypes = new HashMap<>();
    graph.iterateExprents(exprent -> {
      List<Exprent> exprents = exprent.getAllExprents(true);
      exprents.add(exprent);

      for (Exprent expr : exprents) {
        if (expr instanceof VarExprent var && var.getVersion() > 0 && !var.getStackMapTypes().isEmpty()) {
          stackMapTypes.computeIfAbsent(new VarVersionPair(var), k -> new HashSet<>()).addAll(var.getStackMapTypes());
        }
      }

      return 0;
    });

    for (Map.Entry<VarVersionPair, Set<VarType>> entry : stackMapTypes.entrySet()) {
      VarType inferredType = mergeObservedStackMapTypes(entry.getValue());
      if (inferredType != null) {
        applyInitialBound(entry.getKey(), inferredType);
      }
    }
  }

  private static VarType mergeObservedStackMapTypes(Set<VarType> observed) {
    VarType merged = null;
    for (VarType type : observed) {
      merged = merged == null ? type : VarType.join(merged, type);
      if (merged == null) {
        return null;
      }
    }
    return merged;
  }

  private void applyInitialBound(VarVersionPair pair, VarType inferredType) {
    VarType oldUpper = upperBounds.get(pair);
    VarType newUpper = oldUpper == null ? inferredType : VarType.meet(oldUpper, inferredType);
    if (newUpper == null) {
      return;
    }

    upperBounds.put(pair, newUpper);
    stackMapVars.add(pair);
  }

  public VarType getReferenceTypeForNull(VarVersionPair pair) {
    VarType upper = upperBounds.get(pair);
    // A null-only definition has no concrete lower bound. Preserve a frame's usable
    // reference constraint instead of erasing it to Object. Conflicting uses meet at
    // NULL, in which case each use still needs its own cast.
    return stackMapVars.contains(pair) && upper != null && upper.typeFamily == TypeFamily.OBJECT && upper.type != CodeType.NULL
      ? upper : VarType.VARTYPE_OBJECT;
  }

  // The analysis should start on a blank slate, with the types set to the bottom type
  private static void resetExprentTypes(DirectGraph graph) {
    graph.iterateExprents(exprent -> {
      List<Exprent> lst = exprent.getAllExprents(true);
      lst.add(exprent);

      for (Exprent expr : lst) {
        if (expr instanceof VarExprent ve) {
          ve.setVarType(VarType.VARTYPE_UNKNOWN);
        } else if (expr instanceof ConstExprent constExpr) {
          if (constExpr.getConstType().typeFamily == TypeFamily.INTEGER) {
            constExpr.setConstType(ConstExprent.guessIntType(constExpr.getIntValue(), constExpr.isBoolPermitted()));
          }
        }
      }
      return 0;
    });
  }

  void inferTypes(List<Exprent> expressions) {
    worklist = new TypeInferenceWorklist(expressions);
    try {
      int restarts = 0;
      for (int index; (index = worklist.next()) >= 0;) {
        boolean evaluate = worklist.needsEvaluation(index);
        CheckTypesResult result;
        if (evaluate) {
          Exprent expression = expressions.get(index);
          if (expression instanceof ConstExprent constant && constant.getConstType().typeFamily.intOrBool()) {
            lowerBounds.putIfAbsent(new VarVersionPair(constant.id, -1), constant.getConstType());
          }
          result = expression.checkExprTypeBounds();
          worklist.cache(index, result);
        } else {
          result = worklist.constraints(index);
        }
        if (result == null) continue;
        for (CheckTypesResult.ExprentTypePair entry : result.getUpperBounds()) {
          changeExprentType(entry.exprent, entry.type, Bound.UPPER);
        }
        if (evaluate) {
          boolean stable = true;
          for (CheckTypesResult.ExprentTypePair entry : result.getLowerBounds()) {
            stable &= changeExprentType(entry.exprent, entry.type, Bound.LOWER);
          }
          if (!stable) {
            if (++restarts > 10_000) {
              throw new IllegalStateException("Variable type inference did not converge: lower=" + lowerBounds + ", upper=" + upperBounds);
            }
            worklist.restartUpperBounds();
          }
        }
      }
    } finally {
      worklist = null;
    }
  }

  private enum Bound {
    LOWER,
    UPPER
  }

  // False reports lower-bound progress; upper-bound updates do not trigger inference.
  private boolean changeExprentType(Exprent exprent, VarType newType, Bound bound) {
    ValidationHelper.assertTrue(newType != null, "Null type passed to CheckTypesResult!");

    switch (exprent.type) {
      case CONST:
        ConstExprent constExpr = (ConstExprent)exprent;
        VarType constType = constExpr.getConstType();

        if (!newType.typeFamily.intOrBool() || !constType.typeFamily.intOrBool()) {
          return true;
        } else if (newType.typeFamily == TypeFamily.INTEGER) {
          VarType minInteger = ConstExprent.guessIntType(constExpr.getIntValue(), false);
          if (minInteger.higherInLatticeThan(newType)) {
            newType = minInteger;
          }
        }

        return changeVarExprentType(exprent, newType, bound, new VarVersionPair(exprent.id, -1));
      case VAR:
        return changeVarExprentType(exprent, newType, bound, new VarVersionPair((VarExprent) exprent));

      case ASSIGNMENT:
        // TODO: do we need to change the left too?
        return changeExprentType(((AssignmentExprent)exprent).getRight(), newType, bound);

      case FUNCTION:
        return changeFunctionExprentType(newType, bound, (FunctionExprent)exprent);

    }

    return true;
  }

  private boolean changeVarExprentType(Exprent exprent, VarType newType, Bound bound, VarVersionPair pair) {
    // For variables, we want to make sure that the lower bound rises and the upper bound falls.

    if (bound == Bound.LOWER) {
      // Attempt to raise the lower bound of the variable

      VarType currentMinType = lowerBounds.get(pair);
      VarType newMinType;
      if (currentMinType == null || newType.typeFamily.isGreater(currentMinType.typeFamily)) {
        newMinType = newType; // No recorded type or the new type has a higher family? The new type is just the incoming one.
      } else if (newType.typeFamily.isLesser(currentMinType.typeFamily)) {
        // Going backwards? Early out.
        return true;
      } else {
        // Already have a type? Find the higher of the two; the lower bound rises.
        newMinType = VarType.join(currentMinType, newType);
      }
      ValidationHelper.assertTrue(newMinType != null, "Trying to raise the minimum type of disjoint variables!");

      lowerBounds.put(pair, newMinType);
      if (exprent instanceof ConstExprent) {
        ((ConstExprent) exprent).setConstType(newMinType);
      }

      // Graph traversal is not definition-ordered, especially for assignments nested in a combined condition. Learning
      // the first bound can therefore unblock a variable visited earlier in this pass just as raising an existing bound can.
      if (currentMinType == null || newMinType.typeFamily.isGreater(currentMinType.typeFamily) || newMinType.higherInLatticeThan(currentMinType)) {
        worklist.changed(pair);
        return false;
      }
    } else {  // max
      VarType currentMaxType = upperBounds.get(pair);
      VarType newMaxType;
      if (currentMaxType == null || newType.typeFamily.isLesser(currentMaxType.typeFamily)) {
        newMaxType = newType;
      } else if (newType.typeFamily.isGreater(currentMaxType.typeFamily)) {
        // TODO: this return seems to do nothing?
//        return true;
        newMaxType = newType;
      } else {
        // Already have a type? Find the lower of the two; the upper bound falls.
        newMaxType = VarType.meet(currentMaxType, newType);
      }
      ValidationHelper.assertTrue(newMaxType != null, "Trying to lower the maximum type of disjoint variables!");

      upperBounds.put(pair, newMaxType);
    }
    return true;
  }

  private boolean changeFunctionExprentType(VarType newType, Bound bound, FunctionExprent func) {
    int offset = 0;
    switch (func.getFuncType()) {
      case TERNARY:   // FIXME:
        offset++;
      case AND:
      case OR:
      case XOR:
        return changeExprentType(func.getLstOperands().get(offset), newType, bound) &
               changeExprentType(func.getLstOperands().get(offset + 1), newType, bound);
    }
    return true;
  }


  public Map<VarVersionPair, VarType> getUpperBounds() {
    return upperBounds;
  }

  public Map<VarVersionPair, VarType> getLowerBounds() {
    return lowerBounds;
  }

  public Map<VarVersionPair, FinalType> getMapFinalVars() {
    return mapFinalVars;
  }

  public void setVarType(VarVersionPair pair, VarType type) {
    lowerBounds.put(pair, type);
  }

  public VarType getVarType(VarVersionPair pair) {
    return lowerBounds.get(pair);
  }
}
