package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.MinimalFernflowerEnvironment;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNodeType;
import org.jetbrains.java.decompiler.modules.decompiler.sforms.SSAUConstructorSparseEx;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.DoStatement;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import static org.junit.jupiter.api.Assertions.*;

class StackVarsProcessorTest {
  @BeforeEach
  void setUp() {
    MinimalFernflowerEnvironment.setup();
  }

  @AfterEach
  void tearDown() {
    DecompilerContext.setCurrentContext(null);
  }

  @ParameterizedTest
  @ValueSource(booleans = {false, true})
  void emptyForInitializerRestartsOnlyWhenTheLoopChanges(boolean hasIncrement) {
    var body = new BasicBlockStatement(new BasicBlock(1));
    body.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, body, body));
    var loop = (DoStatement)DoStatement.isHead(body);
    assertNotNull(loop);
    loop.setLooptype(DoStatement.Type.FOR);
    if (hasIncrement) loop.setIncExprent(new ConstExprent(1, false, null));

    var graph = new DirectGraph();
    var ssa = new SSAUConstructorSparseEx() {
      @Override
      public DirectGraph getDirectGraph() {
        return graph;
      }
    };
    var options = new StackVarsProcessor.StackSimplifyOptions();
    // An absent for initializer gets a fresh, empty synthetic list on each graph build.
    // Filling that list must not itself report progress and keep simplification running.
    for (int iteration = 0; iteration < 2; iteration++) {
      graph.first = DirectNode.forStat(DirectNodeType.INIT, loop, null);
      assertEquals(!hasIncrement && iteration == 0, StackVarsProcessor.iterateStatements(ssa, options));
      assertEquals(hasIncrement ? DoStatement.Type.FOR : DoStatement.Type.WHILE, loop.getLooptype());
    }
  }
}
