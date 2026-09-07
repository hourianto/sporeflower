package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.util.ArrayDeque;
import java.util.HashSet;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class EmptyIfBranchTest {
  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void sharedTerminalReturnKeepsItsIncomingBreaks(boolean existingContinuation) {
    MinimalFernflowerEnvironment.setup();
    BasicBlockStatement terminal = BasicBlockStatement.create();
    terminal.getExprents().add(new ExitExprent(ExitExprent.Type.RETURN, null, VarType.VARTYPE_VOID, null, null));
    BasicBlockStatement first = BasicBlockStatement.create(), second = BasicBlockStatement.create();
    first.getExprents().add(new ConstExprent(1, false, null));
    second.getExprents().add(new ConstExprent(2, false, null));
    IfStatement nested = conditional(first, second);
    IfStatement outer = conditional(terminal, nested);
    RootStatement root = new RootStatement(outer, new DummyExitStatement(), null);
    root.setAllParent();
    StatEdge explicit = new StatEdge(StatEdge.TYPE_BREAK, first, terminal, nested);
    first.addSuccessor(explicit);
    StatEdge implicit = new StatEdge(StatEdge.TYPE_BREAK, second, terminal, nested);
    implicit.explicit = false;
    second.addSuccessor(implicit);
    StatEdge exit = new StatEdge(StatEdge.TYPE_BREAK, terminal, root.getDummyExit(), outer);
    exit.explicit = false;
    terminal.addSuccessor(exit);
    if (existingContinuation) {
      StatEdge successor = new StatEdge(StatEdge.TYPE_BREAK, outer, root.getDummyExit(), root);
      successor.explicit = false;
      outer.addSuccessor(successor);
    }
    assertGraphLinks(root);
    String jump = ExprProcessor.jmpWrapper(first, 0, false).toString();
    assertTrue(jump.contains("break label" + nested.id + ";"), jump);
    assertTrue(ExitHelper.removeRedundantReturns(root));
    assertTrue(SecondaryFunctionsHelper.identifySecondaryFunctions(root, null));

    for (StatEdge edge : new StatEdge[]{explicit, implicit}) {
      assertSame(terminal, edge.getDestination());
      assertEquals(StatEdge.TYPE_BREAK, edge.getType());
      assertSame(nested, edge.closure);
      assertTrue(nested.getLabelEdges().contains(edge));
    }
    assertTrue(explicit.explicit);
    assertTrue(explicit.labeled);
    assertFalse(implicit.explicit);
    assertEquals(jump, ExprProcessor.jmpWrapper(first, 0, false).toString());
    assertEquals(3, terminal.getAllPredecessorEdges().size());
    assertEquals(1, terminal.getAllSuccessorEdges().size());
    assertInstanceOf(SequenceStatement.class, root.getFirst());
    assertSame(root.getFirst(), terminal.getParent());
    assertSame(terminal, outer.getFirstSuccessor().getDestination());
    assertSame(root.getFirst(), exit.closure);
    assertSame(nested, outer.getIfstat());
    assertEquals(IfStatement.IFTYPE_IF, outer.iftype);
    assertGraphLinks(root);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void unsharedBranchIsFullyDetached(boolean existingContinuation) {
    MinimalFernflowerEnvironment.setup();
    BasicBlockStatement empty = BasicBlockStatement.create(), body = BasicBlockStatement.create();
    body.getExprents().add(new ConstExprent(1, false, null));
    IfStatement conditional = conditional(empty, body);
    RootStatement root = new RootStatement(conditional, new DummyExitStatement(), null);
    root.setAllParent();
    for (Statement source : new Statement[]{empty, body}) {
      StatEdge exit = new StatEdge(StatEdge.TYPE_BREAK, source, root.getDummyExit(), conditional);
      exit.explicit = false;
      source.addSuccessor(exit);
    }
    if (existingContinuation) {
      StatEdge exit = new StatEdge(StatEdge.TYPE_BREAK, conditional, root.getDummyExit(), root);
      exit.explicit = false;
      conditional.addSuccessor(exit);
    }

    assertTrue(SecondaryFunctionsHelper.identifySecondaryFunctions(root, null));
    assertTrue(empty.getAllPredecessorEdges().isEmpty());
    assertTrue(empty.getAllSuccessorEdges().isEmpty());
    assertNull(empty.getParent());
    assertSame(conditional, root.getFirst());
    assertGraphLinks(root);
  }

  @Test
  void sharedLoopTailRetainsItsContinueAndBreakScopes() {
    MinimalFernflowerEnvironment.setup();
    BasicBlockStatement empty = BasicBlockStatement.create(), body = BasicBlockStatement.create();
    body.getExprents().add(new ConstExprent(1, false, null));
    IfStatement conditional = conditional(empty, body);
    StatEdge backedge = new StatEdge(StatEdge.TYPE_REGULAR, conditional, conditional);
    conditional.addSuccessor(backedge);
    DoStatement loop = (DoStatement)DoStatement.isHead(conditional);
    assertNotNull(loop);
    loop.setAllParent();
    backedge.remove();
    RootStatement root = new RootStatement(loop, new DummyExitStatement(), null);
    root.setAllParent();
    StatEdge exit = new StatEdge(StatEdge.TYPE_CONTINUE, empty, loop, loop);
    exit.explicit = false;
    empty.addSuccessor(exit);
    StatEdge jump = new StatEdge(StatEdge.TYPE_BREAK, body, empty, conditional);
    body.addSuccessor(jump);

    assertGraphLinks(root);
    String before = ExprProcessor.jmpWrapper(body, 0, false).toString();
    assertTrue(SecondaryFunctionsHelper.identifySecondaryFunctions(root, null));
    assertSame(conditional, jump.closure);
    assertSame(empty, jump.getDestination());
    assertSame(loop, exit.closure);
    assertSame(loop, exit.getDestination());
    assertEquals(StatEdge.TYPE_CONTINUE, exit.getType());
    assertEquals(before, ExprProcessor.jmpWrapper(body, 0, false).toString());
    assertGraphLinks(root);
  }

  private static IfStatement conditional(Statement whenTrue, Statement whenFalse) {
    IfStatement statement = IfStatement.createSourceOnly(new ConstExprent(1, true, null), whenTrue);
    statement.iftype = IfStatement.IFTYPE_IFELSE;
    statement.setElsestat(whenFalse);
    whenFalse.setParent(statement);
    statement.getStats().addWithKey(whenFalse, whenFalse.id);
    StatEdge edge = new StatEdge(StatEdge.TYPE_REGULAR, statement.getFirst(), whenFalse);
    statement.getFirst().addSuccessor(edge);
    statement.setElseEdge(edge);
    return statement;
  }

  private static void assertGraphLinks(RootStatement root) {
    Set<Statement> statements = new HashSet<>();
    ArrayDeque<Statement> pending = new ArrayDeque<>();
    pending.add(root);
    statements.add(root.getDummyExit());
    while (!pending.isEmpty()) {
      Statement statement = pending.removeFirst();
      assertTrue(statements.add(statement));
      for (Statement child : statement.getStats()) assertSame(statement, child.getParent());
      pending.addAll(statement.getStats());
    }
    for (Statement statement : statements) {
      for (StatEdge edge : statement.getAllSuccessorEdges()) {
        assertSame(statement, edge.getSource());
        assertTrue(statements.contains(edge.getDestination()), edge.toString());
        assertTrue(edge.getDestination().getAllPredecessorEdges().contains(edge));
        if (edge.closure != null) {
          assertTrue(statements.contains(edge.closure));
          assertTrue(edge.closure.getLabelEdges().contains(edge));
        }
      }
      for (StatEdge edge : statement.getAllPredecessorEdges()) {
        assertSame(statement, edge.getDestination());
        assertTrue(statements.contains(edge.getSource()), edge.toString());
        assertTrue(edge.getSource().getAllSuccessorEdges().contains(edge));
      }
      for (StatEdge edge : statement.getLabelEdges()) {
        assertSame(statement, edge.closure);
        assertTrue(statements.contains(edge.getSource()), edge.toString());
      }
    }
    assertDoesNotThrow(() -> FlattenStatementsHelper.build(root));
  }
}
