// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.SwitchHeadExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticFacts.*;

/** Shared JVM value operations; these do not infer semantic meanings. */
final class SemanticExpressions {
  private SemanticExpressions() {}
  private static final Map<String, String> BOXES =
    Map.of("java/lang/Byte", "B", "java/lang/Short", "S", "java/lang/Character", "C", "java/lang/Integer", "I", "java/lang/Long", "J");

  static String primitiveDescriptor(VarType type) {
    if (type == null || type.arrayDim != 0)
      return null;
    return switch (type.type) {
      case BYTE -> "B";
      case SHORT -> "S";
      case CHAR -> "C";
      case INT -> "I";
      case LONG -> "J";
      default -> null;
    };
  }

  static int bitWidth(VarType type) {
    if (type == null)
      return 0;
    return switch (type.type) {
      case BYTE -> 8;
      case SHORT, CHAR -> 16;
      case INT -> 32;
      case LONG -> 64;
      default -> 0;
    };
  }

  static boolean isComparison(FunctionExprent function) {
    return switch (function.getFuncType()) {
      case EQ, NE, LT, LE, GT, GE -> true;
      default -> false;
    };
  }

  static Exprent orderedComparisonValue(Exprent exprent) {
    if (!(exprent instanceof FunctionExprent function) || !isComparison(function)
      || function.getFuncType() == FunctionExprent.FunctionType.EQ || function.getFuncType() == FunctionExprent.FunctionType.NE)
      return null;
    if (literal(function.getLstOperands().get(0)) != null)
      return function.getLstOperands().get(1);
    return literal(function.getLstOperands().get(1)) != null ? function.getLstOperands().get(0) : null;
  }

  static boolean isIntegralCast(FunctionExprent function) {
    return switch (function.getFuncType()) {
      case I2L, I2B, I2S, I2C, L2I -> true;
      default -> false;
    };
  }

  static boolean isValuePreservingCast(FunctionExprent function) {
    if (function.getFuncType() == FunctionExprent.FunctionType.CAST)
      return true;
    if (function.getFuncType() == FunctionExprent.FunctionType.I2L)
      return true;
    if (function.getFuncType().castType == null)
      return false;
    Long value = literal(function.getLstOperands().get(0));
    if (value == null)
      return false;
    // A narrowing cast of a variable can change the represented value. Only
    // pass an integral domain through when this particular operand is unchanged.
    return switch (function.getFuncType()) {
      case I2B -> value == (byte) value.longValue();
      case I2S -> value == (short) value.longValue();
      case I2C -> value == (char) value.longValue();
      case L2I -> value == (int) value.longValue();
      default -> false;
    };
  }

  static boolean fitsType(long value, VarType type) {
    if (type == null || type.arrayDim != 0)
      return false;
    return switch (type.type) {
      case BYTE -> value == (byte) value;
      case SHORT -> value == (short) value;
      case CHAR -> value == (char) value;
      case INT -> value == (int) value;
      case LONG -> true;
      default -> false;
    };
  }

  static boolean isIncrement(FunctionExprent function) {
    return switch (function.getFuncType()) {
      case IPP, PPI, IMM, MMI -> true;
      default -> false;
    };
  }

  static boolean isBitwise(FunctionExprent function) {
    return isBitwise(function.getFuncType());
  }

  static boolean isBitwise(FunctionExprent.FunctionType type) {
    return switch (type) {
      case AND, OR, XOR, BIT_NOT -> true;
      default -> false;
    };
  }

  static MemberKey fieldKey(FieldExprent field) {
    return new MemberKey(field.getClassname(), field.getName(), field.getDescriptor().descriptorString);
  }

  static MemberKey invocationKey(InvocationExprent invocation) {
    return new MemberKey(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor());
  }

  static <T> T unique(Set<T> candidates) {
    return candidates.size() == 1 ? candidates.iterator().next() : null;
  }

  static Long literal(Exprent exprent) {
    if (!(exprent instanceof ConstExprent constant) || !(constant.getValue() instanceof Number number) || number instanceof Float
      || number instanceof Double)
      return null;
    return number.longValue();
  }

  static Exprent boxedArgument(InvocationExprent invocation) {
    String storage = BOXES.get(invocation.getClassname());
    if (storage == null || invocation.getLstParameters().size() != 1 || !Set.of("<init>", "valueOf").contains(invocation.getName()))
      return null;
    // String parsing overloads do not preserve a numeric domain.
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(invocation.getStringDescriptor());
    return storage.equals(primitiveDescriptor(descriptor.params[0])) ? invocation.getLstParameters().get(0) : null;
  }

  static boolean isUnboxing(InvocationExprent invocation) {
    return invocation.getInstance() != null && BOXES.containsKey(invocation.getClassname()) && invocation.getLstParameters().isEmpty()
      && Set.of("byteValue", "shortValue", "charValue", "intValue", "longValue").contains(invocation.getName())
      && primitiveDescriptor(invocation.getExprType()) != null;
  }

  static boolean unboxingPreservesStorage(InvocationExprent invocation) {
    String storage = BOXES.get(invocation.getClassname());
    String target = primitiveDescriptor(invocation.getExprType());
    return storage.equals(target) || "J".equals(target) || "I".equals(target) && !"J".equals(storage)
      || "S".equals(target) && "B".equals(storage);
  }

  static void walk(Exprent exprent, Consumer<Exprent> consumer) {
    consumer.accept(exprent);
    if (exprent instanceof SwitchHeadExprent switchHead) {
      for (List<Exprent> cases : switchHead.getCaseValues()) {
        for (Exprent caseValue : cases)
          if (caseValue != null)
            walk(caseValue, consumer);
      }
    }
    if (exprent instanceof NewExprent creation && creation.getConstructor() != null) {
      consumer.accept(creation.getConstructor());
    }
    for (Exprent child : exprent.getAllExprents()) walk(child, consumer);
  }

  static List<Exprent> roots(Statement statement) {
    List<Exprent> result = new ArrayList<>();
    collect(statement, result);
    return result;
  }
  private static void collect(Statement statement, List<Exprent> result) {
    result.addAll(statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents());
    for (Statement child : statement.getStats()) collect(child, result);
  }
}
