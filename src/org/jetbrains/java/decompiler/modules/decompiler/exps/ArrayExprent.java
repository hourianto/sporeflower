// Copyright 2000-2018 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.exps;

import org.jetbrains.java.decompiler.modules.decompiler.ExprProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.CheckTypesResult;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.InterpreterUtil;
import org.jetbrains.java.decompiler.util.TextBuffer;

import java.util.BitSet;
import java.util.List;

public class ArrayExprent extends Exprent {
  private Exprent array;
  private Exprent index;
  private final VarType hardType;
  private VarType arrayRenderUpperBound;

  public ArrayExprent(Exprent array, Exprent index, VarType hardType, BitSet bytecodeOffsets) {
    super(Type.ARRAY);
    this.array = array;
    this.index = index;
    this.hardType = hardType;

    addBytecodeOffsets(bytecodeOffsets);
  }

  @Override
  public Exprent copy() {
    return new ArrayExprent(array.copy(), index.copy(), hardType, bytecode);
  }

  @Override
  public VarType getExprType() {
    VarType exprType = array.getExprType();
    if (exprType.equals(VarType.VARTYPE_NULL)) {
      return fallbackElementType();
    }
    else if (exprType.arrayDim == 0) {
      return fallbackElementType();
    } else {
      VarType elementType = exprType.decreaseArrayDim();
      if (elementType.typeFamily == VarType.VARTYPE_OBJECT.typeFamily && hardType.typeFamily != VarType.VARTYPE_OBJECT.typeFamily) {
        return fallbackElementType();
      }
      return elementType;
    }
  }

  @Override
  public VarType getInferredExprType(VarType upperBound) {
    VarType arrayUpperBound = toArrayUpperBound(upperBound);
    rememberArrayRenderUpperBound(arrayUpperBound);

    VarType exprType = array.getInferredExprType(arrayUpperBound);
    if (exprType.equals(VarType.VARTYPE_NULL)) {
      return fallbackElementType();
    }
    else if (exprType.arrayDim == 0) {
      return fallbackElementType();
    } else {
      VarType elementType = exprType.decreaseArrayDim();
      if (elementType.typeFamily == VarType.VARTYPE_OBJECT.typeFamily && hardType.typeFamily != VarType.VARTYPE_OBJECT.typeFamily) {
        return fallbackElementType();
      }
      return elementType;
    }
  }

  @Override
  public int getExprentUse() {
    return array.getExprentUse() & index.getExprentUse() & Exprent.MULTIPLE_USES;
  }

  @Override
  public CheckTypesResult checkExprTypeBounds() {
    CheckTypesResult result = new CheckTypesResult();
    result.addExprLowerBound(index, VarType.VARTYPE_BYTECHAR);
    result.addExprUpperBound(index, VarType.VARTYPE_INT);
    return result;
  }

  @Override
  public List<Exprent> getAllExprents(List<Exprent> lst) {
    lst.add(array);
    lst.add(index);
    return lst;
  }

  @Override
  public TextBuffer toJava(int indent) {
    // Array bytecode can merge distinct reference-array branches before an aaload.
    // Give the receiver its source-level array bound so legacy ternaries cast a branch when needed.
    VarType renderUpperBound = arrayRenderUpperBound == null ? fallbackArrayType() : arrayRenderUpperBound;
    array.getInferredExprType(renderUpperBound);

    TextBuffer res = array.toJava(indent);

    if (array.getPrecedence() > getPrecedence() && !canSkipParenEnclose(array)) { // array precedence equals 0
      res.encloseWithParens();
    }

    VarType arrType = array.getExprType();
    if (arrType.arrayDim == 0 || shouldCastToFallbackArrayType(arrType)) {
      VarType castType = fallbackArrayType();
      res.enclose("((" + ExprProcessor.getCastTypeName(castType) + ")", ")");
      res.addTypeNameToken(castType, 2);
    }

    res.addBytecodeMapping(bytecode);

    return res.append('[').append(index.toJava(indent)).append(']');
  }

  private boolean canSkipParenEnclose(Exprent instance) {
    if (!(instance instanceof NewExprent)) {
      return false;
    }

    NewExprent newExpr = (NewExprent) instance;

    return newExpr.isDirectArrayInit() || !newExpr.getLstArrayElements().isEmpty();
  }

  @Override
  public void replaceExprent(Exprent oldExpr, Exprent newExpr) {
    if (oldExpr == array) {
      array = newExpr;
    }
    if (oldExpr == index) {
      index = newExpr;
    }
  }

  @Override
  public boolean equals(Object o) {
    if (o == this) return true;
    if (!(o instanceof ArrayExprent)) return false;

    ArrayExprent arr = (ArrayExprent)o;
    return InterpreterUtil.equalObjects(array, arr.getArray()) &&
           InterpreterUtil.equalObjects(index, arr.getIndex());
  }

  public Exprent getArray() {
    return array;
  }

  public Exprent getIndex() {
    return index;
  }

  public VarType getHardType() {
    return hardType;
  }

  private VarType fallbackElementType() {
    // `baload` is shared by byte[] and boolean[]; when type information is lost,
    // keep it in the integer/boolean lattice instead of collapsing to plain boolean.
    if (hardType.equals(VarType.VARTYPE_BOOLEAN)) {
      return VarType.VARTYPE_BYTECHAR;
    }
    return hardType;
  }

  private VarType fallbackArrayType() {
    VarType elementType = fallbackElementType();
    if (elementType.equals(VarType.VARTYPE_BYTECHAR)) {
      return VarType.VARTYPE_BYTE.resizeArrayDim(1);
    }
    return elementType.resizeArrayDim(1);
  }

  private static VarType toArrayUpperBound(VarType elementUpperBound) {
    return elementUpperBound == null ? null : elementUpperBound.resizeArrayDim(elementUpperBound.arrayDim + 1);
  }

  private void rememberArrayRenderUpperBound(VarType candidate) {
    if (candidate == null) {
      return;
    }

    if (arrayRenderUpperBound == null || arrayRenderUpperBound.higherEqualInLatticeThan(candidate)) {
      arrayRenderUpperBound = candidate;
    }
  }

  private boolean shouldCastToFallbackArrayType(VarType arrType) {
    VarType elementType = arrType.decreaseArrayDim();
    // Primitive array loads (`xaload` except `aaload`) can degrade to Object when locals are aggressively reused.
    // Keep an explicit cast so codegen remains valid and keeps the primitive semantics of the bytecode instruction.
    if (elementType.typeFamily == VarType.VARTYPE_OBJECT.typeFamily && hardType.typeFamily != VarType.VARTYPE_OBJECT.typeFamily) {
      return true;
    }

    if (!(array instanceof VarExprent varExpr)) {
      return false;
    }

    VarProcessor processor = varExpr.getProcessor();
    if (processor == null) {
      return false;
    }

    VarType parameterType = processor.getDeclaredParameterType(varExpr.getIndex());
    if (parameterType != null && processor.getParams().contains(varExpr.getVarVersionPair())) {
      // If this expression points at a non-array method parameter slot, force a cast so we don't emit `param[idx]`.
      return parameterType.arrayDim == 0;
    }

    return false;
  }
  
  @Override
  public void getBytecodeRange(BitSet values) {
    measureBytecode(values, array);
    measureBytecode(values, index);
    measureBytecode(values);
  }
}
