package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.List;

/** Matches bit extraction shapes, keeping signedness and JVM shift widths explicit. */
final class SemanticBitAccess {
  record Extraction(Exprent source, int shift, int bits, boolean signed) {}
  private SemanticBitAccess() {}

  static Extraction extraction(Exprent expression) {
    if (!(expression instanceof FunctionExprent function)) return null;
    List<Exprent> operands = function.getLstOperands();
    int width = function.getExprType().equals(VarType.VARTYPE_LONG) ? 64 : 32;
    if (function.getFuncType() == FunctionExprent.FunctionType.I2B || function.getFuncType() == FunctionExprent.FunctionType.I2S
        || function.getFuncType() == FunctionExprent.FunctionType.I2C) {
      int bits = function.getFuncType() == FunctionExprent.FunctionType.I2B ? 8 : 16;
      Exprent source = operands.get(0); int shift = 0;
      if (source instanceof FunctionExprent shifted && isRightShift(shifted)) {
        Long amount = SemanticContext.integral(shifted.getLstOperands().get(1));
        if (amount == null) return null;
        shift = amount.intValue() & 31; source = shifted.getLstOperands().get(0);
      }
      return shift + bits > 32 ? null : new Extraction(source, shift, bits, function.getFuncType() != FunctionExprent.FunctionType.I2C);
    }
    if (function.getFuncType() == FunctionExprent.FunctionType.AND) {
      Exprent value = operands.get(0); Long mask = SemanticContext.integral(operands.get(1));
      if (mask == null) { mask = SemanticContext.integral(value); value = operands.get(1); }
      if (mask == null) return null;
      int bits = lowMaskBits(mask, width);
      if (bits <= 0) return null;
      int shift = 0;
      if (value instanceof FunctionExprent shifted && isRightShift(shifted)) {
        Long amount = SemanticContext.integral(shifted.getLstOperands().get(1));
        if (amount == null) return null;
        shift = amount.intValue() & (width - 1); value = shifted.getLstOperands().get(0);
      }
      return shift + bits <= width ? new Extraction(value, shift, bits, false) : null;
    }
    if (isRightShift(function)) {
      Long amount = SemanticContext.integral(operands.get(1));
      if (amount == null) return null;
      int shift = amount.intValue() & (width - 1);
      Exprent source = operands.get(0);
      if (source instanceof FunctionExprent masked && masked.getFuncType() == FunctionExprent.FunctionType.AND) {
        Long mask = SemanticContext.integral(masked.getLstOperands().get(1));
        if (mask == null) return null;
        long shifted = (width == 32 ? mask & 0xffffffffL : mask) >>> shift;
        int bits = lowMaskBits(shifted, width);
        if (bits > 0 && bits + shift <= width && (mask & lowMask(shift)) == 0
            && (function.getFuncType() == FunctionExprent.FunctionType.USHR || shift + bits < width)) {
          return new Extraction(masked.getLstOperands().get(0), shift, bits, false);
        }
      }
      if (function.getFuncType() == FunctionExprent.FunctionType.SHR && source instanceof FunctionExprent shifted
          && shifted.getFuncType() == FunctionExprent.FunctionType.SHL) {
        Long left = SemanticContext.integral(shifted.getLstOperands().get(1));
        if (left == null) return null;
        int leftShift = left.intValue() & (width - 1);
        if (leftShift <= shift) return new Extraction(shifted.getLstOperands().get(0), shift - leftShift, width - shift, true);
      }
      return new Extraction(source, shift, width - shift, function.getFuncType() == FunctionExprent.FunctionType.SHR);
    }
    return null;
  }

  static Exprent packingValue(Exprent expression, int shift, int bits) {
    Exprent value = expression;
    if (shift != 0) {
      if (!(expression instanceof FunctionExprent function) || function.getFuncType() != FunctionExprent.FunctionType.SHL) return null;
      Long amount = SemanticContext.integral(function.getLstOperands().get(1));
      int width = function.getExprType().equals(VarType.VARTYPE_LONG) ? 64 : 32;
      if (amount == null || (amount.intValue() & (width - 1)) != shift) return null;
      value = function.getLstOperands().get(0);
    }
    if (value instanceof FunctionExprent mask && mask.getFuncType() == FunctionExprent.FunctionType.AND) {
      Long constant = SemanticContext.integral(mask.getLstOperands().get(1));
      if (constant == null || constant != lowMask(bits)) return null;
      value = mask.getLstOperands().get(0);
    }
    return value;
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
