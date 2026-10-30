package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.struct.gen.FieldDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;

import static org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent.FunctionType.*;
import static org.junit.jupiter.api.Assertions.*;

class InlineAssignmentHoistingTest {
  @BeforeEach void setUp() { MinimalFernflowerEnvironment.setup(); }
  @AfterEach void tearDown() { DecompilerContext.setCurrentContext(null); }

  private static List<Exprent> operands(Exprent... values) { return new ArrayList<>(List.of(values)); }

  private static VarExprent local(int index) { return new VarExprent(index, VarType.VARTYPE_OBJECT, null); }
  private static ConstExprent nil() { return new ConstExprent(VarType.VARTYPE_NULL, null, null); }
  private static Exprent update() { return new AssignmentExprent(local(0), local(1), null); }
  private static Exprent castUpdate() {
    return new FunctionExprent(CAST, operands(update(), new ConstExprent(VarType.VARTYPE_STRING, null, null)), null);
  }

  private static BasicBlockStatement simplify(Exprent expression) {
    var block = new BasicBlockStatement(new BasicBlock(1));
    block.setExprents(operands(expression));
    SimplifyExprentsHelper.simplifyStackVarsStatement(block, new HashSet<>(), null, null, false);
    return block;
  }

  private static void assertUnmoved(Exprent expression) {
    Exprent before = expression.copy();
    var block = simplify(expression);
    assertEquals(1, block.getExprents().size(), "Must not move a write before an earlier evaluation");
    assertEquals(before, block.getExprents().get(0));
  }

  private static void assertHoisted(Exprent expression) {
    var block = simplify(expression);
    assertEquals(2, block.getExprents().size(), "Keep readable standalone assignments when evaluations commute");
    assertInstanceOf(AssignmentExprent.class, block.getExprents().get(0));
  }

  @Test void staticFieldStoreKeepsStandaloneAssignment() {
    Exprent field = new FieldExprent("value", "Holder", true, null, FieldDescriptor.parseDescriptor("Ljava/lang/Object;"), null);
    assertHoisted(new AssignmentExprent(field, castUpdate(), null));
  }

  @Test void independentReceiverAndArrayTargetKeepStandaloneAssignments() {
    Exprent field = new FieldExprent("value", "Holder", false, local(2), FieldDescriptor.parseDescriptor("Ljava/lang/Object;"), null);
    assertHoisted(new AssignmentExprent(field, castUpdate(), null));
    Exprent array = new ArrayExprent(local(2), new VarExprent(3, VarType.VARTYPE_INT, null), VarType.VARTYPE_OBJECT, null);
    assertHoisted(new AssignmentExprent(array, castUpdate(), null));
  }

  @Test void independentEarlierLocalAndLiteralCanBeCrossed() {
    assertHoisted(new FunctionExprent(EQ, operands(local(2), castUpdate()), null));
    assertHoisted(new FunctionExprent(EQ, operands(new ConstExprent(VarType.VARTYPE_STRING, "text", null), castUpdate()), null));
  }

  @Test void independentArrayIndexAssignmentCanBeHoisted() {
    Exprent index = new FunctionExprent(TERNARY,
      operands(new FunctionExprent(EQ, operands(update(), nil()), null),
        new ConstExprent(0, false, null), new ConstExprent(1, false, null)), null);
    assertHoisted(new ArrayExprent(local(2), index, VarType.VARTYPE_OBJECT, null));
  }

  @Test void nestedWritesCannotChangeEarlierReads() {
    var earlier = local(2);
    earlier.setVersion(1);
    var overwritten = local(2);
    overwritten.setVersion(2);
    Exprent nested = new AssignmentExprent(local(0), new AssignmentExprent(overwritten, local(1), null), null);
    Exprent cast = new FunctionExprent(CAST, operands(nested, new ConstExprent(VarType.VARTYPE_STRING, null, null)), null);
    assertUnmoved(new FunctionExprent(EQ, operands(earlier, cast), null));
  }

