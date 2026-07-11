package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.decompiler.ConsoleDecompiler;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;

import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;
import java.io.File;
import java.io.IOException;
import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertTrue;

public abstract class DecompileRegressionTestBase {
  protected DecompilerTestFixture fixture;

  @BeforeEach
  public void setUp() throws IOException {
    fixture = new DecompilerTestFixture();
    fixture.setUp(fixtureOptions());
  }

  @AfterEach
  public void tearDown() {
    if (fixture != null) {
      fixture.tearDown();
      fixture = null;
    }
  }

  protected Object[] fixtureOptions() {
    return new Object[0];
  }

  // --- source writing ---

  protected Path srcRoot() {
    return fixture.getTempDir().resolve("compile-src");
  }

  protected Path outRoot() {
    return fixture.getTempDir().resolve("compile-out");
  }

  protected Path writeSource(String relativePath, String code) throws IOException {
    Path source = srcRoot().resolve(relativePath);
    Files.createDirectories(source.getParent());
    Files.writeString(source, code, StandardCharsets.UTF_8);
    return source;
  }

  // --- compilation ---

  protected static void compileJava8(Path source, Path outputDir) throws IOException {
    compileJava8(List.of(source), outputDir);
  }

  protected static void compileJava8(List<Path> sources, Path outputDir) throws IOException {
    compile(sources, outputDir, false);
  }

  protected static void compileJava8NoDebug(Path source, Path outputDir) throws IOException {
    compileJava8NoDebug(List.of(source), outputDir);
  }

  protected static void compileJava8NoDebug(List<Path> sources, Path outputDir) throws IOException {
    compile(sources, outputDir, true);
  }

  private static void compile(List<Path> sources, Path outputDir, boolean noDebug) throws IOException {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    assertNotNull(compiler, "JDK compiler is required to run this test");
    Files.createDirectories(outputDir);

    List<File> sourceFiles = sources.stream().map(Path::toFile).toList();
    try (StandardJavaFileManager fileManager = compiler.getStandardFileManager(null, Locale.ROOT, StandardCharsets.UTF_8)) {
      Iterable<? extends JavaFileObject> units = fileManager.getJavaFileObjectsFromFiles(sourceFiles);
      List<String> options = new ArrayList<>();
      if (noDebug) {
        options.add("-g:none");
      }
      options.addAll(List.of("-source", "8", "-target", "8", "-d", outputDir.toString()));
      Boolean success = compiler.getTask(null, fileManager, null, options, null, units).call();
      assertTrue(Boolean.TRUE.equals(success), "javac failed for " + sources);
    }
  }

  // --- decompile + read ---

  protected String decompileDirectory(Path classDir, String expectedRelativePath) throws IOException {
    ConsoleDecompiler decompiler = fixture.getDecompiler();
    decompiler.addSource(classDir.toFile());
    decompiler.decompileContext();

    Path decompiledFile = fixture.getTargetDir().resolve(expectedRelativePath);
    assertTrue(Files.isRegularFile(decompiledFile), "Decompiled file not found: " + decompiledFile);
    return DecompilerTestFixture.getContent(decompiledFile);
  }

  protected String decompileClassFile(Path classFile, String expectedRelativePath) throws IOException {
    ConsoleDecompiler decompiler = fixture.getDecompiler();
    decompiler.addSource(classFile.toFile());
    decompiler.decompileContext();

    Path decompiledFile = fixture.getTargetDir().resolve(expectedRelativePath);
    if (!Files.isRegularFile(decompiledFile)) {
      // fall back to flat name (no package directory)
      decompiledFile = fixture.getTargetDir().resolve(Path.of(expectedRelativePath).getFileName());
    }
    assertTrue(Files.isRegularFile(decompiledFile), "Decompiled file not found: " + decompiledFile);
    return DecompilerTestFixture.getContent(decompiledFile);
  }

  protected String compileDecompileAndRead(String relativePath, String source) throws IOException {
    Path srcFile = writeSource(relativePath, source);
    compileJava8NoDebug(srcFile, outRoot());
    return decompileDirectory(outRoot(), relativePath);
  }

  // --- recompilation (validate decompiled output) ---

  protected void recompile() throws IOException {
    recompile(List.of());
  }

  protected void recompile(List<Path> extraSources) throws IOException {
    List<Path> sources = new ArrayList<>(listJavaSources(fixture.getTargetDir()));
    sources.addAll(extraSources);
    compileJava8NoDebug(sources, fixture.getTempDir().resolve("recompiled-out"));
  }

  protected static void assertInvocationThrows(
    Path classes,
    Class<? extends Throwable> expectedFailure,
    String implementationName,
    Class<?>[] constructorTypes,
    Object[] constructorArguments,
    String declarationName,
    String methodName,
    Class<?>[] parameterTypes,
    Object[] arguments
  ) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(
      new URL[] {classes.toUri().toURL()},
      ClassLoader.getPlatformClassLoader()
    )) {
      Class<?> implementation = Class.forName(implementationName, true, loader);
      Constructor<?> constructor = implementation.getDeclaredConstructor(constructorTypes);
      constructor.setAccessible(true);
      Object instance = constructor.newInstance(constructorArguments);

      Class<?> declaration = Class.forName(declarationName, true, loader);
      Method method = declaration.getMethod(methodName, parameterTypes);
      try {
        method.invoke(instance, arguments);
        throw new AssertionError("Expected " + expectedFailure.getSimpleName() + " from " + declarationName + "." + methodName);
      }
      catch (InvocationTargetException exception) {
        assertInstanceOf(expectedFailure, exception.getCause());
      }
    }
  }

  // --- utilities ---

  protected static List<Path> listJavaSources(Path root) throws IOException {
    try (Stream<Path> stream = Files.walk(root)) {
      return stream.filter(Files::isRegularFile)
        .filter(p -> p.getFileName().toString().endsWith(".java"))
        .toList();
    }
  }

  protected static void renameUtf8Constant(Path classFile, String from, String to) throws IOException {
    byte[] fromBytes = from.getBytes(StandardCharsets.UTF_8);
    byte[] toBytes = to.getBytes(StandardCharsets.UTF_8);
    assertTrue(fromBytes.length == toBytes.length, "Replacement must keep the classfile UTF8 length unchanged");

    renameUtf8ConstantBytes(classFile, fromBytes, toBytes);
  }

  protected static void renameUtf8Constant(Path classFile, char from, char to) throws IOException {
    renameUtf8ConstantBytes(classFile, new byte[]{(byte)from}, new byte[]{(byte)to});
  }

  private static void renameUtf8ConstantBytes(Path classFile, byte[] from, byte[] to) throws IOException {
    byte[] bytes = Files.readAllBytes(classFile);
    boolean replaced = false;

    for (int i = 0; i <= bytes.length - from.length - 2; i++) {
      if (bytes[i] != 0 || Byte.toUnsignedInt(bytes[i + 1]) != from.length) {
        continue;
      }

      if (Arrays.equals(bytes, i + 2, i + 2 + from.length, from, 0, from.length)) {
        System.arraycopy(to, 0, bytes, i + 2, to.length);
        replaced = true;
        break;
      }
    }

    assertTrue(replaced, "Missing UTF8 constant '" + new String(from, StandardCharsets.UTF_8) + "' in " + classFile);
    Files.write(classFile, bytes);
  }
}
