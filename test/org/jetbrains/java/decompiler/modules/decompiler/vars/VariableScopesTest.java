package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.SequenceStatement;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;

class VariableScopesTest {
  @BeforeEach
  void setUp() {
    MinimalFernflowerEnvironment.setup();
  }

  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  void resourcesAndCatchBindingsBelongOnlyToTheirOwnBodies() {
    CatchStatement caught = CatchStatement.createSourceOnly(block(1, 2), List.of(block(11), block(21)),
      List.of(List.of("java/lang/Exception"), List.of("java/lang/Error")), List.of(variable(10), variable(20)));
    caught.getResources().add(new AssignmentExprent(variable(30), new ConstExprent(VarType.VARTYPE_NULL, null, null), null));
    List<String> candidates = new ArrayList<>();
    new VariableScopes(VariableScopesTest::original, (declaration, targets) -> {
      candidates.add(describe(declaration, targets));
      return false;
    }).process(caught, Map.of());
    assertEquals(List.of("1:[30]", "2:[30, 1]", "11:[10]", "21:[20]"), candidates);
  }

  @Test
  void transparentSequencesExportSurvivingLocalsAfterMerges() {
    SequenceStatement nested = new SequenceStatement(List.of(block(1), block(2)));
    SequenceStatement outer = new SequenceStatement(List.of(nested, block(3)));
    List<String> candidates = new ArrayList<>();
    new VariableScopes(VariableScopesTest::original, (declaration, targets) -> {
      candidates.add(describe(declaration, targets));
      return !targets.isEmpty();
    }).process(outer, Map.of());
    assertEquals(List.of("1:[]", "2:[1]", "3:[1]"), candidates);
  }

  @Test
  void labeledSequencesExportHoistedDeclarationsButNotTheirContents() {
    BasicBlockStatement body = block(1);
    SequenceStatement labeled = new SequenceStatement(List.of(body));
    BasicBlockStatement after = block(2);
    labeled.getLabelEdges().add(new StatEdge(StatEdge.TYPE_BREAK, body, after, labeled));
    labeled.getVarDefinitions().add(variable(10));
    List<String> candidates = new ArrayList<>();
    new VariableScopes(VariableScopesTest::original, (declaration, targets) -> {
      candidates.add(describe(declaration, targets));
      return false;
    }).process(new SequenceStatement(List.of(labeled, after)), Map.of());
    assertEquals(List.of("10:[]", "1:[10]", "2:[10]"), candidates);
  }

  @Test
  void rejectedCandidatesDoNotHideOlderCompatibleLocals() {
    List<String> candidates = new ArrayList<>();
    new VariableScopes(VariableScopesTest::original, (declaration, targets) -> {
      int source = VariableScopes.declarationVariable(declaration).getIndex();
      candidates.add(describe(declaration, targets));
      return source == 3 && targets.contains(new VarVersionPair(1, 0));
    }).process(block(1, 2, 3), Map.of());
    assertEquals(List.of("1:[]", "2:[1]", "3:[1, 2]"), candidates);
  }

  private static String describe(Exprent declaration, List<VarVersionPair> targets) {
    return VariableScopes.declarationVariable(declaration).getIndex() + ":" + targets.stream().map(pair -> pair.var).toList();
  }

  private static VarVersionPair original(int index) {
    return new VarVersionPair(1, index);
  }

  private static VarExprent variable(int index) {
    VarExprent variable = new VarExprent(index, VarType.VARTYPE_OBJECT, null);
    variable.setDefinition(true);
    return variable;
  }

  private static BasicBlockStatement block(int... locals) {
    BasicBlockStatement block = BasicBlockStatement.create();
    block.setExprents(new ArrayList<>());
    for (int local : locals) block.getExprents().add(variable(local));
    return block;
  }
}
