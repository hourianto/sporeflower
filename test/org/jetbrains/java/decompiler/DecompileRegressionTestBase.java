package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.decompiler.ConsoleDecompiler;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;

import javax.tools.JavaCompiler;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
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
import static org.junit.jupiter.api.Assertions.assertFalse;
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

  protected Object[] fixtureOptions() throws IOException {
    return new Object[0];
  }

  protected void configureOptions(Object... options) {
    fixture.configureOptions(options);
  }

  // --- source writing ---

  protected Path srcRoot() {
    return fixture.getTempDir().resolve("compile-src");
  }

  protected Path outRoot() {
    return fixture.getTempDir().resolve("compile-out");
  }

  protected Path recompiledRoot() {
    return fixture.getTempDir().resolve("recompiled-out");
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
    compile(sources, outputDir, (String)null);
  }

  protected static void compileJava8WithDebug(Path source, Path outputDir) throws IOException {
    compile(List.of(source), outputDir, "-g");
  }

  protected static void compileJava8NoDebug(Path source, Path outputDir) throws IOException {
    compileJava8NoDebug(List.of(source), outputDir);
  }

  protected static void compileJava8NoDebug(List<Path> sources, Path outputDir) throws IOException {
    compile(sources, outputDir, "-g:none");
  }

  private static void compile(List<Path> sources, Path outputDir, String debugOption) throws IOException {
    List<String> options = new ArrayList<>(List.of("-source", "8", "-target", "8"));
    if (debugOption != null) options.add(debugOption);
    compile(sources, outputDir, options);
  }

  protected static void compileJava8ReleaseWithClasspath(List<Path> sources, Path outputDir, Path... classpath) throws IOException {
    List<String> options = new ArrayList<>(List.of("--release", "8"));
    if (classpath.length > 0) {
      options.addAll(List.of("-classpath", String.join(File.pathSeparator, Arrays.stream(classpath).map(Path::toString).toList())));
    }
    compile(sources, outputDir, options);
  }

  private static void compile(List<Path> sources, Path outputDir, List<String> options) throws IOException {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    assertNotNull(compiler, "JDK compiler is required to run this test");
    Files.createDirectories(outputDir);

    List<File> sourceFiles = sources.stream().map(Path::toFile).toList();
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    try (StandardJavaFileManager fileManager = compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
      // Fixture programs depend on the JDK and their explicitly supplied
      // sources/libraries, never Gradle's test runner or its dependency jars.
      // Besides enforcing that boundary, this avoids reopening those archives
      // for every small fixture compilation. Explicit -classpath options below
      // still override this default for library-resolution regressions.
      fileManager.setLocationFromPaths(StandardLocation.CLASS_PATH, List.of());
      Iterable<? extends JavaFileObject> units = fileManager.getJavaFileObjectsFromFiles(sourceFiles);
      options.addAll(List.of("-d", outputDir.toString()));
      Boolean success = compiler.getTask(null, fileManager, diagnostics, options, null, units).call();
      assertTrue(Boolean.TRUE.equals(success), () -> "javac failed for " + sources + "\n" + diagnostics.getDiagnostics());
    }
  }

  // --- decompile + read ---

  protected String decompileDirectory(Path classDir, String expectedRelativePath, Object... options) throws IOException {
    configureOptions(options);
    ConsoleDecompiler decompiler = fixture.getDecompiler();
    decompiler.addSource(classDir.toFile());
    decompiler.decompileContext();

    return readDecompiled(expectedRelativePath);
  }

  protected String decompileClassFile(Path classFile, String expectedRelativePath, Object... options) throws IOException {
    configureOptions(options);
    ConsoleDecompiler decompiler = fixture.getDecompiler();
    decompiler.addSource(classFile.toFile());
    decompiler.decompileContext();

    // The legacy single-class saver flattens output; directory decompilation
    // and named output reads must still enforce the requested package path.
    Path output = fixture.getTargetDir().resolve(expectedRelativePath);
    if (!Files.isRegularFile(output)) output = fixture.getTargetDir().resolve(Path.of(expectedRelativePath).getFileName());
    assertTrue(Files.isRegularFile(output), "Decompiled file not found: " + output);
    String source = DecompilerTestFixture.getContent(output);
    assertDecompiled(source);
    return source;
  }

  protected String readDecompiled(String expectedRelativePath) {
    Path decompiledFile = fixture.getTargetDir().resolve(expectedRelativePath);
    assertTrue(Files.isRegularFile(decompiledFile), "Decompiled file not found: " + decompiledFile);
    String source = DecompilerTestFixture.getContent(decompiledFile);
    assertDecompiled(source);
    return source;
  }

  private static void assertDecompiled(String source) {
    assertNotContains(source, "$VF: Couldn't be decompiled");
  }

  protected String readAllDecompiled() throws IOException {
    List<Path> sources = listJavaSources(fixture.getTargetDir());
    assertFalse(sources.isEmpty(), "No decompiled sources found");
    StringBuilder content = new StringBuilder();
    for (Path source : sources) {
      content.append(readDecompiled(fixture.getTargetDir().relativize(source).toString())).append('\n');
    }
    return content.toString();
  }

  protected String decompileFixture(String name, Object... options) throws IOException {
    Path classFile = fixture.getTestDataDir().resolve("classes/jasm/pkg/" + name + ".class");
    assertTrue(Files.isRegularFile(classFile), "Missing fixture: " + classFile);
    return decompileClassFile(classFile, "pkg/" + name + ".java", options);
  }

  protected Path copyJasmClasses(String... names) throws IOException {
    Path root = fixture.getTempDir().resolve("input-classes");
    Files.createDirectories(root.resolve("pkg"));
    for (String name : names) {
      Files.copy(fixture.getTestDataDir().resolve("classes/jasm/pkg/" + name + ".class"), root.resolve("pkg/" + name + ".class"));
    }
    return root;
  }

  protected String compileDecompileAndRead(String relativePath, String source, Object... options) throws IOException {
    Path srcFile = writeSource(relativePath, source);
    compileJava8NoDebug(srcFile, outRoot());
    return decompileDirectory(outRoot(), relativePath, options);
  }

  // --- recompilation (validate decompiled output) ---

  protected void recompile() throws IOException {
    recompile(List.of());
  }

  protected void recompile(List<Path> extraSources) throws IOException {
    List<Path> sources = new ArrayList<>(listJavaSources(fixture.getTargetDir()));
    sources.addAll(extraSources);
    compileJava8NoDebug(sources, recompiledRoot());
  }

  protected BehaviorComparison compareBehavior(Path original) throws IOException {
    return new BehaviorComparison(original, recompiledRoot());
  }

  protected BehaviorComparison compareBehavior() throws IOException {
    return compareBehavior(outRoot());
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

  protected static URLClassLoader loader(Path... directories) throws IOException {
    URL[] urls = new URL[directories.length];
    for (int i = 0; i < directories.length; i++) urls[i] = directories[i].toUri().toURL();
    // Test fixtures must never resolve against the decompiler/test classpath.
    return new URLClassLoader(urls, ClassLoader.getPlatformClassLoader());
  }

  protected static int countOccurrences(String text, String needle) {
    if (needle.isEmpty()) throw new IllegalArgumentException("Empty search string");
    int count = 0;
    for (int offset = 0; (offset = text.indexOf(needle, offset)) >= 0; offset += needle.length()) count++;
    return count;
  }

  protected static void assertContains(String source, String... snippets) {
    for (String snippet : snippets) assertTrue(source.contains(snippet), () -> "Missing: " + snippet + "\n" + source);
  }

  protected static void assertNotContains(String source, String... snippets) {
    for (String snippet : snippets) assertFalse(source.contains(snippet), () -> "Unexpected: " + snippet + "\n" + source);
  }

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
