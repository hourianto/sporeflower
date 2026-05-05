// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdgeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.IfStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.BitSet;
import java.util.Deque;
import java.util.HashSet;
import java.util.LinkedList;
import java.util.List;

public class PPandMMHelper {

  private boolean exprentReplaced;
  private VarProcessor varProc;
  private DirectGraph dgraph;

  public PPandMMHelper(VarProcessor varProc) {
    this.varProc = varProc;
  }

  public boolean findPPandMM(RootStatement root) {

    FlattenStatementsHelper flatthelper = new FlattenStatementsHelper();
    this.dgraph = flatthelper.buildDirectGraph(root);

    LinkedList<DirectNode> stack = new LinkedList<>();
    stack.add(this.dgraph.first);

    HashSet<DirectNode> setVisited = new HashSet<>();

    boolean res = false;

    while (!stack.isEmpty()) {

      DirectNode node = stack.removeFirst();

      if (setVisited.contains(node)) {
        continue;
      }
      setVisited.add(node);

      res |= processExprentList(node.exprents);

      for (DirectEdge suc : node.getSuccessors(DirectEdgeType.REGULAR)) {
        stack.add(suc.getDestination());
      }
    }

    return res;
  }

  private boolean processExprentList(List<Exprent> lst) {

    boolean result = false;

    for (int i = 0; i < lst.size(); i++) {
      Exprent exprent = lst.get(i);
      exprentReplaced = false;

      Exprent retexpr = processExprentRecursive(exprent);
      if (retexpr != null) {
        lst.set(i, retexpr);

        result = true;
        i--; // process the same exprent again
      }

      result |= exprentReplaced;
    }

    return result;
  }

  private Exprent processExprentRecursive(Exprent exprent) {

    boolean replaced = true;
    while (replaced) {
      replaced = false;

      for (Exprent expr : exprent.getAllExprents()) {
        Exprent retexpr = processExprentRecursive(expr);
        if (retexpr != null) {
          exprent.replaceExprent(expr, retexpr);
          retexpr.addBytecodeOffsets(expr.bytecode);
          replaced = true;
          exprentReplaced = true;
          break;
        }
      }
    }

    if (exprent instanceof AssignmentExprent) {
      AssignmentExprent as = (AssignmentExprent)exprent;

      if (as.getRight() instanceof FunctionExprent) {
        FunctionExprent func = (FunctionExprent)as.getRight();

        VarType midlayer = func.getFuncType().castType;
        if (midlayer != null) {
          if (func.getLstOperands().get(0) instanceof FunctionExprent) {
            func = (FunctionExprent)func.getLstOperands().get(0);
          }
          else {
            return null;
          }
        }

        if (func.getFuncType() == FunctionExprent.FunctionType.ADD ||
            func.getFuncType() == FunctionExprent.FunctionType.SUB) {
          Exprent econd = func.getLstOperands().get(0);
          Exprent econst = func.getLstOperands().get(1);

          if (!(econst instanceof ConstExprent) && econd instanceof ConstExprent &&
              func.getFuncType() == FunctionExprent.FunctionType.ADD) {
            econd = econst;
            econst = func.getLstOperands().get(0);
          }

          if (econst instanceof ConstExprent && ((ConstExprent)econst).hasValueOne()) {
            Exprent left = as.getLeft();

            VarType condtype = left.getExprType();
            if (exprsEqual(left, econd) && (midlayer == null || midlayer.equals(condtype))) {
              FunctionExprent ret = new FunctionExprent(
                func.getFuncType() == FunctionExprent.FunctionType.ADD ? FunctionExprent.FunctionType.PPI : FunctionExprent.FunctionType.MMI,
                econd, func.bytecode);
              ret.setImplicitType(condtype);

              exprentReplaced = true;

              if (!left.equals(econd)) {
                updateVersions(this.dgraph, new VarVersionPair((VarExprent)left), new VarVersionPair((VarExprent)econd));
              }

              return ret;
            }
          }
        }
      }
    }

    return null;
  }

