package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;

/** A constant-step for-loop counter; no general loop solving or heap assumptions. */
record SemanticLoopIndex(SemanticContext.Key variable, long start, int step, int storageBits, long minimum, long maximum, boolean noWrap, boolean checkedEntry) {
  record Offset(VarExprent variable, long delta) {}

  static SemanticLoopIndex analyze(DoStatement loop, SemanticContext context) {
    if (loop.getLooptype() != DoStatement.Type.FOR
        || !(loop.getInitExprent() instanceof AssignmentExprent init)
        || init.getCondType() != null || !(init.getLeft() instanceof VarExprent variable)) return null;
    // Java compound updates narrow back to byte/short/char storage. An int
    // overflow proof alone would wrongly accept byte counters crossing 127 or
    // a descending char counter wrapping from zero to 65535.
    int bits = switch (variable.getExprType().type) {
      case BYTE, BYTECHAR -> 8;
      case SHORT, CHAR, SHORTCHAR -> 16;
      case INT -> 32;
      default -> 0;
    };
    if (bits == 0) return null;
    boolean unsigned = variable.getExprType().equals(org.jetbrains.java.decompiler.struct.gen.VarType.VARTYPE_CHAR);
    long storageMin = unsigned ? 0 : -(1L << (bits - 1));
    long storageMax = unsigned ? 65535 : (1L << (bits - 1)) - 1;
    Long start = SemanticContext.integral(init.getRight());
    if (start == null || start < 0 || start > storageMax) return null;
    Long step = step(loop.getIncExprent(), variable);
    if (step == null || step == 0 || step < Integer.MIN_VALUE || step > Integer.MAX_VALUE || writes(loop.getFirst(), variable.getIndex())) return null;
    if (!(loop.getConditionExprent() instanceof FunctionExprent condition) || condition.getLstOperands().size() != 2) return null;
    Exprent bound = condition.getLstOperands().get(1);
    FunctionExprent.FunctionType comparison = condition.getFuncType();
    if (!same(condition.getLstOperands().get(0), variable)) {
      if (!same(bound, variable)) return null;
      bound = condition.getLstOperands().get(0);
      comparison = switch (comparison) {
        case LT -> FunctionExprent.FunctionType.GT; case LE -> FunctionExprent.FunctionType.GE;
        case GT -> FunctionExprent.FunctionType.LT; case GE -> FunctionExprent.FunctionType.LE;
        default -> comparison;
      };
    }
    if (step < 0) {
      long minimum = context.range(bound).min();
      switch (comparison) {
        case GT -> minimum++;
        case GE -> { }
        default -> { return null; }
      }
      if (minimum < 0 || minimum > start) return null;
      minimum = start - Math.floorDiv(start - minimum, -step) * -step;
      return new SemanticLoopIndex(SemanticContext.variable(variable.getIndex(), variable.getVersion()), start, step.intValue(), bits, minimum, start,
        minimum + step >= storageMin, false);
    }
    long maximum = context.range(bound).max();
    switch (comparison) {
      case LT -> maximum--;
      case LE -> { }
      default -> { return null; }
    }
    maximum = Math.min(maximum, storageMax);
    if (maximum < start) return null;
    maximum = alignedMaximum(start, step, maximum);
    boolean noWrap = maximum + step <= storageMax;
    if (!noWrap && bits == 32) {
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
    return new SemanticLoopIndex(SemanticContext.variable(variable.getIndex(), variable.getVersion()), start, step.intValue(), bits, start, maximum, noWrap,
      !unsigned && (bits == 32 || variable.getExprType().equals(org.jetbrains.java.decompiler.struct.gen.VarType.VARTYPE_BYTE)
        || variable.getExprType().equals(org.jetbrains.java.decompiler.struct.gen.VarType.VARTYPE_SHORT))
        && step <= storageMax + 1 && checkedEntry(loop.getFirst(), variable));
  }

  boolean wrapPreservesResidue(int stride) {
    return Integer.bitCount(stride) == 1 && (1L << storageBits) % stride == 0;
  }

  private enum Prefix { EMPTY, CHECKED, BLOCKED }

  /**
   * A positive increment can wrap a nonnegative int only to a negative int.
   * If every iteration first performs a[i], that next iteration must throw
   * before any later array access completes. Its completed record accesses
   * therefore retain the initial residue even when the increment can wrap.
   * Do not promote this proof to a numeric range for arbitrary expressions.
   */
  private static boolean checkedEntry(Statement statement, VarExprent variable) {
    return prefix(statement, variable) == Prefix.CHECKED;
  }

  private static Prefix prefix(Statement statement, VarExprent variable) {
    if (statement.getExprents() != null) {
      for (Exprent expression : statement.getExprents()) {
        Prefix result = prefix(expression, variable);
        if (result != Prefix.EMPTY) return result;
      }
      return Prefix.EMPTY;
    }
    if (statement instanceof SequenceStatement) {
      for (Statement child : statement.getStats()) {
        Prefix result = prefix(child, variable);
        if (result != Prefix.EMPTY) return result;
      }
      return Prefix.EMPTY;
    }
    if (statement instanceof IfStatement conditional) {
      Prefix result = prefix(conditional.getFirst(), variable);
      if (result != Prefix.EMPTY) return result;
      result = prefix(conditional.getHeadexprent().getCondition(), variable);
      // No facts from optional branches or caught exceptions may justify a
      // later access, and a branch can bypass the rest of the iteration.
      return result == Prefix.CHECKED ? result : Prefix.BLOCKED;
    }
    if (statement instanceof SwitchStatement selection) {
      Prefix result = prefix(selection.getFirst(), variable);
      if (result != Prefix.EMPTY) return result;
      result = prefix(selection.getHeadexprent(), variable);
      return result == Prefix.CHECKED ? result : Prefix.BLOCKED;
    }
    return Prefix.BLOCKED;
  }

  private static Prefix prefix(Exprent expression, VarExprent variable) {
    if (expression instanceof FunctionExprent function && switch (function.getFuncType()) {
      case TERNARY, BOOLEAN_AND, BOOLEAN_OR -> true; default -> false;
    }) {
      Prefix result = prefix(function.getLstOperands().get(0), variable);
      return result == Prefix.CHECKED ? result : Prefix.BLOCKED;
    }
    for (Exprent child : expression.getAllExprents()) {
      Prefix result = prefix(child, variable);
      if (result != Prefix.EMPTY) return result;
    }
    if (expression instanceof ArrayExprent array) {
      return same(array.getIndex(), variable) ? Prefix.CHECKED : Prefix.BLOCKED;
    }
    if (expression instanceof ExitExprent || expression instanceof InvocationExprent) return Prefix.BLOCKED;
    return Prefix.EMPTY;
  }

  private static long alignedMaximum(long start, long step, long upper) {
    return start + Math.floorDiv(upper - start, step) * step;
  }

  private static boolean same(Exprent expression, VarExprent variable) {
    return expression instanceof VarExprent other && other.getVarVersionPair().equals(variable.getVarVersionPair());
  }

  private static Long step(Exprent expression, VarExprent variable) {
    if (expression instanceof FunctionExprent function && same(function.getLstOperands().get(0), variable)) {
      if (function.getFuncType() == FunctionExprent.FunctionType.IPP || function.getFuncType() == FunctionExprent.FunctionType.PPI) return 1L;
      if (function.getFuncType() == FunctionExprent.FunctionType.IMM || function.getFuncType() == FunctionExprent.FunctionType.MMI) return -1L;
    }
    if (!(expression instanceof AssignmentExprent assignment) || !same(assignment.getLeft(), variable)) return null;
    if (assignment.getCondType() == FunctionExprent.FunctionType.ADD) return SemanticContext.integral(assignment.getRight());
    if (assignment.getCondType() == FunctionExprent.FunctionType.SUB) {
      Long amount = SemanticContext.integral(assignment.getRight());
      return amount == null ? null : -amount;
    }
    if (assignment.getCondType() == null && assignment.getRight() instanceof FunctionExprent function
        && (function.getFuncType() == FunctionExprent.FunctionType.ADD || function.getFuncType() == FunctionExprent.FunctionType.SUB)
        && same(function.getLstOperands().get(0), variable)) {
      Long amount = SemanticContext.integral(function.getLstOperands().get(1));
      return amount == null ? null : function.getFuncType() == FunctionExprent.FunctionType.SUB ? -amount : amount;
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
