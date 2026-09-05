package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.RecordLayout;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/** Proves record offsets without assuming that JVM index arithmetic cannot overflow. */
final class SemanticRecordAccess {
  private SemanticRecordAccess() {}

  static Integer slot(Exprent index, RecordLayout layout, SemanticContext context) {
    Long constant = SemanticContext.integral(index);
    if (constant != null) return slot(constant, layout);
    SemanticContext.Range range = context.range(index);
    long min = Math.max(0, range.min()); // A completed array access has a nonnegative index.
    if (range.max() < min) return null;
    if (layout.planes()) {
      if (min < layout.offset()) return null;
      long first = (min - layout.offset()) / layout.stride();
      long last = (range.max() - layout.offset()) / layout.stride();
      return first == last ? (int)first : null;
    }
    Integer residue = context.loopResidue(index, layout.stride());
    if (residue == null) residue = residue(index, layout.stride(), context, new IdentityHashMap<>());
    if (residue == null) return null;
    long first = min + Math.floorMod(residue - min, layout.stride());
    // A residue may also address the header after overflow. Only use the
    // record contract if no successful index in the proven range can do that.
    if (first < layout.offset() || first > range.max()) return null;
    return Math.floorMod(residue - layout.offset(), layout.stride());
  }

  static Integer slot(long index, RecordLayout layout) {
    if (index < layout.offset()) return null;
    return (int)(layout.planes() ? (index - layout.offset()) / layout.stride() : (index - layout.offset()) % layout.stride());
  }

  static ConstExprent offsetLiteral(Exprent index, RecordLayout layout, int slot) {
    if (layout.planes()) return null;
    Long constant = SemanticContext.integral(index);
    if (layout.offset() == 0 && constant != null && constant == slot) return (ConstExprent)index;
    if (index instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.ADD) {
      Exprent right = function.getLstOperands().get(1);
      constant = SemanticContext.integral(right);
      if (right instanceof ConstExprent value && constant != null && constant == slot) return value;
    }
    return null;
  }

  private static Integer residue(Exprent expression, int stride, SemanticContext context, Map<Exprent, Integer> memo) {
    if (memo.containsKey(expression)) return memo.get(expression);
    // Mark an in-progress node unknown so cyclic assignments cannot prove their
    // own alignment. Every reaching definition must independently agree.
    memo.put(expression, null);
    Integer result = expressionResidue(expression, stride, context, memo);
    memo.put(expression, result);
    return result;
  }

  private static Integer expressionResidue(Exprent expression, int stride, SemanticContext context, Map<Exprent, Integer> memo) {
    Long constant = SemanticContext.integral(expression);
    if (constant != null) return Math.floorMod(constant, stride);
    List<Exprent> sources = context.definitions(expression);
    if (sources != null && !sources.isEmpty()) {
      Integer common = null;
      for (Exprent source : sources) {
        Integer candidate = residue(source, stride, context, memo);
        if (candidate == null || common != null && !common.equals(candidate)) return null;
        common = candidate;
      }
      return common;
    }
    if (!(expression instanceof FunctionExprent function) || function.getLstOperands().size() != 2) return null;
    // Powers of two divide the JVM int overflow modulus. Other strides need
    // a separate proof that this operation cannot wrap.
    boolean powerOfTwo = Integer.bitCount(stride) == 1;
    if (!powerOfTwo && SemanticContext.arithmeticRange(function.getFuncType(),
        context.range(function.getLstOperands().get(0)), context.range(function.getLstOperands().get(1))) == null) return null;
    Integer left = residue(function.getLstOperands().get(0), stride, context, memo);
    Integer right = residue(function.getLstOperands().get(1), stride, context, memo);
    return switch (function.getFuncType()) {
      case ADD -> left == null || right == null ? null : Math.floorMod((long)left + right, stride);
      case SUB -> left == null || right == null ? null : Math.floorMod((long)left - right, stride);
      case MUL -> Integer.valueOf(0).equals(left) || Integer.valueOf(0).equals(right) ? 0
        : left == null || right == null ? null : Math.floorMod((long)left * right, stride);
      case AND -> !powerOfTwo ? null : Integer.valueOf(0).equals(left) || Integer.valueOf(0).equals(right) ? 0
        : left == null || right == null ? null : left & right;
      case SHL -> {
        Long shift = SemanticContext.integral(function.getLstOperands().get(1));
        if (shift == null) yield null;
        long factor = 1L << (shift.intValue() & 31);
        yield factor % stride == 0 ? 0 : left == null ? null : Math.floorMod(left * factor, stride);
      }
      default -> null;
    };
  }

}
