package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.Instruction;
import org.jetbrains.java.decompiler.main.Init;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.vars.StackMapTypeEvidence;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DataInputFullStream;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

public class StackMapTypeEvidenceTest implements CodeConstants {
  private static final VarType BOOLEAN_ARRAY = new VarType("[Z");

  @BeforeEach
  public void setUp() {
    MinimalFernflowerEnvironment.setup();
    Init.init();
  }

  @Test
  public void frameOnStoreConstrainsThePreviousDefinition() throws Exception {
    StructMethod method = method("TestStackMapDefinitionEvidence", "overwrite", "(Z)I");
    StackMapTypeEvidence evidence = StackMapTypeEvidence.analyze(method);
    List<Instruction> stores = instructions(method, opc_astore);
    assertEquals(Set.of(BOOLEAN_ARRAY), evidence.getWriteTypes(stores.get(0)));
    assertEquals(Set.of(VarType.VARTYPE_NULL), evidence.getWriteTypes(stores.get(1)));
    assertEquals(Set.of(VarType.VARTYPE_NULL), evidence.getReadTypes(instructions(method, opc_aload).get(0)));
  }

  @Test
  public void controlFlowRatherThanOffsetOrderDeterminesSlotTypes() throws Exception {
    StructMethod method = method("TestLegacyStackMapSlotProbe", "test", "(Ljava/lang/Object;I)I");
    StackMapTypeEvidence evidence = StackMapTypeEvidence.analyze(method);
    assertEquals(Set.of(VarType.VARTYPE_OBJECT), evidence.getWriteTypes(instructions(method, opc_astore).get(0)));
    assertEquals(Set.of(VarType.VARTYPE_INT), evidence.getWriteTypes(instructions(method, opc_istore).get(0)));
    assertEquals(Set.of(VarType.VARTYPE_INT), evidence.getReadTypes(instructions(method, opc_iload).get(0)));
  }

  @Test
  public void loopFramesReachBothInitialAndIncrementDefinitions() throws Exception {
    StructMethod method = method("TestStackMapDefinitionEvidence", "loop", "(I)I");
    StackMapTypeEvidence evidence = StackMapTypeEvidence.analyze(method);
    Instruction increment = instructions(method, opc_iinc).get(0);
    assertEquals(Set.of(VarType.VARTYPE_INT), evidence.getWriteTypes(instructions(method, opc_istore).get(0)));
    assertEquals(Set.of(VarType.VARTYPE_INT), evidence.getReadTypes(increment));
    assertEquals(Set.of(VarType.VARTYPE_INT), evidence.getWriteTypes(increment));
  }

  @Test
  public void handlerFramesReachDefinitionsThroughoutProtectedBlocks() throws Exception {
    StructMethod method = method("TestStackMapDefinitionEvidence", "protectedReuse", "(I)I");
    StackMapTypeEvidence evidence = StackMapTypeEvidence.analyze(method);
    List<Instruction> stores = instructions(method, opc_astore);
    assertEquals(Set.of(VarType.VARTYPE_NULL, VarType.VARTYPE_OBJECT), evidence.getWriteTypes(stores.get(0)));
    assertEquals(Set.of(VarType.VARTYPE_OBJECT), evidence.getWriteTypes(stores.get(1)));
    assertTrue(evidence.getWriteTypes(stores.get(2)).isEmpty(), "Handler's own store has no local frame constraint");
  }

  @Test
  public void wideSlotsAndClonedAccessesKeepTheirIdentity() throws Exception {
    StructMethod method = method("TestStackMapDefinitionEvidence", "wideReuse", "(I)I");
    StackMapTypeEvidence evidence = StackMapTypeEvidence.analyze(method);
    var frames = method.getAttribute(StructGeneralAttribute.ATTRIBUTE_STACK_MAP).getEntries();
    assertEquals(VarType.VARTYPE_LONG, frames.get(0).getLocalType(1));
    assertNull(frames.get(0).getLocalType(2));
    assertNull(frames.get(1).getLocalType(1));
    assertEquals(VarType.VARTYPE_INT, frames.get(1).getLocalType(2));
    assertEquals(Set.of(VarType.VARTYPE_LONG), evidence.getWriteTypes(instructions(method, opc_lstore).get(0)));
    Instruction store = instructions(method, opc_istore).get(0);
    assertEquals(Set.of(VarType.VARTYPE_INT), evidence.getWriteTypes(store.clone()));
    Instruction synthetic = Instruction.create(opc_istore, false, GROUP_GENERAL, store.bytecodeVersion, new int[]{2}, -1, 1);
    assertTrue(evidence.getWriteTypes(synthetic).isEmpty());
  }

  @Test
  public void evidenceSurvivesExpressionCopiesAndRenumbering() throws Exception {
    StructMethod method = method("TestStackMapDefinitionEvidence", "delayedArray", "(I)Z");
    var evidence = StackMapTypeEvidence.analyze(method);
    var original = new VarExprent(1, VarType.VARTYPE_OBJECT, null);
    original.setStackMapTypes(evidence.getReadTypes(instructions(method, opc_aload).get(0)));
    var copy = (VarExprent)original.copy();
    copy.setIndex(73);
    copy.setVersion(4);
    assertEquals(Set.of(BOOLEAN_ARRAY), copy.getStackMapTypes());
  }

  private static List<Instruction> instructions(StructMethod method, int opcode) {
    return method.getInstructionSequence().instructions().stream().filter(instruction -> instruction.opcode == opcode).toList();
  }

  private static StructMethod method(String className, String name, String descriptor) throws Exception {
    Path file = new DecompilerTestFixture().getTestDataDir().resolve("classes/jasm/pkg/" + className + ".class");
    StructClass owner;
    try (DataInputFullStream input = new DataInputFullStream(Files.readAllBytes(file))) {
      owner = StructClass.create(input, true);
    }
    StructMethod method = owner.getMethod(name, descriptor);
    assertNotNull(method);
    method.expandData(owner);
    return method;
  }
}
