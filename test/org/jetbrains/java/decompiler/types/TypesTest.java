package org.jetbrains.java.decompiler.types;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

public class TypesTest {
  @Test
  public void intJoins() {
    MinimalFernflowerEnvironment.setup();

    // lower type join with int => int
    Assertions.assertEquals(VarType.VARTYPE_INT, VarType.join(VarType.VARTYPE_INT, VarType.VARTYPE_BYTECHAR));
    Assertions.assertEquals(VarType.VARTYPE_INT, VarType.join(VarType.VARTYPE_INT, VarType.VARTYPE_SHORTCHAR));
    Assertions.assertEquals(VarType.VARTYPE_INT, VarType.join(VarType.VARTYPE_INT, VarType.VARTYPE_CHAR));
    Assertions.assertEquals(VarType.VARTYPE_INT, VarType.join(VarType.VARTYPE_INT, VarType.VARTYPE_SHORT));
    Assertions.assertEquals(VarType.VARTYPE_INT, VarType.join(VarType.VARTYPE_INT, VarType.VARTYPE_BYTE));
  }

  @Test
  public void primitiveArrayJoinsUseJavaArrayAssignability() {
    MinimalFernflowerEnvironment.setup();

    VarType byteArray = VarType.VARTYPE_BYTE.resizeArrayDim(1);
    VarType intArray = VarType.VARTYPE_INT.resizeArrayDim(1);
    VarType objectArray = VarType.VARTYPE_OBJECT.resizeArrayDim(1);
    VarType stringArray = new VarType("java/lang/String", true).resizeArrayDim(1);

    Assertions.assertEquals(VarType.VARTYPE_OBJECT, VarType.join(byteArray, intArray));
    Assertions.assertEquals(VarType.VARTYPE_OBJECT, VarType.join(byteArray, objectArray));
    Assertions.assertEquals(objectArray, VarType.join(stringArray, objectArray));
  }

  @Test
  public void multidimensionalPrimitiveArraysJoinThroughReferenceComponents() {
    MinimalFernflowerEnvironment.setup();

    VarType byteMatrix = VarType.VARTYPE_BYTE.resizeArrayDim(2);
    VarType intMatrix = VarType.VARTYPE_INT.resizeArrayDim(2);
    VarType objectArray = VarType.VARTYPE_OBJECT.resizeArrayDim(1);

    Assertions.assertEquals(objectArray, VarType.join(byteMatrix, intMatrix));
  }

  @Test
  public void resizeArrayDimRecomputesTypeFamily() {
    MinimalFernflowerEnvironment.setup();

    VarType intArray = VarType.VARTYPE_INT.resizeArrayDim(1);
    Assertions.assertTrue(VarType.VARTYPE_OBJECT.higherEqualInLatticeThan(intArray));
    Assertions.assertFalse(VarType.VARTYPE_INT.higherEqualInLatticeThan(intArray));
    Assertions.assertEquals(VarType.VARTYPE_INT, intArray.resizeArrayDim(0));
  }

  @Test
  public void arraysAreAssignableToStandardArraySupertypes() {
    MinimalFernflowerEnvironment.setup();

    VarType intArray = VarType.VARTYPE_INT.resizeArrayDim(1);
    VarType stringArray = VarType.VARTYPE_STRING.resizeArrayDim(1);

    Assertions.assertTrue(VarType.VARTYPE_OBJECT.higherEqualInLatticeThan(intArray));
    Assertions.assertTrue(VarType.VARTYPE_CLONEABLE.higherEqualInLatticeThan(intArray));
    Assertions.assertTrue(VarType.VARTYPE_SERIALIZABLE.higherEqualInLatticeThan(intArray));
    Assertions.assertTrue(VarType.VARTYPE_CLONEABLE.higherEqualInLatticeThan(stringArray));
    Assertions.assertTrue(VarType.VARTYPE_SERIALIZABLE.higherEqualInLatticeThan(stringArray));
    Assertions.assertFalse(VarType.VARTYPE_STRING.higherEqualInLatticeThan(intArray));
  }

  // TODO: add the rest
}
