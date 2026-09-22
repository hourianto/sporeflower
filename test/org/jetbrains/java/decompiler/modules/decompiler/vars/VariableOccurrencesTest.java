package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class VariableOccurrencesTest {
  @Test
  void cleanupDiscardsStandaloneCopiesAndPreservesNestedValues() {
    MinimalFernflowerEnvironment.setup();
    try {
      BasicBlockStatement block = BasicBlockStatement.create();
      AssignmentExprent copy = new AssignmentExprent(variable(2, false), variable(1, false), null);
      AssignmentExprent nested = new AssignmentExprent(variable(2, false), variable(1, false), null);
      AssignmentExprent declaration = new AssignmentExprent(variable(4, true), variable(4, false), null);
      block.setExprents(new ArrayList<>(List.of(copy, declaration,
        new ExitExprent(ExitExprent.Type.RETURN, nested, VarType.VARTYPE_INT, null, null))));
      assertTrue(new VariableOccurrences(block, expression -> {}).merge(pair(2), pair(1), VarType.VARTYPE_INT));
      assertTrue(org.jetbrains.java.decompiler.modules.decompiler.SecondaryFunctionsHelper.updateAssignments(block));
      assertEquals(List.of(declaration, block.getExprents().get(1)), block.getExprents());
      assertSame(nested.getRight(), ((ExitExprent)block.getExprents().get(1)).getValue());
      assertFalse(org.jetbrains.java.decompiler.modules.decompiler.SecondaryFunctionsHelper.updateAssignments(block));
    } finally {
      DecompilerContext.setCurrentContext(null);
    }
  }

  @Test
  void chainedMergesUpdateExistingUsesConstantsAndBothKindsOfDeclaration() {
    MinimalFernflowerEnvironment.setup();
    try {
      BasicBlockStatement block = BasicBlockStatement.create();
      VarExprent declaration = variable(1, true);
      VarExprent standalone = variable(2, true);
      VarExprent assigned = variable(3, true);
      VarExprent use = variable(3, false);
      ConstExprent constant = new ConstExprent(VarType.VARTYPE_BYTE, 7, null);
      block.getVarDefinitions().add(declaration);
      block.setExprents(new ArrayList<>(List.of(standalone,
        new AssignmentExprent(assigned, constant, null),
        new ExitExprent(ExitExprent.Type.RETURN, use, VarType.VARTYPE_INT, null, null))));
      VariableOccurrences occurrences = new VariableOccurrences(block, expression -> { });
      assertTrue(occurrences.merge(pair(3), pair(2), VarType.VARTYPE_INT));
      assertFalse(assigned.isDefinition());
      assertEquals(2, use.getIndex());
      assertEquals(VarType.VARTYPE_INT, constant.getConstType());
      assertTrue(occurrences.merge(pair(2), pair(1), VarType.VARTYPE_INT));
      assertEquals(1, assigned.getIndex());
      assertEquals(1, use.getIndex());
      assertFalse(block.getExprents().contains(standalone));
      assertEquals(VarType.VARTYPE_INT, declaration.getVarType());
      assertTrue(occurrences.merge(pair(1), pair(0), VarType.VARTYPE_INT));
      assertTrue(block.getVarDefinitions().isEmpty());
      assertEquals(0, use.getIndex());
    } finally {
      DecompilerContext.setCurrentContext(null);
    }
  }

  private static VarExprent variable(int index, boolean definition) {
    VarExprent variable = new VarExprent(index, VarType.VARTYPE_BYTE, null);
    variable.setDefinition(definition);
    return variable;
  }

  private static VarVersionPair pair(int index) {
    return new VarVersionPair(index, 0);
  }
}