  @Test void nestedIncrementCannotChangeEarlierArrayIndex() {
    var index = new VarExprent(2, VarType.VARTYPE_INT, null);
    Exprent increment = new FunctionExprent(PPI, index.copy(), null);
    Exprent write = new AssignmentExprent(new VarExprent(0, VarType.VARTYPE_INT, null), increment, null);
    Exprent cast = new FunctionExprent(CAST, operands(write, new ConstExprent(VarType.VARTYPE_INT, null, null)), null);
    Exprent target = new ArrayExprent(local(3), index, VarType.VARTYPE_INT, null);
    assertUnmoved(new AssignmentExprent(target, cast, null));
  }

  @Test void compoundFieldReadCannotBeCrossed() {
    Exprent field = new FieldExprent("value", "Holder", true, null, FieldDescriptor.parseDescriptor("I"), null);
    Exprent write = new AssignmentExprent(new VarExprent(0, VarType.VARTYPE_INT, null), new ConstExprent(2, false, null), null);
    Exprent cast = new FunctionExprent(CAST, operands(write, new ConstExprent(VarType.VARTYPE_INT, null, null)), null);
    assertUnmoved(new AssignmentExprent(field, cast, ADD, null));
  }

  @Test void smallIntegerLiteralsAndIndependentArithmeticCanBeCrossed() {
    for (int literal : new int[]{0, 1, 55, 200, 32768}) {
      Exprent write = new AssignmentExprent(new VarExprent(0, VarType.VARTYPE_INT, null), new ConstExprent(2, false, null), null);
      Exprent cast = new FunctionExprent(CAST, operands(write, new ConstExprent(VarType.VARTYPE_INT, null, null)), null);
      assertHoisted(new FunctionExprent(LT, operands(new ConstExprent(literal, false, null), cast), null));
    }
    Exprent prefix = new FunctionExprent(MUL, operands(new VarExprent(2, VarType.VARTYPE_INT, null), new ConstExprent(2, false, null)), null);
    assertHoisted(new ArrayExprent(local(3), new FunctionExprent(ADD,
      operands(prefix, new FunctionExprent(TERNARY, operands(new FunctionExprent(EQ, operands(update(), nil()), null),
        new ConstExprent(0, false, null), new ConstExprent(1, false, null)), null)), null), VarType.VARTYPE_OBJECT, null));
  }

  @Test void arithmeticReadsStillProtectTheirSlots() {
    Exprent prefix = new FunctionExprent(ADD, operands(new VarExprent(0, VarType.VARTYPE_INT, null), new ConstExprent(1, false, null)), null);
    Exprent write = new AssignmentExprent(new VarExprent(0, VarType.VARTYPE_INT, null), new ConstExprent(2, false, null), null);
    assertUnmoved(new FunctionExprent(ADD, operands(prefix,
      new FunctionExprent(CAST, operands(write, new ConstExprent(VarType.VARTYPE_INT, null, null)), null)), null));
  }

  @Test void divisionAndFieldReadsRemainBarriers() {
    Exprent division = new FunctionExprent(DIV, operands(new VarExprent(2, VarType.VARTYPE_INT, null), new ConstExprent(0, false, null)), null);
    Exprent field = new FieldExprent("value", "Holder", true, null, FieldDescriptor.parseDescriptor("Ljava/lang/Object;"), null);
    Exprent write = new AssignmentExprent(new VarExprent(0, VarType.VARTYPE_INT, null), new ConstExprent(2, false, null), null);
    Exprent cast = new FunctionExprent(CAST, operands(write, new ConstExprent(VarType.VARTYPE_INT, null, null)), null);
    assertUnmoved(new FunctionExprent(ADD, operands(division, cast), null));
    assertUnmoved(new FunctionExprent(EQ, operands(field, castUpdate()), null));
  }

