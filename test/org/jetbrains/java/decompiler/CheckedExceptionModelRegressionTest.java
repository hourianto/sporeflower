package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class CheckedExceptionModelRegressionTest extends DecompileRegressionTestBase {
  @Test
  public void testInferenceCompletesPartialExceptionsAttribute() throws IOException {
    String content = decompileJasmClass("TestPartialDeclaredThrows");
    assertTrue(content.contains("throws SQLException, IOException"), content);
    recompile();
  }

  @Test
  public void testExplicitThrowableIsNotHiddenByPartialExceptionsAttribute() throws IOException {
    String content = decompileJasmClass("TestExplicitThrowableWithPartialThrows");
    assertTrue(content.contains("throws IOException, Throwable"), content);
    recompile();
  }

  @Test
  public void testPrivateAncestorMethodDoesNotConstrainThrows() throws IOException {
    String content = decompileJasmPair("TestPrivateAncestorThrowsBase", "TestPrivateAncestorThrowsChild");
    assertTrue(content.contains("int read() throws IOException"), content);
    assertFalse(content.contains("Wrapped undeclared checked exception"), content);
    recompile();
  }

  @Test
  public void testCovariantOverrideStillConstrainsThrows() throws IOException {
    String content = decompileJasmPair("TestCovariantThrowsBase", "TestCovariantThrowsChild");
    assertFalse(content.contains("String value() throws IOException"), content);
    assertTrue(content.contains("catch (IOException $VF$ex)"), content);
    assertTrue(content.contains("throw new RuntimeException"), content);
    recompile();
  }

  @Test
  public void testCheckedExceptionInStaticInitializerIsRepaired() throws IOException {
    String content = decompileJasmClass("TestCheckedStaticInitializer");
    assertTrue(content.contains("static {"), content);
    assertTrue(content.contains("catch (IOException"), content);
    assertTrue(content.contains("throw new RuntimeException"), content);
    recompile();
  }

  @Test
  public void testCheckedExceptionInInterfaceInitializerUsesModeledHolder() throws IOException {
    String content = decompileJasmClass("TestCheckedInterfaceInitializer");
    assertTrue(content.contains("final class VFCheckedInitializer"), content);
    assertTrue(content.contains("catch (ClassNotFoundException $VF$ex)"), content);
    assertTrue(content.contains("throw new RuntimeException"), content);
    recompile();
  }

  @Test
  public void testCheckedExceptionInEnumStaticInitializerUsesModeledHolder() throws IOException {
    String content = decompileJasmClass("TestCheckedEnumStaticInitializer");
    assertTrue(content.contains("final class VFCheckedInitializer"), content);
    assertTrue(content.contains("catch (ClassNotFoundException $VF$ex)"), content);
    recompile();
  }

  @Test
  public void testReachabilityMarkerParticipatesInPreciseRethrowSummary() throws IOException {
    String content = decompileJasmClass("TestCheckedCatchRethrowWithoutThrows");
    assertTrue(content.contains("test() throws IOException"), content);
    assertTrue(content.contains("if (false)"), content);
    recompile();
  }

  @Test
  public void testExceptionCatchConsumesUnknownCheckedFlow() throws IOException {
    String content = decompileJasmClass("TestUnknownCaughtByException");
    assertFalse(content.contains("work() throws IOException"), content);
    assertTrue(content.contains("catch (Exception"), content);

    Path apiStub = writeSource("ext/MissingApi.java", """
      package ext;

      public final class MissingApi {
        public static void run() {
        }
      }
      """);
    recompile(List.of(apiStub));
  }

  @Test
  public void testPackagePrivateMethodIsInheritedAcrossInterveningPackage() throws IOException {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm");
    Path input = fixture.getTempDir().resolve("package-path-input");
    Files.createDirectories(input.resolve("patha"));
    Files.createDirectories(input.resolve("pathb"));
    Files.copy(classes.resolve("patha/TestPackagePathBase.class"), input.resolve("patha/TestPackagePathBase.class"));
    Files.copy(classes.resolve("pathb/TestPackagePathMiddle.class"), input.resolve("pathb/TestPackagePathMiddle.class"));
    Files.copy(classes.resolve("patha/TestPackagePathChild.class"), input.resolve("patha/TestPackagePathChild.class"));

    String content = decompileDirectory(input, "patha/TestPackagePathChild.java");
    assertFalse(content.contains("int read() throws IOException"), content);
    assertTrue(content.contains("catch (IOException $VF$ex)"), content);
    recompile();
  }

  @Test
  public void testMissingCustomExceptionHierarchyIsRetained() throws IOException {
    String content = decompileJasmClass("TestMissingCustomCheckedThrows");
    assertTrue(content.contains("caller() throws MissingFailure"), content);

    Path missingException = writeSource("pkg/MissingFailure.java", """
      package pkg;

      public class MissingFailure extends Exception {
      }
      """);
    recompile(List.of(missingException));
  }

  @Test
  public void testExplicitCustomCheckedTypeWithoutExceptionSuffixIsInferred() throws IOException {
    String content = decompileJasmPair("TestExplicitCustomFailure", "TestExplicitCustomFailureThrower");
    assertTrue(content.contains("throws TestExplicitCustomFailure"), content);
    recompile();
  }

  @Test
  public void testPreciseCatchRethrowDoesNotWidenThrowsClause() throws IOException {
    Path source = fixture.getTestDataDir().resolve("classes/java8/pkg/TestTryWithResources.class");
    assertTrue(Files.isRegularFile(source), "Missing test class: " + source);

    String content = decompileClassFile(source, "pkg/TestTryWithResources.java");
    assertFalse(content.contains("throws Throwable"), content);
    recompile();
  }

  private String decompileJasmClass(String name) throws IOException {
    Path source = fixture.getTestDataDir().resolve("classes/jasm/pkg/" + name + ".class");
    assertTrue(Files.isRegularFile(source), "Missing test class: " + source);
    return decompileClassFile(source, "pkg/" + name + ".java");
  }

  private String decompileJasmPair(String baseName, String childName) throws IOException {
    Path classes = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Path input = fixture.getTempDir().resolve(childName + "/pkg");
    Files.createDirectories(input);
    Files.copy(classes.resolve(baseName + ".class"), input.resolve(baseName + ".class"));
    Files.copy(classes.resolve(childName + ".class"), input.resolve(childName + ".class"));
    return decompileDirectory(input.getParent(), "pkg/" + childName + ".java");
  }
}
