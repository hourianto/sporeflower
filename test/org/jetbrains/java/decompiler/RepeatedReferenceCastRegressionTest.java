package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.io.Serializable;
import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Vector;

import static org.junit.jupiter.api.Assertions.*;

public class RepeatedReferenceCastRegressionTest extends DecompileRegressionTestBase {
  private static final String NAME = "TestRepeatedReferenceCasts";
  private static final String DESCRIPTOR = "(Ljava/util/Vector;)Ljava/lang/Object;";

  @Override
  protected Object[] fixtureOptions() {
    return new Object[]{IFernflowerPreferences.BUNDLED_J2ME_API, "0", IFernflowerPreferences.INCLUDE_ENTIRE_CLASSPATH, "0"};
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  public void repeatedCastsRemainLegalWithOrWithoutMetadata(boolean withMetadata) throws Exception {
    Path original = fixture.getTestDataDir().resolve("classes/jasm/pkg/" + NAME + ".class");
    assertBytecodeShape(original);

    // Supply declarations explicitly so the missing-metadata case cannot
    // accidentally use the test JVM's classpath or bundled API declarations.
    List<Class<?>> declarations = List.of(Object.class, Vector.class, Number.class, Integer.class,
      String.class, Comparable.class, Serializable.class);
    if (withMetadata) {
      Path library = fixture.getTempDir().resolve("library");
      for (Class<?> type : declarations) {
        String resource = type.getName().replace('.', '/') + ".class";
        Path target = library.resolve(resource);
        Files.createDirectories(target.getParent());
        try (var stream = type.getResourceAsStream("/" + resource)) {
          assertNotNull(stream);
          Files.copy(stream, target);
        }
      }
      fixture.getDecompiler().addLibrary(library.toFile());
    }
    for (Class<?> type : declarations) {
      assertEquals(withMetadata, DecompilerContext.getStructContext().getClass(type.getName().replace('.', '/')) != null,
        "Unexpected metadata availability for " + type.getName());
    }

    assertBehavior(original.getParent().getParent());
    String content = decompileClassFile(original, NAME + ".java");
    assertFalse(content.contains("$VF: Couldn't be decompiled"), content);
    assertAll(
      () -> assertFalse(content.contains("Integer & Integer"), content),
      () -> assertFalse(content.contains("String & String"), content),
      () -> assertFalse(content.contains("Serializable & Serializable"), content)
    );
    assertEquals(withMetadata, content.contains("Serializable & Comparable"),
      "Distinct, resolved interface bounds should still form an intersection: " + content);
    recompile();
    assertBehavior(fixture.getTempDir().resolve("recompiled-out"));
  }

  private static void assertBytecodeShape(Path original) throws Exception {
    try (var stream = new DataInputFullStream(Files.readAllBytes(original))) {
      StructClass type = StructClass.create(stream, true);
      for (String methodName : List.of("readInteger", "readString", "useInteger", "useString")) {
        var method = type.getMethod(methodName, DESCRIPTOR);
        assertNotNull(method);
        method.expandData(type);
        var instructions = method.getInstructionSequence();
        assertEquals(CodeConstants.opc_invokevirtual, instructions.getInstr(2).opcode);
        var read = type.getPool().getLinkConstant(instructions.getInstr(2).operand(0));
        assertEquals("java/util/Vector", read.classname);
        assertEquals("elementAt", read.elementname);
        assertEquals("(I)Ljava/lang/Object;", read.descriptor);
        String expectedType = methodName.endsWith("Integer") ? "java/lang/Integer" : "java/lang/String";
        for (int index : new int[]{3, 4}) {
          assertEquals(CodeConstants.opc_checkcast, instructions.getInstr(index).opcode);
          assertEquals(expectedType, type.getPool().getPrimitiveConstant(instructions.getInstr(index).operand(0)).getString());
        }
      }
    }
  }

  private static void assertBehavior(Path classes) throws Exception {
    try (var loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      Class<?> type = loader.loadClass("pkg." + NAME);
      for (String method : List.of("readInteger", "readString", "useInteger", "useString", "useNumberThenInteger",
        "useInterfaces", "useDistinctInterfaces")) {
        Object valid = method.endsWith("String") || method.endsWith("Interfaces") ? "value" : Integer.valueOf(42);
        checkCall(type, method, valid, true);
        checkCall(type, method, null, true);
        checkCall(type, method, new Object(), false);
        if (method.endsWith("Interfaces")) {
          // An object satisfying only either bound must still fail.
          checkCall(type, method, new Serializable() {}, false);
          checkCall(type, method, (Comparable<Object>)other -> 0, false);
        } else {
          checkCall(type, method, method.endsWith("String") ? Integer.valueOf(42) : "wrong", false);
          if (method.equals("useNumberThenInteger")) {
            checkCall(type, method, Double.valueOf(1), false);
          }
        }
      }
    }
  }

  private static void checkCall(Class<?> type, String method, Object value, boolean valid) throws Exception {
    CountingVector values = new CountingVector();
    values.addElement(value);
    var target = type.getMethod(method, Vector.class);
    if (valid) {
      assertSame(value, target.invoke(null, values), method);
    } else {
      InvocationTargetException failure = assertThrows(InvocationTargetException.class, () -> target.invoke(null, values), method);
      assertInstanceOf(ClassCastException.class, failure.getCause(), method);
    }
    assertEquals(1, values.reads, "Read evaluated more than once in " + method);
    assertEquals(valid && method.startsWith("read") ? 0 : 1, values.size(), "Side effect moved across a cast in " + method);
  }

  private static final class CountingVector extends Vector<Object> {
    private int reads;

    @Override
    public synchronized Object elementAt(int index) {
      reads++;
      return super.elementAt(index);
    }
  }
}