  @Test void earlierLocalReadMustKeepItsOldValue() {
    // x == (String)(x = y): a local read is side-effect-free but cannot commute with this write.
    assertUnmoved(new FunctionExprent(EQ, operands(local(0), castUpdate()), null));
  }

  @Test void earlierArrayReadCanThrowBeforeTheAssignment() {
    Exprent read = new ArrayExprent(local(2), new ConstExprent(0, false, null), VarType.VARTYPE_OBJECT, null);
    assertUnmoved(new FunctionExprent(EQ, operands(read, castUpdate()), null));
  }

  @Test void arrayIndexCannotChangeTheAlreadyEvaluatedBase() {
    Exprent index = new FunctionExprent(TERNARY,
      operands(new FunctionExprent(EQ, operands(update(), nil()), null),
        new ConstExprent(0, false, null), new ConstExprent(1, false, null)), null);
    assertUnmoved(new ArrayExprent(local(0), index, VarType.VARTYPE_OBJECT, null));
  }

  @Test void arrayStoreMustCaptureItsTargetBeforeTheRhs() {
    Exprent target = new ArrayExprent(local(0), new ConstExprent(0, false, null), VarType.VARTYPE_OBJECT, null);
    assertUnmoved(new AssignmentExprent(target, castUpdate(), null));
  }

  @Test void fieldStoreMustCaptureItsReceiverBeforeTheRhs() {
    Exprent target = new FieldExprent("value", "Holder", false, local(0), FieldDescriptor.parseDescriptor("Ljava/lang/Object;"), null);
    assertUnmoved(new AssignmentExprent(target, castUpdate(), null));
  }

  @Test void conditionalOperandsStayConditional() {
    for (var type : List.of(BOOLEAN_AND, BOOLEAN_OR)) {
      assertUnmoved(new FunctionExprent(type, operands(
        new VarExprent(2, VarType.VARTYPE_BOOLEAN, null),
        new FunctionExprent(EQ, operands(update(), nil()), null)), null));
    }
    for (int branch = 1; branch <= 2; branch++) {
      var operands = operands(new VarExprent(2, VarType.VARTYPE_BOOLEAN, null), nil(), nil());
      operands.set(branch, castUpdate());
      assertUnmoved(new FunctionExprent(TERNARY, operands, null));
    }
  }

  @Test void compoundStoreReadsItsOldValueBeforeTheRhs() {
    var value = new VarExprent(0, VarType.VARTYPE_INT, null);
    var write = new AssignmentExprent(value.copy(), new ConstExprent(2, false, null), null);
    var cast = new FunctionExprent(CAST, operands(write, new ConstExprent(VarType.VARTYPE_INT, null, null)), null);
    assertUnmoved(new AssignmentExprent(value, cast, ADD, null));
  }

  @Test void nullOnTheLeftDoesNotPreventSafeHoisting() {
    var block = simplify(new FunctionExprent(EQ, operands(nil(), update()), null));
    assertEquals(2, block.getExprents().size());
  }

  @Test void firstConditionalOperandCanStillBeHoisted() {
    for (var type : List.of(BOOLEAN_AND, BOOLEAN_OR, TERNARY)) {
      var values = operands(new FunctionExprent(EQ, operands(update(), nil()), null),
        new ConstExprent(1, true, null));
      if (type == TERNARY) values.add(new ConstExprent(0, true, null));
      assertEquals(2, simplify(new FunctionExprent(type, values, null)).getExprents().size());
    }
  }

  @Test void arrayBaseAssignmentCanStillBeHoisted() {
    assertEquals(2, simplify(new ArrayExprent(update(), new ConstExprent(0, false, null),
      VarType.VARTYPE_OBJECT, null)).getExprents().size());
  }

  @Test void leadingAssignmentCanStillBeHoisted() {
    var block = simplify(new FunctionExprent(EQ, operands(castUpdate(), local(2)), null));
    assertEquals(2, block.getExprents().size());
    assertInstanceOf(AssignmentExprent.class, block.getExprents().get(0));
  }
}
