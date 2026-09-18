package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.List;

import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.BitFieldPart.*;

/** Matches packed-field reads and writes, keeping signedness and JVM shift widths explicit. */
final class SemanticBitAccess {
  record Operand(ConstExprent constant, SemanticMappings.BitFieldPart part) {}
  record Packing(Exprent value, List<Operand> operands) {}
  private record Mask(Exprent value, ConstExprent constant) {}
  record Extraction(Exprent source, int shift, int bits, boolean signed, List<Operand> operands) {
    Extraction(Exprent source, int shift, int bits, boolean signed) {
      this(source, shift, bits, signed, List.of());
    }
  }
  private SemanticBitAccess() {}

  static Extraction extraction(Exprent expression) {
    // Match compound masks without rewriting the assignment or reevaluating its target.
    if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == FunctionExprent.FunctionType.AND) {
      Long mask = literal(assignment.getRight());
      VarType type = assignment.getCompoundOperationType() == null ? assignment.getLeft().getExprType() : assignment.getCompoundOperationType();
      int bits = mask == null ? -1 : lowMaskBits(mask, type.equals(VarType.VARTYPE_LONG) ? 64 : 32);
      return bits <= 0 ? null : new Extraction(assignment.getLeft(), 0, bits, false,
        List.of(new Operand((ConstExprent)assignment.getRight(), MASK)));
    }
    if (!(expression instanceof FunctionExprent function)) return null;
    List<Exprent> operands = function.getLstOperands();
    int width = function.getExprType().equals(VarType.VARTYPE_LONG) ? 64 : 32;
    if (function.getFuncType() == FunctionExprent.FunctionType.I2B || function.getFuncType() == FunctionExprent.FunctionType.I2S
        || function.getFuncType() == FunctionExprent.FunctionType.I2C) {
      int bits = function.getFuncType() == FunctionExprent.FunctionType.I2B ? 8 : 16;
      Exprent source = operands.get(0); int shift = 0;
      List<Operand> layout = List.of();
      if (source instanceof FunctionExprent shifted && isRightShift(shifted)) {
        Long amount = literal(shifted.getLstOperands().get(1));
        if (amount == null) return null;
        shift = amount.intValue() & 31; source = shifted.getLstOperands().get(0);
        layout = List.of(new Operand((ConstExprent)shifted.getLstOperands().get(1), SHIFT));
      }
      return shift + bits > 32 ? null : new Extraction(source, shift, bits, function.getFuncType() != FunctionExprent.FunctionType.I2C, layout);
    }
    if (function.getFuncType() == FunctionExprent.FunctionType.AND) {
      Mask mask = mask(function);
      if (mask == null) return null;
      Exprent value = mask.value();
      int bits = lowMaskBits(literal(mask.constant()), width);
      if (bits <= 0) return null;
      int shift = 0;
      List<Operand> layout = List.of(new Operand(mask.constant(), MASK));
      if (value instanceof FunctionExprent shifted && isRightShift(shifted)) {
        Long amount = literal(shifted.getLstOperands().get(1));
        if (amount == null) return null;
        shift = amount.intValue() & (width - 1); value = shifted.getLstOperands().get(0);
        layout = List.of(new Operand(mask.constant(), shift == 0 ? MASK : VALUE_MASK),
          new Operand((ConstExprent)shifted.getLstOperands().get(1), SHIFT));
      }
      return shift + bits <= width ? new Extraction(value, shift, bits, false, layout) : null;
    }
    if (isRightShift(function)) {
      Long amount = literal(operands.get(1));
      if (amount == null) return null;
      int shift = amount.intValue() & (width - 1);
      Exprent source = operands.get(0);
      if (source instanceof FunctionExprent masked && masked.getFuncType() == FunctionExprent.FunctionType.AND) {
        Mask mask = mask(masked);
        if (mask == null) return null;
        long constant = literal(mask.constant());
        long shifted = (width == 32 ? constant & 0xffffffffL : constant) >>> shift;
        int bits = lowMaskBits(shifted, width);
        if (bits > 0 && bits + shift <= width && (constant & lowMask(shift)) == 0
            && (function.getFuncType() == FunctionExprent.FunctionType.USHR || shift + bits < width)) {
          return new Extraction(mask.value(), shift, bits, false, List.of(new Operand(mask.constant(), MASK),
            new Operand((ConstExprent)operands.get(1), SHIFT)));
        }
      }
      if (function.getFuncType() == FunctionExprent.FunctionType.SHR && source instanceof FunctionExprent shifted
          && shifted.getFuncType() == FunctionExprent.FunctionType.SHL) {
        Long left = literal(shifted.getLstOperands().get(1));
        if (left == null) return null;
        int leftShift = left.intValue() & (width - 1);
        if (leftShift <= shift) return new Extraction(shifted.getLstOperands().get(0), shift - leftShift, width - shift, true);
      }
      return new Extraction(source, shift, width - shift, function.getFuncType() == FunctionExprent.FunctionType.SHR,
        List.of(new Operand((ConstExprent)operands.get(1), SHIFT)));
    }
    return null;
  }

  static Packing packing(Exprent expression, int shift, int bits) {
    Exprent value = expression;
    List<Operand> layout = List.of();
    if (shift != 0) {
      if (!(expression instanceof FunctionExprent function) || function.getFuncType() != FunctionExprent.FunctionType.SHL) return null;
      Long amount = literal(function.getLstOperands().get(1));
      int width = function.getExprType().equals(VarType.VARTYPE_LONG) ? 64 : 32;
      if (amount == null || (amount.intValue() & (width - 1)) != shift) return null;
      value = function.getLstOperands().get(0);
      layout = List.of(new Operand((ConstExprent)function.getLstOperands().get(1), SHIFT));
    }
    if (value instanceof FunctionExprent masked && masked.getFuncType() == FunctionExprent.FunctionType.AND) {
      Mask mask = mask(masked);
      int width = masked.getExprType().equals(VarType.VARTYPE_LONG) ? 64 : 32;
      if (mask == null || lowMaskBits(literal(mask.constant()), width) != bits) return null;
      // At bit zero the stored and value masks coincide. Use one name so a
      // read-modify-write does not acquire conflicting extraction/packing names.
      Operand operand = new Operand(mask.constant(), shift == 0 ? MASK : VALUE_MASK);
      layout = layout.isEmpty() ? List.of(operand) : List.of(operand, layout.get(0));
      value = mask.value();
    }
    return new Packing(value, layout);
  }

  private static Mask mask(FunctionExprent function) {
    List<Exprent> operands = function.getLstOperands();
    for (int index = 1; index >= 0; index--) {
      if (literal(operands.get(index)) != null)
        return new Mask(operands.get(1 - index), (ConstExprent)operands.get(index));
    }
    return null;
  }

  private static boolean isRightShift(FunctionExprent function) {
    return function.getFuncType() == FunctionExprent.FunctionType.SHR || function.getFuncType() == FunctionExprent.FunctionType.USHR;
  }

  static long lowMask(int bits) { return bits == 64 ? -1L : (1L << bits) - 1; }

  private static int lowMaskBits(long mask, int width) {
    if (width == 32) mask &= 0xffffffffL;
    return mask != 0 && (mask & (mask + 1)) == 0 ? Long.bitCount(mask) : -1;
  }
}
