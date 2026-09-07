package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.*;

import static org.junit.jupiter.api.Assertions.*;

class VarTypeWorklistTest {
  @BeforeEach
  void setUp() { MinimalFernflowerEnvironment.setup(); }

  @AfterEach
  void tearDown() { DecompilerContext.setCurrentContext(null); }

  @Test
  void backwardDependenciesPropagateWithoutRescanningUnchangedAssignments() {
    var processor = new VarTypeProcessor(null, null);
    List<Exprent> roots = new ArrayList<>();
    List<CountingAssignment> assignments = new ArrayList<>();
    int count = 500;
    for (int i = count; i > 0; i--) {
      CountingAssignment assignment = new CountingAssignment(variable(processor, i), variable(processor, i - 1));
      assignments.add(assignment);
      roots.add(assignment);
    }
    roots.add(new AssignmentExprent(variable(processor, 0), new ConstExprent(7, false, null), null));
    processor.inferTypes(flatten(roots));
    for (int i = 0; i <= count; i++) assertEquals(VarType.VARTYPE_BYTECHAR, processor.getVarType(new VarVersionPair(i, 0)));
    assertTrue(assignments.stream().mapToInt(a -> a.evaluations).sum() < 5 * count);
  }

  @Test
  void worklistMatchesOrderedRestartSolverOnSharedVariablesConstantsAndUpperBounds() throws Exception {
    for (int seed = 0; seed < 24; seed++) {
      var reference = new VarTypeProcessor(null, null);
      var actual = new VarTypeProcessor(null, null);
      List<Exprent> expectedExpressions = scenario(reference, seed);
      List<Exprent> actualExpressions = scenario(actual, seed);
      solveByRestarting(reference, expectedExpressions);
      actual.inferTypes(actualExpressions);
      for (int index = 0; index < 12; index++) {
        var key = new VarVersionPair(index, 0);
        assertEquals(reference.getLowerBounds().get(key), actual.getLowerBounds().get(key), "lower: seed=" + seed + ", var=" + index);
        assertEquals(reference.getUpperBounds().get(key), actual.getUpperBounds().get(key), "upper: seed=" + seed + ", var=" + index);
      }
      List<VarType> expectedConstants = expectedExpressions.stream().filter(ConstExprent.class::isInstance)
        .map(e -> ((ConstExprent)e).getConstType()).toList();
      assertEquals(expectedConstants, actualExpressions.stream().filter(ConstExprent.class::isInstance)
        .map(e -> ((ConstExprent)e).getConstType()).toList(), "constants: seed=" + seed);
    }
  }

  @Test
  void sharedConstantsInvalidateParentsWithoutConflatingSlotVersions() {
    var processor = new VarTypeProcessor(null, null);
    ConstExprent shared = new ConstExprent(1, true, null);
    VarExprent first = variable(processor, 0);
    VarExprent reused = variable(processor, 0);
    reused.setVersion(1);
    List<Exprent> roots = List.of(
      new AssignmentExprent(first, shared, null),
      new AssignmentExprent(reused, new ConstExprent(VarType.VARTYPE_LONG, 4L, null), null),
      new ExitExprent(ExitExprent.Type.RETURN, shared, VarType.VARTYPE_INT, null, null));
    processor.inferTypes(flatten(roots));
    assertEquals(VarType.VARTYPE_BYTECHAR, processor.getVarType(new VarVersionPair(0, 0)));
    assertEquals(VarType.VARTYPE_LONG, processor.getVarType(new VarVersionPair(0, 1)));
    assertEquals(VarType.VARTYPE_BYTECHAR, shared.getConstType());
  }

  private static List<Exprent> scenario(VarTypeProcessor processor, int seed) {
    Random random = new Random(seed);
    List<Exprent> roots = new ArrayList<>();
    for (int i = 0; i < 24; i++) {
      VarExprent target = variable(processor, random.nextInt(12));
      Exprent value = variable(processor, random.nextInt(12));
      switch (i % 4) {
        case 0 -> value = new FunctionExprent(FunctionExprent.FunctionType.TERNARY,
          List.of(variable(processor, 11), value, new ConstExprent(random.nextInt(300), false, null)), null);
        case 1 -> roots.add(new ExitExprent(ExitExprent.Type.RETURN, variable(processor, random.nextInt(12)), VarType.VARTYPE_INT, null, null));
        case 2 -> value = new FunctionExprent(FunctionExprent.FunctionType.ADD,
          List.of(value, new ConstExprent(1, true, null)), null);
        case 3 -> value = new ConstExprent(random.nextBoolean() ? 1 : 100_000, true, null);
      }
      roots.add(new AssignmentExprent(target, value, null));
    }
    return flatten(roots);
  }

  private static List<Exprent> flatten(List<Exprent> roots) {
    List<Exprent> result = new ArrayList<>();
    for (Exprent root : roots) {
      result.addAll(root.getAllExprents(true));
      result.add(root);
    }
    return result;
  }

  private static VarExprent variable(VarTypeProcessor processor, int index) {
    return new VarExprent(index, VarType.VARTYPE_UNKNOWN, null) {
      @Override
      public VarType getExprType() {
        VarType inferred = processor.getVarType(getVarVersionPair());
        return inferred == null ? VarType.VARTYPE_UNKNOWN : inferred;
      }
    };
  }

  // The reference retains the former full-prefix restart schedule, using the same
  // bound-transfer rules so this comparison isolates scheduling and invalidation.
  @SuppressWarnings({"unchecked", "rawtypes"})
  private static void solveByRestarting(VarTypeProcessor processor, List<Exprent> expressions) throws Exception {
    Class<? extends Enum> bound = (Class<? extends Enum>)Class.forName(VarTypeProcessor.class.getName() + "$Bound");
    Object upper = Enum.valueOf(bound, "UPPER"), lower = Enum.valueOf(bound, "LOWER");
    Method transfer = VarTypeProcessor.class.getDeclaredMethod("changeExprentType", Exprent.class, VarType.class, bound);
    transfer.setAccessible(true);
    Field worklist = VarTypeProcessor.class.getDeclaredField("worklist");
    worklist.setAccessible(true);
    worklist.set(processor, new TypeInferenceWorklist(expressions));
    for (int pass = 0; pass < 10_000; pass++) {
      boolean stable = true;
      for (Exprent expression : expressions) {
        if (expression instanceof ConstExprent constant && constant.getConstType().typeFamily.intOrBool()) {
          processor.getLowerBounds().putIfAbsent(new VarVersionPair(constant.id, -1), constant.getConstType());
        }
        CheckTypesResult constraints = expression.checkExprTypeBounds();
        if (constraints == null) continue;
        for (var entry : constraints.getUpperBounds()) transfer.invoke(processor, entry.exprent, entry.type, upper);
        for (var entry : constraints.getLowerBounds()) stable &= (Boolean)transfer.invoke(processor, entry.exprent, entry.type, lower);
        if (!stable) break;
      }
      if (stable) return;
    }
    fail("Reference inference did not converge");
  }

  private static final class CountingAssignment extends AssignmentExprent {
    int evaluations;
    CountingAssignment(Exprent left, Exprent right) { super(left, right, null); }
    @Override
    public CheckTypesResult checkExprTypeBounds() {
      evaluations++;
      return super.checkExprTypeBounds();
    }
  }
}
