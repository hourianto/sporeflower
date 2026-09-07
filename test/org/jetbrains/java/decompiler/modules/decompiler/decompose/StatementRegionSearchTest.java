package org.jetbrains.java.decompiler.modules.decompiler.decompose;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class StatementRegionSearchTest {
  private StatementRegionSearch search;
  private int nextId;

  @BeforeEach
  void setUp() {
    MinimalFernflowerEnvironment.setup();
    search = new StatementRegionSearch();
  }

  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  void preservesBreadthFirstOrderAndStopsAtTheBoundary() {
    Statement head = node(), left = node(), right = node(), join = node(), exit = node();
    connect(head, left); connect(head, right); connect(head, left);
    connect(left, join); connect(right, join); connect(join, exit);
    assertEquals(List.of(head, left, right, join), search.findRegion(head, exit, 5));
    assertNull(search.findRegion(head, head, 5)); // the whole parent is not a subregion
    assertEquals(List.of(head, left, right), search.findRegion(head, join, 5));
  }

  @Test
  void rejectsSideEntriesButAllowsEntriesAtTheHead() {
    Statement outside = node(), head = node(), body = node(), exit = node();
    connect(outside, head); connect(outside, body); connect(head, body); connect(body, exit);
    assertNull(search.findRegion(head, exit, 4));
    assertEquals(List.of(outside, head, body), search.findRegion(outside, exit, 4));
  }

  @Test
  void aSingletonRegionRequiresASelfLoop() {
    Statement loop = node(), exit = node();
    connect(loop, loop); connect(loop, exit);
    assertEquals(List.of(loop), search.findRegion(loop, exit, 2));
    assertNull(search.findRegion(exit, exit, 2));
  }

  @Test
  void externalHandlerMustCoverTheEntireRegion() {
    Statement head = node(), body = node(), exit = node(), handler = node();
    connect(head, body); connect(body, exit);
    protect(head, handler); protect(body, handler);
    // An equal range stays outside under the historical strict-subset rule.
    assertEquals(List.of(head, body), search.findRegion(head, exit, 4));
  }

  @Test
  void partiallyOverlappingExceptionRangesCannotBeSplit() {
    Statement head = node(), body = node(), outside = node(), exit = node(), handler = node();
    connect(head, body); connect(body, exit);
    protect(head, handler); protect(outside, handler);
    assertNull(search.findRegion(head, exit, 5));
  }

  @Test
  void reconsiderEarlierHandlersAfterTheirDependenciesEnterTheRegion() {
    Statement head = node(), body = node(), tail = node(), exit = node(), first = node(), second = node();
    connect(head, body); connect(body, tail); connect(tail, exit);
    protect(head, first); protect(second, first); protect(head, second);
    assertEquals(List.of(head, body, tail, second, first), search.findRegion(head, exit, 6));
  }

  @Test
  void boundaryHandlerIsCheckedWithoutBeingEnteredOrRetriedForever() {
    Statement head = node(), body = node(), exit = node();
    connect(head, body); connect(body, exit); protect(head, exit);
    assertNull(search.findRegion(head, exit, 3));
  }

  @Test
  void duplicateExceptionEdgesDoNotCountAsCoverageOfAnotherNode() {
    Statement head = node(), body = node(), exit = node(), handler = node();
    connect(head, body); connect(body, exit);
    protect(head, handler); protect(head, handler);
    assertNull(search.findRegion(head, exit, 4));
  }

  @Test
  void monitorEntryAndItsRegularSuccessorStayTogether() {
    Statement head = node(), body = node(), exit = node();
    Statement monitor = new Statement(Statement.StatementType.BASIC_BLOCK) {
      @Override
      public boolean isMonitorEnter() {
        return true;
      }
    };
    connect(head, monitor); connect(monitor, body); connect(body, exit);
    assertEquals(List.of(head, monitor, body), search.findRegion(head, exit, 4));
    assertNull(search.findRegion(head, body, 4));
  }

  private Statement node() {
    return new BasicBlockStatement(new BasicBlock(++nextId));
  }

  private static void connect(Statement from, Statement to) {
    from.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, from, to));
  }

  private static void protect(Statement from, Statement handler) {
    from.addSuccessor(new StatEdge(from, handler, List.of("java/lang/Exception")));
  }
}
