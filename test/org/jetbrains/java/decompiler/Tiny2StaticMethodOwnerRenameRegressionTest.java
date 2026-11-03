package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

public class Tiny2StaticMethodOwnerRenameRegressionTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
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
""";
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

    String unitContent = readDecompiled("Unit.java");
    String unmappedUnitContent = readDecompiled("UnmappedUnit.java");
    String buildingContent = readDecompiled("Building.java");
    String callerContent = readDecompiled("Caller.java");

    assertContains(unitContent, "public static void resetUnitStaticState()");
    assertNotContains(unitContent, "public static void resetEntityState()");

    assertContains(unmappedUnitContent, "public static void c()");
    assertNotContains(unmappedUnitContent, "public static void resetEntityState()");

    assertContains(buildingContent, "public static void resetBuildingStaticState()",
      "public static void tickBuildingAnimations(int");
    assertNotContains(buildingContent, "public static void resetEntityState()", "public static void tickSharedTimers(int");

    assertContains(callerContent, "Unit.resetUnitStaticState();", "UnmappedUnit.c();",
      "Building.resetBuildingStaticState();", "Building.tickBuildingAnimations(");

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

    String childContent = readDecompiled("ExternalStaticChild.java");
    assertNotContains(childContent, "static void externalCollision()");

    recompile(List.of(externalBase));
  }

  @Test
  public void testRenderedNonOverrideSourceConflictsAreRenamed() throws IOException {
    List<Path> sources = List.of(
      writeSource("StaticInstanceBase.java", """
        class StaticInstanceBase {
          public static void a() {}
        }
        """),
      writeSource("StaticInstanceChild.java", """
        class StaticInstanceChild extends StaticInstanceBase {
          public void b() {}
        }
        """),
      writeSource("FinalStaticBase.java", """
        class FinalStaticBase {
          public static final void a() {}
        }
        """),
      writeSource("FinalStaticChild.java", """
        class FinalStaticChild extends FinalStaticBase {
          public static void b() {}
        }
        """),
      writeSource("WeakerStaticBase.java", """
        class WeakerStaticBase {
          public static void a() {}
        }
        """),
      writeSource("WeakerStaticChild.java", """
        class WeakerStaticChild extends WeakerStaticBase {
          static void b() {}
        }
        """),
      writeSource("InstanceStaticBase.java", """
        class InstanceStaticBase {
          public void a() {}
        }
        """),
      writeSource("InstanceStaticChild.java", """
        class InstanceStaticChild extends InstanceStaticBase {
          public static void b() {}
        }
        """),
      writeSource("PrivateStaticBase.java", """
        class PrivateStaticBase {
          public static void a() {}
        }
        """),
      writeSource("PrivateStaticChild.java", """
        class PrivateStaticChild extends PrivateStaticBase {
          private static void b() {}
        }
        """)
    );
    compileJava8NoDebug(sources, outRoot());
    // Java forbids this static/instance name collision, but the JVM accepts it.
    renameUtf8Constant(outRoot().resolve("InstanceStaticChild.class"), 'b', 'a');
    decompileDirectory(outRoot(), "StaticInstanceChild.java");

    assertContains(readDecompiled("StaticInstanceBase.java"), "public static void renderedCollision()");
    assertNotContains(readDecompiled("StaticInstanceChild.java"), "void renderedCollision()");
    assertContains(readDecompiled("FinalStaticBase.java"), "public static final void finalCollision()");
    assertNotContains(readDecompiled("FinalStaticChild.java"), "void finalCollision()");
    assertContains(readDecompiled("WeakerStaticBase.java"), "public static void accessCollision()");
    assertNotContains(readDecompiled("WeakerStaticChild.java"), "void accessCollision()");
    assertContains(readDecompiled("InstanceStaticBase.java"), "public void instanceStaticCollision()");
    assertNotContains(readDecompiled("InstanceStaticChild.java"), "void instanceStaticCollision()");
    assertContains(readDecompiled("PrivateStaticBase.java"), "public static void privateCollision()");
    assertNotContains(readDecompiled("PrivateStaticChild.java"), "void privateCollision()");
    recompile();
  }
}