  private boolean exprsEqual(Exprent e1, Exprent e2) {
    if (e1 == e2) return true;
    if (e1 == null || e2 == null) return false;
    if (e1 instanceof VarExprent) {
      return varsEqual(e1, e2);
    }
    return e1.equals(e2);
  }

  private boolean varsEqual(Exprent e1, Exprent e2) {
    if (!(e1 instanceof VarExprent)) return false;
    if (!(e2 instanceof VarExprent)) return false;

    VarExprent v1 = (VarExprent)e1;
    VarExprent v2 = (VarExprent)e2;
    return varProc.getVarOriginalIndex(v1.getIndex()) == varProc.getVarOriginalIndex(v2.getIndex());
    // TODO: Verify the types are in the same 'family' {byte->short->int}
    //        && InterpreterUtil.equalObjects(v1.getVarType(), v2.getVarType());
  }


  private void updateVersions(DirectGraph graph, final VarVersionPair oldVVP, final VarVersionPair newVVP) {
    graph.iterateExprents(new DirectGraph.ExprentIterator() {
      @Override
      public int processExprent(Exprent exprent) {
        List<Exprent> lst = exprent.getAllExprents(true);
        lst.add(exprent);

        for (Exprent expr : lst) {
          if (expr instanceof VarExprent) {
            VarExprent var = (VarExprent)expr;
            if (var.getIndex() == oldVVP.var && var.getVersion() == oldVVP.version) {
              var.setIndex(newVVP.var);
              var.setVersion(newVVP.version);
            }
          }
        }

        return 0;
      }
    });
  }

  //
  // ++a
  // (a > 0) {
  //   ...
  // }
  //
  // becomes
  //
  // if (++a > 0) {
  //   ...
  // }
  //
  // Semantically the same, but cleaner and allows for loop inlining
  public static boolean inlinePPIandMMIIf(RootStatement stat) {
    boolean res = inlinePPIandMMIIfRec(stat);

    if (res) {
      SequenceHelper.condenseSequences(stat);
    }

    return res;
  }

  private static boolean inlinePPIandMMIIfRec(Statement stat) {
    boolean res = false;
    for (Statement st : stat.getStats()) {
      res |= inlinePPIandMMIIfRec(st);
    }

    if (stat.getExprents() != null && !stat.getExprents().isEmpty()) {
      IfStatement destination = findIfSuccessor(stat);

      if (destination != null) {
        // Last exprent
        Exprent expr = stat.getExprents().get(stat.getExprents().size() - 1);
        if (expr instanceof FunctionExprent) {
          FunctionExprent func = (FunctionExprent)expr;

          if (func.getFuncType() == FunctionExprent.FunctionType.PPI || func.getFuncType() == FunctionExprent.FunctionType.MMI) {
            Exprent inner = func.getLstOperands().get(0);

            if (inner instanceof VarExprent) {
              Exprent ifExpr = destination.getHeadexprent().getCondition();

              if (ifExpr instanceof FunctionExprent) {
                FunctionExprent ifFunc = (FunctionExprent)ifExpr;

                while (ifFunc.getFuncType() == FunctionExprent.FunctionType.BOOL_NOT) {
                  Exprent innerFunc = ifFunc.getLstOperands().get(0);

                  if (innerFunc instanceof FunctionExprent) {
                    ifFunc = (FunctionExprent)innerFunc;
                  } else {
                    break;
                  }
                }

                InlineTarget target = findInlineTarget(ifFunc, ((VarExprent)inner).getIndex(), false, true, expr);
                if (!target.valid) {
                  return false;
                }

                if (target.var != null) {
                  Deque<Exprent> stack = new LinkedList<>();
                  stack.push(ifFunc);

                  while (!stack.isEmpty()) {
                    Exprent exprent = stack.pop();

                    for (Exprent ex : exprent.getAllExprents()) {
                      if (ex == target.var) {
                        // Replace variable with ppi/mmi
                        exprent.replaceExprent(target.var, expr);
                        expr.addBytecodeOffsets(target.var.bytecode);

                        // No more itr
                        stack.clear();
                        break;
                      }

                      stack.push(ex);
                    }
                  }

                  // Remove old expr
                  stat.getExprents().remove(expr);
                  res = true;

                  destination.setHasPPMM(true);
                }
              }
            }
          }
        }
      }
    }

    return res;
  }

