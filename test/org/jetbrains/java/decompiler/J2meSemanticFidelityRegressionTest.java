package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.Init;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Set;
import java.util.stream.Collectors;

import static org.junit.jupiter.api.Assertions.assertAll;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class J2meSemanticFidelityRegressionTest extends DecompileRegressionTestBase {
  private static final String PACKAGE = "pkg.";

  @Override
  protected Object[] fixtureOptions() {
    return new Object[] {IFernflowerPreferences.J2ME_STRICT_SLOT_MERGE, "1"};
  }

  @Test
  public void testLoopCarriedValueSurvivesEarlierIncompatibleSlotLifetimes() throws Exception {
    String simpleName = "TestLoopCarriedSlotReuse";
    decompileJasmFixture(simpleName);
    recompile();

    Path originalRoot = fixture.getTestDataDir().resolve("classes/jasm");
    Path recompiledRoot = fixture.getTempDir().resolve("recompiled-out");
    int[][] dimensions = {{0, 0}, {1, 1}, {2, 3}, {4, 2}, {5, 5}};

    try (URLClassLoader originalLoader = isolatedLoader(originalRoot);
         URLClassLoader recompiledLoader = isolatedLoader(recompiledRoot)) {
      Method original = Class.forName(PACKAGE + simpleName, true, originalLoader)
        .getMethod("scan", int.class, Object.class, int.class, int.class);
      Method recompiled = Class.forName(PACKAGE + simpleName, true, recompiledLoader)
        .getMethod("scan", int.class, Object.class, int.class, int.class);

      for (int[] pair : dimensions) {
        for (Object marker : new Object[] {null, new Object()}) {
          int expected = expectedScan(pair[0], pair[1], marker != null);
          assertEquals(expected, original.invoke(null, 0, marker, pair[0], pair[1]),
            "JASM fixture does not implement its semantic oracle");
          assertEquals(expected, recompiled.invoke(null, 0, marker, pair[0], pair[1]),
            "Loop-carried value changed for maxX=" + pair[0] + ", maxY=" + pair[1] + ", marker=" + (marker != null));
        }
      }
    }
  }

  @Test
  public void testDuplicatedFieldNullChecksUseTheCapturedValue() throws Exception {
    String simpleName = "TestCapturedFieldNullCheck";
    decompileJasmFixture(simpleName);
    assertCapturedFieldBehavior(fixture.getTestDataDir().resolve("classes/jasm"), simpleName);
    recompile();

    Path originalClass = jasmClass(simpleName);
    Path recompiledRoot = fixture.getTempDir().resolve("recompiled-out");
    Path recompiledClass = recompiledRoot.resolve("pkg/" + simpleName + ".class");
    prepareClassReader();
    assertAll(
      () -> assertSingleFieldRead(originalClass, recompiledClass, "captureStatic", "()Ljava/lang/Object;", CodeConstants.opc_getstatic),
      () -> assertSingleFieldRead(originalClass, recompiledClass, "sumInstance", "()I", CodeConstants.opc_getfield),
      () -> assertSingleFieldRead(originalClass, recompiledClass, "trimCaptured", "(Z)Ljava/lang/String;", CodeConstants.opc_getstatic)
    );
    assertCapturedFieldBehavior(recompiledRoot, simpleName);
  }

  private static void assertCapturedFieldBehavior(Path root, String simpleName) throws Exception {
    try (URLClassLoader loader = isolatedLoader(root)) {
      Class<?> type = Class.forName(PACKAGE + simpleName, true, loader);
      Field staticValue = type.getField("staticValue");
      assertNull(type.getMethod("captureStatic").invoke(null));
      Object marker = new Object();
      staticValue.set(null, marker);
      assertSame(marker, type.getMethod("captureStatic").invoke(null));

      Object instance = type.getConstructor().newInstance();
      assertEquals(-1, type.getMethod("sumInstance").invoke(instance));
      type.getField("instanceValues").set(instance, new int[0]);
      assertEquals(0, type.getMethod("sumInstance").invoke(instance));
      type.getField("instanceValues").set(instance, new int[] {2, 3, 5, 7});
      assertEquals(17, type.getMethod("sumInstance").invoke(instance));

      assertNull(type.getMethod("trimCaptured", boolean.class).invoke(null, false));
      type.getField("text").set(null, "  captured  ");
      assertEquals("captured", type.getMethod("trimCaptured", boolean.class).invoke(null, true));
      type.getField("text").set(null, "unchanged");
      assertEquals("unchanged", type.getMethod("trimCaptured", boolean.class).invoke(null, false));
    }
  }

  @Test
  public void testResidualInterfaceInitializerIsLegalAndKeepsInterleavedOrder() throws Exception {
    String interfaceName = "TestResidualInterfaceInitializer";
    String probeName = interfaceName + "Probe";
    String content = decompileJasmFixture(interfaceName, probeName);
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertTrue(content.contains("new int"), "Discarded allocation vanished:\n" + content);

    Path originalRoot = fixture.getTestDataDir().resolve("classes/jasm");
    assertEquals(123, initializeInterfaceAndReadProbe(originalRoot, interfaceName, probeName));
    prepareClassReader();
    assertEquals(1, countOpcodeInClassFamily(originalRoot.resolve("pkg"), interfaceName, CodeConstants.opc_newarray),
      "JASM fixture must contain exactly one discarded primitive-array allocation");

    recompile();
    Path recompiledRoot = fixture.getTempDir().resolve("recompiled-out");
    assertEquals(123, initializeInterfaceAndReadProbe(recompiledRoot, interfaceName, probeName));

    Path recompiledInterface = recompiledRoot.resolve("pkg/" + interfaceName + ".class");
    prepareClassReader();
    StructClass interfaceClass = readClass(recompiledInterface);
    assertEquals(Set.of("first", "last"), interfaceClass.getFields().stream().map(field -> field.getName()).collect(Collectors.toSet()),
      "Do not legalize residual initialization by adding observable interface fields");
    assertTrue(interfaceClass.getMethods().stream().allMatch(method -> "<clinit>".equals(method.getName())),
      "Do not add source-level static methods to a legacy interface");
    assertTrue(countOpcodeInClassFamily(recompiledRoot.resolve("pkg"), interfaceName, CodeConstants.opc_newarray) >= 1,
      "The discarded primitive-array allocation must remain in the reconstructed class family");
  }

  private String decompileJasmFixture(String primaryName, String... supportNames) throws IOException {
    Path inputPackage = fixture.getTempDir().resolve("jasm-input/pkg");
    Files.createDirectories(inputPackage);
    Files.copy(jasmClass(primaryName), inputPackage.resolve(primaryName + ".class"));
    for (String supportName : supportNames) {
      Files.copy(jasmClass(supportName), inputPackage.resolve(supportName + ".class"));
    }
    return decompileDirectory(inputPackage.getParent(), "pkg/" + primaryName + ".java");
  }

  private Path jasmClass(String simpleName) {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/" + simpleName + ".class");
    assertTrue(Files.isRegularFile(classFile), "Missing JASM fixture: " + classFile);
    return classFile;
  }

  private static URLClassLoader isolatedLoader(Path classes) throws IOException {
    return new URLClassLoader(new URL[] {classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader());
  }

  private static int expectedScan(int maxX, int maxY, boolean markerPresent) {
    int accumulator = 1; // The pre-loop contributes 0 + 1 with outer index zero.
    for (int diagonal = 0; diagonal <= maxX + maxY; diagonal++) {
      int x = Math.min(diagonal, maxX);
      int y = diagonal - x;
      do {
        accumulator = accumulator * 31 + x * 7 + y;
        if (x <= maxX && x >= 0 && y <= maxY && y >= 0) {
          if (((x + y) & 1) == 0) {
            accumulator += x + y;
          }
          else if (markerPresent) {
            int packed = x * 2 + y;
            accumulator += (packed & 1) == 0 ? x - y : x + y;
          }
        }
        x--;
        y = diagonal - x;
      }
      while (x >= 0 && y <= maxY);
    }
    return accumulator;
  }

  private static void assertSingleFieldRead(
    Path originalClass,
    Path recompiledClass,
    String methodName,
    String descriptor,
    int fieldReadOpcode
  ) throws IOException {
    int originalReads = countOpcode(originalClass, methodName, descriptor, fieldReadOpcode);
    int recompiledReads = countOpcode(recompiledClass, methodName, descriptor, fieldReadOpcode);
    assertEquals(1, originalReads, "Fixture must contain exactly one field read in " + methodName);
    assertEquals(originalReads, recompiledReads,
      "Recompilation introduced another field read instead of testing the captured value in " + methodName);
  }

  private static int countOpcode(Path classFile, String methodName, String descriptor, int opcode) throws IOException {
    StructClass structClass = readClass(classFile);
    StructMethod method = structClass.getMethod(methodName, descriptor);
    assertNotNull(method, "Missing method " + methodName + descriptor + " in " + classFile);
    method.expandData(structClass);
    int count = 0;
    for (var instruction : method.getInstructionSequence()) {
      if (instruction.opcode == opcode) {
        count++;
      }
    }
    return count;
  }

  private static StructClass readClass(Path classFile) throws IOException {
    try (DataInputFullStream in = new DataInputFullStream(Files.readAllBytes(classFile))) {
      return StructClass.create(in, true);
    }
  }

  private static void prepareClassReader() {
    MinimalFernflowerEnvironment.setup();
    Init.init();
  }

  private static int initializeInterfaceAndReadProbe(Path root, String interfaceName, String probeName) throws Exception {
    try (URLClassLoader loader = isolatedLoader(root)) {
      Class<?> probe = Class.forName(PACKAGE + probeName, true, loader);
      probe.getField("log").setInt(null, 0);
      Class<?> interfaceClass = Class.forName(PACKAGE + interfaceName, false, loader);
      assertEquals(3, interfaceClass.getField("last").getInt(null));
      return probe.getField("log").getInt(null);
    }
  }

  private static int countOpcodeInClassFamily(Path packageDir, String namePrefix, int opcode) throws IOException {
    int count = 0;
    try (var files = Files.list(packageDir)) {
      for (Path classFile : files.filter(path -> path.getFileName().toString().startsWith(namePrefix))
        .filter(path -> path.getFileName().toString().endsWith(".class")).toList()) {
        StructClass structClass = readClass(classFile);
        for (StructMethod method : structClass.getMethods()) {
          method.expandData(structClass);
          if (method.getInstructionSequence() == null) {
            continue;
          }
          for (var instruction : method.getInstructionSequence()) {
            if (instruction.opcode == opcode) {
              count++;
            }
          }
        }
      }
    }
    return count;
  }
}
