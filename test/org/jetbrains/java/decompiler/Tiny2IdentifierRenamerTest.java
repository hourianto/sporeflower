package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.modules.renamer.Tiny2IdentifierRenamer;
import org.junit.jupiter.api.Test;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Tiny2IdentifierRenamerTest {
  @Test
  public void testTiny2MappingRenamesClassFieldAndMethod() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\taf\tdefpackage/GameEngine
\tf\tI\ta\tcounter
\tm\t(IJ)I\tc\ttick
\t\tp\t1\tp0\tframe
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, "official", "named");

      assertTrue(renamer.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_CLASS, "af", null, null));
      assertEquals("defpackage/GameEngine", renamer.getNextClassName("af", "af"));

      assertTrue(renamer.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_FIELD, "af", "a", "I"));
      assertEquals("counter", renamer.getNextFieldName("af", "a", "I"));

      assertTrue(renamer.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_METHOD, "af", "c", "(IJ)I"));
      assertEquals("tick", renamer.getNextMethodName("af", "c", "(IJ)I"));
      assertFalse(renamer.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_METHOD, "af", "<init>", "()V"));
      assertEquals(1, renamer.parameterRenameCount());
      assertEquals("frame", renamer.getParameterRename("af", "c", "(IJ)I", 1));
      assertEquals("frame", renamer.getParameterRename("defpackage/GameEngine", "tick", "(IJ)I", 1));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testTiny2MappingResolvesConfiguredNamespaces() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tintermediary\tnamed
c\taf\tafInter\tdefpackage/GameEngine
\tm\t(I)V\ta\taInter\tsetValue
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, null, "named");

      assertTrue(renamer.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_CLASS, "af", null, null));
      assertEquals("defpackage/GameEngine", renamer.getNextClassName("af", "af"));
      assertTrue(renamer.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_METHOD, "af", "a", "(I)V"));
      assertEquals("setValue", renamer.getNextMethodName("af", "a", "(I)V"));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testTiny2MappingRejectsConflictingDuplicates() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\taf\tdefpackage/GameEngine
c\taf\tdefpackage/OtherName
""", StandardCharsets.UTF_8);

      assertThrows(IOException.class, () -> Tiny2IdentifierRenamer.fromFile(mapping, "official", "named"));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testRenamerAddsStableSuffixOnRetryCalls() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\taf\tdefpackage/GameEngine
\tf\tI\ta\tcounter
\tm\t(I)V\tb\ttick
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, "official", "named");

      assertEquals("defpackage/GameEngine", renamer.getNextClassName("af", "af"));
      assertEquals("defpackage/GameEngine_1", renamer.getNextClassName("af", "af"));

      assertEquals("counter", renamer.getNextFieldName("af", "a", "I"));
      assertEquals("counter_1", renamer.getNextFieldName("af", "a", "I"));

      assertEquals("tick", renamer.getNextMethodName("af", "b", "(I)V"));
      assertEquals("tick_1", renamer.getNextMethodName("af", "b", "(I)V"));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testTiny2ParameterLookupSupportsMappedMethodDescriptor() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\taf\tpkg/Entity
\tm\t(Laf;I)Laf;\ta\tcreate
\t\tp\t1\tp0\tentityType
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, "official", "named");

      assertEquals("entityType", renamer.getParameterRename("af", "a", "(Laf;I)Laf;", 1));
      assertEquals("entityType", renamer.getParameterRename("pkg/Entity", "create", "(Lpkg/Entity;I)Lpkg/Entity;", 1));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testTiny2ParameterLookupHandlesForwardClassRenameInDescriptor() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\ta\tpkg/A
\tm\t(Lb;)V\ta\tuse
\t\tp\t1\tp0\tvalue
c\tb\tpkg/B
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, "official", "named");

      assertEquals("value", renamer.getParameterRename("a", "a", "(Lb;)V", 1));
      assertEquals("value", renamer.getParameterRename("pkg/A", "use", "(Lpkg/B;)V", 1));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testTiny2ParameterLookupSupportsMixedRenamePhaseKeys() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
c\tag\tdefpackage/GameLevel
\tm\t(IIIII)Lj;\ta\tcreateEntity
\t\tp\t1\tp0\tentityType
\t\tp\t2\tp1\ttileX
\t\tp\t3\tp2\ttileY
\t\tp\t4\tp3\tlayerIndex
\t\tp\t5\tp4\tdirection
c\tj\tdefpackage/Entity
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, "official", "named");

      assertEquals("entityType", renamer.getParameterRename("ag", "a", "(IIIII)Lj;", 1));
      assertEquals("entityType", renamer.getParameterRename("defpackage/GameLevel", "createEntity", "(IIIII)Lj;", 1));
      assertEquals("entityType", renamer.getParameterRename("defpackage/GameLevel", "createEntity", "(IIIII)Ldefpackage/Entity;", 1));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testTiny2EscapedNamesPropertyDecodesEscapes() throws IOException {
    Path mapping = Files.createTempFile("vf-tiny2-", ".tiny");
    try {
      Files.writeString(mapping, """
tiny\t2\t0\tofficial\tnamed
\tescaped-names
c\taf\tdefpackage/GameEngine
\tm\t(I)V\ta\tset\\tvalue
""", StandardCharsets.UTF_8);

      Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(mapping, "official", "named");
      assertEquals("set\tvalue", renamer.getNextMethodName("af", "a", "(I)V"));
    } finally {
      Files.deleteIfExists(mapping);
    }
  }

  @Test
  public void testMappedParameterNameResolutionPreservesCollisionAdjustedConcreteNames() {
    assertEquals("pausedx", resolveMappedParameterName(0, "pausedx", "paused"));
    assertEquals("value_2", resolveMappedParameterName(0, "value_2", "value"));
    assertEquals("state", resolveMappedParameterName(0, "state", "state"));
    assertEquals("paused", resolveMappedParameterName(0, "a", "paused"));
  }

  @Test
  public void testMappedParameterNameResolutionForcesMappingForAbstractNativeMethods() {
    assertEquals("paused", resolveMappedParameterName(CodeConstants.ACC_ABSTRACT, "pausedx", "paused"));
    assertEquals("paused", resolveMappedParameterName(CodeConstants.ACC_NATIVE, "pausedx", "paused"));
  }

  private static String resolveMappedParameterName(int flags, String currentName, String mappedName) {
    try {
      Method method = Tiny2IdentifierRenamer.class.getDeclaredMethod(
        "resolveMappedParameterName",
        int.class,
        String.class,
        String.class
      );
      method.setAccessible(true);
      return (String)method.invoke(null, flags, currentName, mappedName);
    }
    catch (NoSuchMethodException | IllegalAccessException e) {
      throw new AssertionError("Failed to invoke Tiny2IdentifierRenamer.resolveMappedParameterName", e);
    }
    catch (InvocationTargetException e) {
      Throwable cause = e.getCause() != null ? e.getCause() : e;
      throw new AssertionError("resolveMappedParameterName threw", cause);
    }
  }
}
