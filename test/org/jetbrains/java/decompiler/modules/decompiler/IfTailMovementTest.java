package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;
import org.junit.jupiter.params.provider.ValueSource;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class IfTailMovementTest {
  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @ParameterizedTest
  @CsvSource({"false,false,false", "false,true,false", "true,false,false", "true,true,false",
              "false,false,true", "false,true,true", "true,false,true", "true,true,true"})
  void catchContinuationCannotBecomeAnElseBranch(boolean secondBranch, boolean implicit, boolean wrapped) {
    Fixture fixture = fixture(wrapped);
    StatEdge continuation = continueTo(secondBranch ? fixture.second : fixture.first, fixture.tail);
    continuation.explicit = !implicit;

    ValidationHelper.validateStatement(fixture.root);
    assertFalse(IfHelper.mergeIfs(fixture.outer, new HashSet<>()));
    assertSame(fixture.sequence, fixture.tail.getParent());
    assertSame(fixture.tail, fixture.outer.getFirstSuccessor().getDestination());
    assertSame(fixture.tail, continuation.getDestination());
    assertEquals(!implicit, continuation.explicit);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void independentReturningBranchesCanStillBeDenested(boolean wrapped) {
    Fixture fixture = fixture(wrapped);
    ValidationHelper.validateStatement(fixture.root);
    assertTrue(IfHelper.mergeIfs(fixture.outer, new HashSet<>()));
    assertSame(fixture.tail, fixture.outer.getIfstat());
    assertSame(fixture.outer, fixture.tail.getParent());
  }

  private static Fixture fixture(boolean wrapped) {
    MinimalFernflowerEnvironment.setup();
    DummyExitStatement exit = new DummyExitStatement();
    BasicBlockStatement tail = returning(3);
    CatchStatement first = catchingReturn(1, exit), second = catchingReturn(2, exit);
    IfStatement nested = IfStatement.createSourceOnly(new ConstExprent(1, true, null), first);
    nested.iftype = IfStatement.IFTYPE_IFELSE;
    nested.setElsestat(second);
    nested.getStats().addWithKey(second, second.id);
    StatEdge elseEdge = new StatEdge(StatEdge.TYPE_REGULAR, nested.getFirst(), second);
    nested.getFirst().addSuccessor(elseEdge);
    nested.setElseEdge(elseEdge);
    IfStatement outer = IfStatement.createSourceOnly(new ConstExprent(1, true, null), wrapped ? new SequenceStatement(nested) : nested);
    SequenceStatement sequence = new SequenceStatement(List.of(outer, tail));
    RootStatement root = new RootStatement(sequence, exit, null);
    linkParents(root);
    outer.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, outer, tail));
    tail.addSuccessor(new StatEdge(StatEdge.TYPE_BREAK, tail, exit, sequence));
    return new Fixture(root, sequence, outer, first, second, tail);
  }

  private static CatchStatement catchingReturn(int value, Statement exit) {
    BasicBlockStatement body = returning(value), handler = returning(4);
    StatEdge exception = new StatEdge(body, handler, List.of("java/lang/Exception"));
    body.addSuccessor(exception);
    CatchStatement statement = assertInstanceOf(CatchStatement.class, CatchStatement.isHead(body));
    // Exception edges have already been represented by the catch statement at
    // the simplification stage; handler exits remain ordinary control flow.
    exception.remove();
    body.addSuccessor(new StatEdge(StatEdge.TYPE_BREAK, body, exit, statement));
    handler.addSuccessor(new StatEdge(StatEdge.TYPE_BREAK, handler, exit, statement));
    return statement;
  }

  private static StatEdge continueTo(CatchStatement statement, Statement target) {
    Statement handler = statement.getStats().get(1);
    handler.getExprents().clear();
    for (StatEdge edge : new ArrayList<>(handler.getAllSuccessorEdges())) {
      edge.remove();
    }
    StatEdge continuation = new StatEdge(StatEdge.TYPE_BREAK, handler, target, statement);
    handler.addSuccessor(continuation);
    return continuation;
  }

  private static void linkParents(Statement statement) {
    statement.setAllParent();
    for (Statement child : statement.getStats()) {
      linkParents(child);
    }
  }

  private static BasicBlockStatement returning(int value) {
    BasicBlockStatement block = BasicBlockStatement.create();
    block.getExprents().add(new ExitExprent(ExitExprent.Type.RETURN, new ConstExprent(value, false, null), VarType.VARTYPE_INT, null, null));
    return block;
  }

  private record Fixture(RootStatement root, SequenceStatement sequence, IfStatement outer,
                         CatchStatement first, CatchStatement second, Statement tail) { }
}
