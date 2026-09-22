package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class ParameterDependenciesTest {
  @BeforeEach
  void setUp() { MinimalFernflowerEnvironment.setup(); }

  @AfterEach
  void tearDown() { DecompilerContext.setCurrentContext(null); }

  @Test
  void anAccumulatorDoesNotConnectAnIndependentCounterBackToTheParameter() {
    ParameterDependencies dependencies = dependencies();
    dependencies.collect(assign(1, var(0)));
    dependencies.collect(assign(2, constant()));
    dependencies.collect(assign(1, add(var(1), var(2))));
    dependencies.collect(assign(2, add(var(2), constant())));
    dependencies.finish();
    assertTrue(dependencies.supports(pair(1), pair(0)));
    assertFalse(dependencies.supports(pair(2), pair(0)));
  }

  @Test
  void optionalMergesRequireSupportFromEveryLifetime() {
    ParameterDependencies dependencies = dependencies();
    dependencies.collect(assign(1, var(0)));
    dependencies.collect(assign(2, constant()));
    dependencies.collect(assign(3, var(1)));
    dependencies.finish();
    assertTrue(dependencies.supports(pair(3), pair(0)));
    dependencies.merge(pair(2), pair(1));
    assertFalse(dependencies.supports(pair(1), pair(0)));
    dependencies.merge(pair(1), pair(3));
    assertFalse(dependencies.supports(pair(3), pair(0)));
  }

  @Test
  void cyclesNeedAnInputAndAnAssignmentTargetIsNotARead() {
    ParameterDependencies dependencies = dependencies();
    dependencies.collect(assign(1, var(2)));
    dependencies.collect(assign(2, var(1)));
    dependencies.collect(assign(3, assign(0, constant())));
    dependencies.collect(assign(4, var(5)));
    dependencies.collect(assign(5, add(var(4), var(0))));
    dependencies.finish();
    assertFalse(dependencies.supports(pair(1), pair(0)));
    assertFalse(dependencies.supports(pair(3), pair(0)));
    assertTrue(dependencies.supports(pair(4), pair(0)));
  }

  private static ParameterDependencies dependencies() {
    return new ParameterDependencies(List.of(pair(0)), parameter -> true, index -> new VarVersionPair(0, index + 1));
  }

  private static VarVersionPair pair(int index) { return new VarVersionPair(index, 0); }
  private static VarExprent var(int index) { return new VarExprent(index, VarType.VARTYPE_INT, null); }
  private static ConstExprent constant() { return new ConstExprent(VarType.VARTYPE_INT, 0, null); }
  private static AssignmentExprent assign(int index, Exprent right) { return new AssignmentExprent(var(index), right, null); }
  private static FunctionExprent add(Exprent left, Exprent right) {
    return new FunctionExprent(FunctionExprent.FunctionType.ADD, List.of(left, right), null);
  }
}
