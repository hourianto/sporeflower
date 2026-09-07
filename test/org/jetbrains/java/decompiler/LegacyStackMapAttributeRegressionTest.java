package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.Init;
import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructStackMapAttribute;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacyStackMapAttributeRegressionTest {
  @Test
  public void testUninitializedValuesAreNotJavaReferenceConstraints() throws Exception {
    StructStackMapAttribute attribute = new StructStackMapAttribute();
    // One frame: uninitializedThis, uninitialized(new@10), long; stack: double.
    byte[] bytes = {0, 1, 0, 0, 0, 3, 6, 8, 0, 10, 4, 0, 1, 3};
    try (DataInputFullStream input = new DataInputFullStream(bytes)) {
      attribute.initContent(input, null, new BytecodeVersion(45, 3));
    }
    assertNull(attribute.getLocalTypeExact(0, 0));
    assertNull(attribute.getLocalTypeExact(0, 1));
    assertEquals(VarType.VARTYPE_LONG, attribute.getLocalTypeExact(0, 2));
    assertNull(attribute.getLocalTypeExact(0, 3));
    assertEquals(java.util.List.of(VarType.VARTYPE_DOUBLE), attribute.getFrame(0).getStack());
  }

  @Test
  public void testLegacyStackMapAttributeIsParsedAndQueryable() throws Exception {
    MinimalFernflowerEnvironment.setup();
    Init.init();

    Path classFile = new DecompilerTestFixture().getTestDataDir().resolve("classes/jasm/pkg/TestLegacyStackMapSlotProbe.class");
    assertTrue(Files.isRegularFile(classFile), "Missing test class: " + classFile);

    StructClass structClass;
    try (DataInputFullStream in = new DataInputFullStream(Files.readAllBytes(classFile))) {
      structClass = StructClass.create(in, true);
    }

    StructMethod method = structClass.getMethod("test", "(Ljava/lang/Object;I)I");
    assertNotNull(method, "Missing method test(Ljava/lang/Object;I)I");

    StructStackMapAttribute stackMap = method.getAttribute(StructGeneralAttribute.ATTRIBUTE_STACK_MAP);
    assertNotNull(stackMap, "Legacy StackMap attribute should be preserved on methods");

    assertEquals(VarType.VARTYPE_INT, stackMap.getLocalTypeExact(5, 2));
    assertEquals(new VarType("java/lang/Object", true), stackMap.getLocalTypeExact(9, 2));
    assertNull(stackMap.getFrame(10), "Frames are constraints at instruction entries, not ranges");
    assertNull(stackMap.getLocalTypeExact(10, 2));
  }
}