  private static InlineTarget findInlineTarget(Exprent exprent, int varIndex, boolean writeContext, boolean rootCondition, Exprent inlineExpr) {
    if (exprent instanceof VarExprent var && var.getIndex() == varIndex) {
      return writeContext ? InlineTarget.invalid() : InlineTarget.of(var);
    }

    if (exprent instanceof FunctionExprent func) {
      FunctionExprent.FunctionType type = func.getFuncType();
      if (!rootCondition && (type == FunctionExprent.FunctionType.BOOLEAN_AND || type == FunctionExprent.FunctionType.BOOLEAN_OR)) {
        // Cannot yet handle these as we aren't able to decompose a condition into parts that are always run (not short-circuited) and parts that are.
        // FIXME: handle this case
        return InlineTarget.invalid();
      }

      return findInlineTargetInChildren(func.getAllExprents(), varIndex, writeContext || type.isPPMM(), inlineExpr);
    }

    if (exprent instanceof AssignmentExprent assignment) {
      InlineTarget left = findInlineTarget(assignment.getLeft(), varIndex, true, false, inlineExpr);
      if (!left.valid) {
        return left;
      }

      InlineTarget right = findInlineTarget(assignment.getRight(), varIndex, writeContext, false, inlineExpr);
      return left.merge(right);
    }

    return findInlineTargetInChildren(exprent.getAllExprents(), varIndex, writeContext, inlineExpr);
  }

  private static InlineTarget findInlineTargetInChildren(List<Exprent> children, int varIndex, boolean writeContext, Exprent inlineExpr) {
    InlineTarget result = InlineTarget.none();
    boolean unsafeBeforeTarget = false;

    for (Exprent child : children) {
      InlineTarget childTarget = findInlineTarget(child, varIndex, writeContext, false, inlineExpr);
      if (!childTarget.valid || (unsafeBeforeTarget && childTarget.var != null)) {
        return InlineTarget.invalid();
      }

      result = result.merge(childTarget);
      if (!result.valid) {
        return result;
      }

      if (result.var == null && !isEvaluatedBefore(child, inlineExpr)) {
        unsafeBeforeTarget = true;
      }
    }

    return result;
  }

  private static boolean isEvaluatedBefore(Exprent exprent, Exprent reference) {
    BitSet exprentRange = new BitSet();
    exprent.getBytecodeRange(exprentRange);

    BitSet referenceRange = new BitSet();
    reference.getBytecodeRange(referenceRange);

    int exprentLast = exprentRange.length() - 1;
    int referenceFirst = referenceRange.nextSetBit(0);

    if (exprentLast < 0 || referenceFirst < 0) {
      return exprent instanceof ConstExprent || exprent instanceof VarExprent;
    }

    return exprentLast < referenceFirst;
  }

  private static final class InlineTarget {
    private static final InlineTarget NONE = new InlineTarget(null, true);
    private static final InlineTarget INVALID = new InlineTarget(null, false);

    final VarExprent var;
    final boolean valid;

    private InlineTarget(VarExprent var, boolean valid) {
      this.var = var;
      this.valid = valid;
    }

    static InlineTarget none() {
      return NONE;
    }

    static InlineTarget invalid() {
      return INVALID;
    }

    static InlineTarget of(VarExprent var) {
      return new InlineTarget(var, true);
    }

    InlineTarget merge(InlineTarget other) {
      if (!this.valid || !other.valid) {
        return INVALID;
      }

      if (this.var == null) {
        return other;
      }

      if (other.var == null) {
        return this;
      }

      return INVALID;
    }
  }

  private static IfStatement findIfSuccessor(Statement stat) {
    if (stat.getParent() instanceof IfStatement) {
      if (stat.getParent().getFirst() == stat) {
        return (IfStatement) stat.getParent();
      }
    }

    return null;
  }
}
