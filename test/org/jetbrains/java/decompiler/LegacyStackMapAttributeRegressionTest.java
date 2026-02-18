package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.Init;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.jetbrains.java.decompiler.util.Key;
import org.junit.jupiter.api.Test;

import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class LegacyStackMapAttributeRegressionTest {
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

    Object stackMap = method.getAttribute(Key.of("StackMap"));
    assertNotNull(stackMap, "Legacy StackMap attribute should be preserved on methods");

    Method getLocalType = stackMap.getClass().getMethod("getLocalTypeExact", int.class, int.class);
    assertEquals(VarType.VARTYPE_INT, getLocalType.invoke(stackMap, 5, 2));
    assertEquals(new VarType("java/lang/Object", true), getLocalType.invoke(stackMap, 9, 2));
  }
}
