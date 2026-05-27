package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2StaticMethodOwnerRenameRegressionTest extends DecompileRegressionTestBase {
  private Path mapping;

  @Override
  @BeforeEach
  public void setUp() throws IOException {
    mapping = Files.createTempFile("vf-tiny2-static-owner-", ".tiny");
    Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\ts\tMapEntity
\tm\t()V\tc\tresetEntityState
\tm\t(I)V\ta_\ttickSharedTimers
c\tn\tUnit
\tm\t()V\tc\tresetUnitStaticState
c\tr\tUnmappedUnit
c\tq\tBuilding
\tm\t()V\tc\tresetBuildingStaticState
\tm\t(I)V\ta_\ttickBuildingAnimations
c\tCaller\tCaller
c\tStaticInstanceBase\tStaticInstanceBase
\tm\t()V\ta\trenderedCollision
c\tStaticInstanceChild\tStaticInstanceChild
\tm\t()V\tb\trenderedCollision
c\tFinalStaticBase\tFinalStaticBase
\tm\t()V\ta\tfinalCollision
c\tFinalStaticChild\tFinalStaticChild
\tm\t()V\tb\tfinalCollision
c\tWeakerStaticBase\tWeakerStaticBase
\tm\t()V\ta\taccessCollision
c\tWeakerStaticChild\tWeakerStaticChild
\tm\t()V\tb\taccessCollision
c\tInstanceStaticBase\tInstanceStaticBase
\tm\t()V\ta\tinstanceStaticCollision
c\tInstanceStaticChild\tInstanceStaticChild
\tm\t()V\ta\tinstanceStaticCollision
c\tPrivateStaticBase\tPrivateStaticBase
\tm\t()V\ta\tprivateCollision
c\tPrivateStaticChild\tPrivateStaticChild
\tm\t()V\tb\tprivateCollision
c\tExternalStaticChild\tExternalStaticChild
\tm\t()V\tb\texternalCollision
""", StandardCharsets.UTF_8);

    fixture = new DecompilerTestFixture();
    fixture.setUp(
      IFernflowerPreferences.MAPPINGS_PATH, mapping.toString(),
      IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE, "official",
      IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE, "named"
    );
  }

  @Override
  @AfterEach
  public void tearDown() {
    super.tearDown();
    try {
      if (mapping != null) {
        Files.deleteIfExists(mapping);
      }
    }
    catch (IOException ignored) {
    }
  }

  @Test
  public void testStaticSubclassMethodsKeepOwnMappedNames() throws IOException {
    Path entity = writeSource("s.java", """
class s {
  public static void c() {
  }

  public static void a_(int elapsed) {
  }
}
""");

    Path unit = writeSource("n.java", """
class n extends s {
  public static void c() {
  }
}
""");

    Path building = writeSource("q.java", """
class q extends s {
  public static void c() {
  }

  public static void a_(int elapsed) {
  }
}
""");

    Path unmappedUnit = writeSource("r.java", """
class r extends s {
  public static void c() {
  }
}
""");

    Path caller = writeSource("Caller.java", """
class Caller {
  static void call(int elapsed) {
    s.c();
    n.c();
    r.c();
    q.c();
    q.a_(elapsed);
    s.a_(elapsed);
  }
}
""");

    compileJava8NoDebug(List.of(entity, unit, unmappedUnit, building, caller), outRoot());
    decompileDirectory(outRoot(), "Caller.java");

    String unitContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("Unit.java"));
    String unmappedUnitContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("UnmappedUnit.java"));
    String buildingContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("Building.java"));
    String callerContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("Caller.java"));

    assertTrue(unitContent.contains("public static void resetUnitStaticState()"), unitContent);
    assertFalse(unitContent.contains("public static void resetEntityState()"), unitContent);

    assertTrue(unmappedUnitContent.contains("public static void c()"), unmappedUnitContent);
    assertFalse(unmappedUnitContent.contains("public static void resetEntityState()"), unmappedUnitContent);

    assertTrue(buildingContent.contains("public static void resetBuildingStaticState()"), buildingContent);
    assertTrue(buildingContent.contains("public static void tickBuildingAnimations(int"), buildingContent);
    assertFalse(buildingContent.contains("public static void resetEntityState()"), buildingContent);
    assertFalse(buildingContent.contains("public static void tickSharedTimers(int"), buildingContent);

    assertTrue(callerContent.contains("Unit.resetUnitStaticState();"), callerContent);
    assertTrue(callerContent.contains("UnmappedUnit.c();"), callerContent);
    assertTrue(callerContent.contains("Building.resetBuildingStaticState();"), callerContent);
    assertTrue(callerContent.contains("Building.tickBuildingAnimations("), callerContent);

    recompile();
  }

  @Test
  public void testExternalStaticSuperclassMethodBlocksRenderedChildCollision() throws IOException {
    Path externalBase = writeSource("ExternalStaticBase.java", """
class ExternalStaticBase {
  public static final void externalCollision() {
  }
}
""");

    Path child = writeSource("ExternalStaticChild.java", """
class ExternalStaticChild extends ExternalStaticBase {
  public static void b() {
  }
}
""");

    compileJava8NoDebug(List.of(externalBase, child), outRoot());

    Path libraryOut = fixture.getTempDir().resolve("library-out");
    Files.createDirectories(libraryOut);
    Files.move(outRoot().resolve("ExternalStaticBase.class"), libraryOut.resolve("ExternalStaticBase.class"));

    fixture.getDecompiler().addLibrary(libraryOut.toFile());
    decompileDirectory(outRoot(), "ExternalStaticChild.java");

    String childContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("ExternalStaticChild.java"));
    assertFalse(childContent.contains("static void externalCollision()"), childContent);

    recompile(List.of(externalBase));
  }

  @Test
  public void testRenderedNonOverrideSourceConflictsAreRenamed() throws IOException {
    Path staticInstanceBase = writeSource("StaticInstanceBase.java", """
class StaticInstanceBase {
  public static void a() {
  }
}
""");

    Path staticInstanceChild = writeSource("StaticInstanceChild.java", """
class StaticInstanceChild extends StaticInstanceBase {
  public void b() {
  }
}
""");

    Path finalStaticBase = writeSource("FinalStaticBase.java", """
class FinalStaticBase {
  public static final void a() {
  }
}
""");

    Path finalStaticChild = writeSource("FinalStaticChild.java", """
class FinalStaticChild extends FinalStaticBase {
  public static void b() {
  }
}
""");

    Path weakerStaticBase = writeSource("WeakerStaticBase.java", """
class WeakerStaticBase {
  public static void a() {
  }
}
""");

    Path weakerStaticChild = writeSource("WeakerStaticChild.java", """
class WeakerStaticChild extends WeakerStaticBase {
  static void b() {
  }
}
""");

    Path instanceStaticBase = writeSource("InstanceStaticBase.java", """
class InstanceStaticBase {
  public void a() {
  }
}
""");

    Path instanceStaticChild = writeSource("InstanceStaticChild.java", """
class InstanceStaticChild extends InstanceStaticBase {
  public static void b() {
  }
}
""");

    Path privateStaticBase = writeSource("PrivateStaticBase.java", """
class PrivateStaticBase {
  public static void a() {
  }
}
""");

    Path privateStaticChild = writeSource("PrivateStaticChild.java", """
class PrivateStaticChild extends PrivateStaticBase {
  private static void b() {
  }
}
""");

    compileJava8NoDebug(
      List.of(
        staticInstanceBase,
        staticInstanceChild,
        finalStaticBase,
        finalStaticChild,
        weakerStaticBase,
        weakerStaticChild,
        instanceStaticBase,
        instanceStaticChild,
        privateStaticBase,
        privateStaticChild
      ),
      outRoot()
    );
    renameUtf8Constant(outRoot().resolve("InstanceStaticChild.class"), 'b', 'a');

    decompileDirectory(outRoot(), "StaticInstanceChild.java");
    String staticInstanceBaseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("StaticInstanceBase.java"));
    String staticInstanceChildContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("StaticInstanceChild.java"));
    String finalStaticBaseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("FinalStaticBase.java"));
    String finalStaticChildContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("FinalStaticChild.java"));
    String weakerStaticBaseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("WeakerStaticBase.java"));
    String weakerStaticChildContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("WeakerStaticChild.java"));
    String instanceStaticBaseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("InstanceStaticBase.java"));
    String instanceStaticChildContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("InstanceStaticChild.java"));
    String privateStaticBaseContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("PrivateStaticBase.java"));
    String privateStaticChildContent = DecompilerTestFixture.getContent(fixture.getTargetDir().resolve("PrivateStaticChild.java"));

    assertTrue(staticInstanceBaseContent.contains("public static void renderedCollision()"), staticInstanceBaseContent);
    assertFalse(staticInstanceChildContent.contains("void renderedCollision()"), staticInstanceChildContent);

    assertTrue(finalStaticBaseContent.contains("public static final void finalCollision()"), finalStaticBaseContent);
    assertFalse(finalStaticChildContent.contains("static void finalCollision()"), finalStaticChildContent);

    assertTrue(weakerStaticBaseContent.contains("public static void accessCollision()"), weakerStaticBaseContent);
    assertFalse(weakerStaticChildContent.contains("static void accessCollision()"), weakerStaticChildContent);

    assertTrue(instanceStaticBaseContent.contains("public void instanceStaticCollision()"), instanceStaticBaseContent);
    assertFalse(instanceStaticChildContent.contains("static void instanceStaticCollision()"), instanceStaticChildContent);

    assertTrue(privateStaticBaseContent.contains("public static void privateCollision()"), privateStaticBaseContent);
    assertFalse(privateStaticChildContent.contains("private static void privateCollision()"), privateStaticChildContent);

    recompile();
  }
}
