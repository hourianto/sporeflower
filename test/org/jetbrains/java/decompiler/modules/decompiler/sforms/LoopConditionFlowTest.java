package org.jetbrains.java.decompiler.modules.decompiler.sforms;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionNode;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.EnumSource;

import java.util.HashSet;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class LoopConditionFlowTest {
  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @ParameterizedTest
  @EnumSource(value = DoStatement.Type.class, names = {"WHILE", "DO_WHILE", "FOR"})
  void shortCircuitOutcomesReachTheCorrectLoopEdges(DoStatement.Type type) {
    for (boolean and : new boolean[]{false, true}) {
      for (SFormsConstructor ssa : List.of(new SSAConstructorSparseEx(), new SSAUConstructorSparseEx())) {
        MinimalFernflowerEnvironment.setup();
        BasicBlockStatement entry = BasicBlockStatement.create(), body = BasicBlockStatement.create(), exit = BasicBlockStatement.create();
        VarExprent initial = variable(), written = variable(), bodyRead = variable(), exitRead = variable();
        entry.getExprents().add(new AssignmentExprent(initial, new ConstExprent(0, true, null), null));
        body.getExprents().add(bodyRead);
        exit.getExprents().add(exitRead);
        StatEdge backedge = new StatEdge(StatEdge.TYPE_REGULAR, body, body);
        body.addSuccessor(backedge);
        DoStatement loop = (DoStatement)DoStatement.isHead(body);
        assertNotNull(loop);
        body.removeSuccessor(backedge);
        body.addSuccessor(new StatEdge(StatEdge.TYPE_CONTINUE, body, loop, loop));
        loop.setLooptype(type);
        if (type == DoStatement.Type.FOR) loop.setIncExprent(new ConstExprent(1, true, null));
        loop.setConditionExprent(new FunctionExprent(and ? FunctionExprent.FunctionType.BOOLEAN_AND : FunctionExprent.FunctionType.BOOLEAN_OR,
          List.of(new ConstExprent(1, true, null), new AssignmentExprent(written, new ConstExprent(1, true, null), null)), null));
        entry.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, entry, loop));
        loop.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, loop, exit));
        RootStatement root = new RootStatement(new SequenceStatement(List.of(entry, loop, exit)), new DummyExitStatement(), null);
        root.setAllParent();
        DirectGraph graph = FlattenStatementsHelper.build(root);
        ssa.factory = new FastSparseSetFactory<>(List.of());
        Set<String> updated = new HashSet<>();
        int iteration = 0;
        do {
          assertTrue(++iteration < 10, "Loop flow must converge");
          ssa.ssaStatements(graph, updated, false, null, iteration);
        } while (!updated.isEmpty());

        // && can skip the write on exit; || can skip it when entering the body.
        // A do/while additionally enters the body before testing its condition.
        assertEquals(and, reaches(ssa, exitRead, initial), type + " exit, and=" + and);
        assertTrue(reaches(ssa, exitRead, written));
        assertEquals(!and || type == DoStatement.Type.DO_WHILE, reaches(ssa, bodyRead, initial), type + " body, and=" + and);
        assertTrue(reaches(ssa, bodyRead, written));
      }
    }
  }

  private static boolean reaches(SFormsConstructor ssa, VarExprent read, VarExprent definition) {
    if (ssa instanceof SSAUConstructorSparseEx ssau) {
      return reaches(ssau.getNode(read), ssau.getNode(definition), new HashSet<>());
    }
    return reaches((SSAConstructorSparseEx)ssa, read.getVarVersionPair(), definition.getVersion(), new HashSet<>());
  }

  private static boolean reaches(SSAConstructorSparseEx ssa, VarVersionPair read, int definition, Set<Integer> visited) {
    if (read.version == definition) return true;
    if (!visited.add(read.version)) return false;
    var inputs = ssa.getPhi().get(read);
    if (inputs != null) {
      for (int version : inputs) {
        if (reaches(ssa, new VarVersionPair(read.var, version), definition, visited)) return true;
      }
    }
    return false;
  }

  private static boolean reaches(VarVersionNode node, VarVersionNode definition, Set<VarVersionNode> visited) {
    return node == definition || visited.add(node) && node.predecessors.stream().anyMatch(pred -> reaches(pred, definition, visited));
  }

  private static VarExprent variable() {
    return new VarExprent(0, VarType.VARTYPE_BOOLEAN, null);
  }
}
