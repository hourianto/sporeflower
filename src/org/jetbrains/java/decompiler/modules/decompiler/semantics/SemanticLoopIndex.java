package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;

/** A constant-step for-loop counter; no general loop solving or heap assumptions. */
record SemanticLoopIndex(SemanticContext.Key variable, long start, int step, long maximum, boolean noWrap) {
  record Offset(VarExprent variable, long delta) {}

  static SemanticLoopIndex analyze(DoStatement loop, SemanticContext context) {
    if (loop.getLooptype() != DoStatement.Type.FOR
        || !(loop.getInitExprent() instanceof AssignmentExprent init)
        || init.getCondType() != null || !(init.getLeft() instanceof VarExprent variable)) return null;
    Long start = SemanticContext.integral(init.getRight());
    if (start == null || start < 0 || start > Integer.MAX_VALUE) return null;
    Long step = step(loop.getIncExprent(), variable);
    if (step == null || step <= 1 || step > Integer.MAX_VALUE || writes(loop.getFirst(), variable.getIndex())) return null;
    if (!(loop.getConditionExprent() instanceof FunctionExprent condition) || condition.getLstOperands().size() != 2
        || !same(condition.getLstOperands().get(0), variable)) return null;
    long maximum = context.range(condition.getLstOperands().get(1)).max();
    switch (condition.getFuncType()) {
      case LT -> maximum--;
      case LE -> { }
      default -> { return null; }
    }
    if (maximum < start || maximum > Integer.MAX_VALUE) return null;
    maximum = alignedMaximum(start, step, maximum);
    boolean noWrap = maximum + step <= Integer.MAX_VALUE;
    if (!noWrap) {
      // A mandatory array access before the increment can rule out the last
      // overflowing iteration: a[i+k] must fit below the maximum array length.
      // Only inspect the entry block, so a continue or conditional access cannot
      // accidentally supply this proof.
      Statement entry = loop.getFirst();
      while (entry instanceof SequenceStatement || entry instanceof IfStatement || entry instanceof SwitchStatement) entry = entry.getFirst();
      if (entry.getExprents() != null) {
        long offset = -1;
        for (Exprent expression : entry.getExprents()) offset = Math.max(offset, mandatoryOffset(expression, variable));
        if (offset >= 0) {
          long beforeIncrement = alignedMaximum(start, step, Math.min(maximum, Integer.MAX_VALUE - 1L - offset));
          noWrap = beforeIncrement + step <= Integer.MAX_VALUE;
        }
      }
    }
    return new SemanticLoopIndex(SemanticContext.variable(variable.getIndex(), variable.getVersion()), start, step.intValue(), maximum, noWrap);
  }

  private static long alignedMaximum(long start, long step, long upper) {
    return start + Math.floorDiv(upper - start, step) * step;
  }

  private static boolean same(Exprent expression, VarExprent variable) {
    return expression instanceof VarExprent other && other.getVarVersionPair().equals(variable.getVarVersionPair());
  }

  private static Long step(Exprent expression, VarExprent variable) {
    if (!(expression instanceof AssignmentExprent assignment) || !same(assignment.getLeft(), variable)) return null;
    if (assignment.getCondType() == FunctionExprent.FunctionType.ADD) return SemanticContext.integral(assignment.getRight());
    if (assignment.getCondType() == null && assignment.getRight() instanceof FunctionExprent function
        && function.getFuncType() == FunctionExprent.FunctionType.ADD && same(function.getLstOperands().get(0), variable)) {
      return SemanticContext.integral(function.getLstOperands().get(1));
    }
    return null;
  }

  static Offset offset(Exprent expression) {
    if (expression instanceof VarExprent variable) return new Offset(variable, 0);
    if (expression instanceof FunctionExprent function && (function.getFuncType() == FunctionExprent.FunctionType.ADD
        || function.getFuncType() == FunctionExprent.FunctionType.SUB)) {
      Exprent left = function.getLstOperands().get(0), right = function.getLstOperands().get(1);
      Long delta = SemanticContext.integral(right);
      if (left instanceof VarExprent variable && delta != null) return new Offset(variable,
        function.getFuncType() == FunctionExprent.FunctionType.SUB ? -delta : delta);
      delta = SemanticContext.integral(left);
      if (function.getFuncType() == FunctionExprent.FunctionType.ADD && right instanceof VarExprent variable && delta != null) {
        return new Offset(variable, delta);
      }
    }
    return null;
  }

  private static long mandatoryOffset(Exprent expression, VarExprent variable) {
    long result = -1;
    if (expression instanceof ArrayExprent array) {
      Offset offset = offset(array.getIndex());
      if (offset != null && same(offset.variable(), variable) && offset.delta() >= 0 && offset.delta() <= Integer.MAX_VALUE) result = offset.delta();
    }
    if (expression instanceof FunctionExprent function && switch (function.getFuncType()) {
      case TERNARY, BOOLEAN_AND, BOOLEAN_OR -> true; default -> false;
    }) return Math.max(result, mandatoryOffset(function.getLstOperands().get(0), variable));
    for (Exprent child : expression.getAllExprents()) result = Math.max(result, mandatoryOffset(child, variable));
    return result;
  }

  private static boolean writes(Statement statement, int index) {
    for (Exprent expression : statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents()) {
      if (writes(expression, index)) return true;
    }
    for (Statement child : statement.getStats()) if (writes(child, index)) return true;
    return false;
  }

  private static boolean writes(Exprent expression, int index) {
    Exprent target = expression instanceof AssignmentExprent assignment ? assignment.getLeft()
      : expression instanceof FunctionExprent function && switch (function.getFuncType()) {
        case IPP, PPI, IMM, MMI -> true; default -> false;
      } ? ((FunctionExprent)expression).getLstOperands().get(0) : null;
    if (target instanceof VarExprent variable && variable.getIndex() == index) return true;
    for (Exprent child : expression.getAllExprents()) if (writes(child, index)) return true;
    return false;
  }
}
