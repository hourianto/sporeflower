package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.api.Decompiler;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.SemanticMappingData;
import org.jetbrains.java.decompiler.main.decompiler.DirectoryResultSaver;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class LegacyClassLiteralPreparedNamesTest extends DecompileRegressionTestBase {
  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void recoveredLegacyLiteralUsesTheAllocatedClassName(boolean semanticReplacement) throws Exception {
    Path input = outRoot().resolve("pkg");
    Files.createDirectories(input);
    Path compiled = fixture.getTestDataDir().resolve("classes/jasm/pkg");
    Files.copy(compiled.resolve("x.class"), input.resolve("x.class"));
    Files.copy(compiled.resolve("y.class"), input.resolve("y.class"));
    ClassReader reader = new ClassReader(Files.readAllBytes(input.resolve("y.class")));
    ClassWriter writer = new ClassWriter(reader, 0);
    reader.accept(writer, 0);
    MethodVisitor ordinary = writer.visitMethod(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC,
      "literalName", "()Ljava/lang/String;", null, null);
    ordinary.visitCode();
    ordinary.visitLdcInsn("pkg.x");
    ordinary.visitInsn(Opcodes.ARETURN);
    ordinary.visitMaxs(1, 0);
    ordinary.visitEnd();
    Files.write(input.resolve("y.class"), writer.toByteArray());
    assertEquals("pkg.x", invoke(outRoot(), "pkg.y", "targetName"));

    int[] instruction = {-1}, literal = {-1};
    new ClassReader(Files.readAllBytes(input.resolve("y.class"))) {
      @Override protected void readBytecodeInstructionOffset(int offset) { instruction[0] = offset; }
    }.accept(new ClassVisitor(Opcodes.ASM9) {
      @Override public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        return !name.equals("targetName") ? null : new MethodVisitor(Opcodes.ASM9) {
          @Override public void visitLdcInsn(Object value) {
            if (value.equals("pkg.x")) literal[0] = instruction[0];
          }
        };
      }
    }, ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
    assertTrue(literal[0] >= 0);

    NamingPlan plan = builder().build().prepareNames();
    String owner = plan.classes().get("pkg/y");
    String replacementName = plan.classes().get("pkg/x").replace('/', '.');
    assertNotEquals("pkg.x", replacementName);
    var replacement = new SemanticMappingData.ClassNameLiteralEntry(
      new SemanticMappingData.TargetEntry("return", owner, "targetName", "()Ljava/lang/String;", null),
      literal[0], "pkg.x", replacementName);
    var ordinaryReplacement = new SemanticMappingData.ClassNameLiteralEntry(
      new SemanticMappingData.TargetEntry("return", owner, "literalName", "()Ljava/lang/String;", null),
      0, "pkg.x", replacementName);
    var semantics = new SemanticMappingData(List.of(), List.of(), List.of(), List.of(), List.of(),
      List.of(), List.of(), List.of(), List.of(), List.of(), List.of(replacement, ordinaryReplacement));
    builder().preparedNames(plan).semanticMappings(semanticReplacement ? semantics : null).build().decompile();
    recompile();
    Path rebuilt = fixture.getTempDir().resolve("recompiled-out");
    assertEquals(replacementName, invoke(rebuilt, owner.replace('/', '.'), "targetName"));
    assertEquals(semanticReplacement ? replacementName : "pkg.x",
      invoke(rebuilt, owner.replace('/', '.'), "literalName"));
  }

  private Decompiler.Builder builder() {
    return Decompiler.builder().inputs(outRoot().toFile())
      .output(new DirectoryResultSaver(fixture.getTargetDir().toFile()))
      .option(IFernflowerPreferences.RENAME_ENTITIES, "1")
      .option(IFernflowerPreferences.BUNDLED_J2ME_API, false);
  }

  private static Object invoke(Path classes, String owner, String method) throws Exception {
    try (URLClassLoader loader = new URLClassLoader(new URL[]{classes.toUri().toURL()}, ClassLoader.getPlatformClassLoader())) {
      return loader.loadClass(owner).getMethod(method).invoke(null);
    }
  }
}
