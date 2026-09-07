package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.StrongConnectivityHelper;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.GeneralStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

class StatementTraversalTest {
  @BeforeEach
  void setUp() {
    MinimalFernflowerEnvironment.setup();
  }

  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @Test
  void preservesEdgeOrderAndOrderingBetweenExitComponents() {
    Statement a = node(1), b = node(2), c = node(3), d = node(4), e = node(5), f = node(6);
    var graph = new GeneralStatement(a, List.of(a, b, c, d, e, f), null);
    connect(a, b, StatEdge.TYPE_REGULAR);
    connect(a, c, StatEdge.TYPE_EXCEPTION);
    connect(a, d, StatEdge.TYPE_REGULAR);
    connect(b, d, StatEdge.TYPE_EXCEPTION);
    connect(c, d, StatEdge.TYPE_REGULAR);
    connect(d, b, StatEdge.TYPE_CONTINUE); // excluded from both traversals
    connect(e, f, StatEdge.TYPE_REGULAR);
    assertEquals(List.of(a, c, b, d), graph.getReversePostOrderList());
    assertEquals(List.of(f, e, d, b, c, a), graph.getPostReversePostOrderList(List.of(d, f)));
    assertFalse(StrongConnectivityHelper.isExitComponent(List.of(a, b, c)));
    assertTrue(StrongConnectivityHelper.isExitComponent(List.of(d)));
    assertTrue(StrongConnectivityHelper.isExitComponent(List.of(e, f)));
  }

  @Test
  void deepGraphsUseAnExplicitStackInBothDirections() {
    List<Statement> nodes = new ArrayList<>();
    for (int i = 0; i < 12_000; i++) nodes.add(node(i + 1));
    for (int i = 1; i < nodes.size(); i++) connect(nodes.get(i - 1), nodes.get(i), StatEdge.TYPE_REGULAR);
    var graph = new GeneralStatement(nodes.get(0), nodes, null);
    assertEquals(nodes, graph.getReversePostOrderList());
    List<Statement> reversed = new ArrayList<>(nodes);
    Collections.reverse(reversed);
    assertEquals(reversed, graph.getPostReversePostOrderList(List.of(nodes.get(nodes.size() - 1))));
  }

  private static Statement node(int id) {
    return new BasicBlockStatement(new BasicBlock(id));
  }

  private static void connect(Statement from, Statement to, int type) {
    from.addSuccessor(type == StatEdge.TYPE_EXCEPTION
      ? new StatEdge(from, to, List.of("java/lang/Exception")) : new StatEdge(type, from, to));
  }
}
