package org.jetbrains.java.decompiler.modules.code;

import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.code.cfg.ExceptionRangeCFG;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.GenericDominatorEngine;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.IGraph;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.IGraphNode;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class ExceptionDeobfuscatorTest {
  @Test
  public void nestedSparseEntryIsSplitBeforeItsDominator() {
    BasicBlock root = block(0);
    BasicBlock outerEntry = block(1);
    BasicBlock unprotectedConnector = block(2);
    BasicBlock nestedEntry = block(3);
    BasicBlock nestedTail = block(4);

    connect(root, outerEntry);
    connect(outerEntry, unprotectedConnector);
    connect(unprotectedConnector, nestedEntry);
    connect(nestedEntry, nestedTail);

    ExceptionRangeCFG range = range(outerEntry, nestedEntry, nestedTail);
    GenericDominatorEngine dominators = dominators(root, outerEntry, unprotectedConnector, nestedEntry, nestedTail);

    assertEquals(
      List.of(3, 4),
      blockIds(ExceptionDeobfuscator.selectSubrangeToSplit(range, List.of(outerEntry, nestedEntry), dominators))
    );
    assertEquals(
      List.of(3, 4),
      blockIds(ExceptionDeobfuscator.selectSubrangeToSplit(range, List.of(nestedEntry, outerEntry), dominators))
    );
  }

  @Test
  public void nestedEntryChainIsPartitionedInsideOut() {
    BasicBlock root = block(0);
    BasicBlock outerEntry = block(1);
    BasicBlock firstConnector = block(2);
    BasicBlock middleEntry = block(3);
    BasicBlock secondConnector = block(4);
    BasicBlock innerEntry = block(5);
    BasicBlock innerTail = block(6);

    connect(root, outerEntry);
    connect(outerEntry, firstConnector);
    connect(firstConnector, middleEntry);
    connect(middleEntry, secondConnector);
    connect(secondConnector, innerEntry);
    connect(innerEntry, innerTail);

    ExceptionRangeCFG range = range(outerEntry, middleEntry, innerEntry, innerTail);
    GenericDominatorEngine dominators = dominators(
      root,
      outerEntry,
      firstConnector,
      middleEntry,
      secondConnector,
      innerEntry,
      innerTail
    );

    List<BasicBlock> firstSplit = ExceptionDeobfuscator.selectSubrangeToSplit(
      range,
      List.of(middleEntry, outerEntry, innerEntry),
      dominators
    );
    assertEquals(List.of(5, 6), blockIds(firstSplit));
    range.getProtectedRange().removeAll(firstSplit);

    assertEquals(
      List.of(3),
      blockIds(ExceptionDeobfuscator.selectSubrangeToSplit(range, List.of(outerEntry, middleEntry), dominators))
    );
  }

  @Test
  public void incomparableEntriesUseStableTieBreaker() {
    BasicBlock root = block(0);
    BasicBlock leftEntry = block(1);
    BasicBlock leftTail = block(2);
    BasicBlock rightEntry = block(3);
    BasicBlock rightTail = block(4);

    connect(root, leftEntry);
    connect(root, rightEntry);
    connect(leftEntry, leftTail);
    connect(rightEntry, rightTail);

    ExceptionRangeCFG range = range(leftEntry, leftTail, rightEntry, rightTail);
    GenericDominatorEngine dominators = dominators(root, rightEntry, rightTail, leftEntry, leftTail);

    assertEquals(
      List.of(3, 4),
      blockIds(ExceptionDeobfuscator.selectSubrangeToSplit(range, List.of(leftEntry, rightEntry), dominators))
    );
    assertEquals(
      List.of(3, 4),
      blockIds(ExceptionDeobfuscator.selectSubrangeToSplit(range, List.of(rightEntry, leftEntry), dominators))
    );
  }

  private static BasicBlock block(int id) {
    return new BasicBlock(id);
  }

  private static void connect(BasicBlock source, BasicBlock destination) {
    source.addSuccessor(destination);
  }

  private static ExceptionRangeCFG range(BasicBlock... blocks) {
    return new ExceptionRangeCFG(new ArrayList<>(List.of(blocks)), new BasicBlock(100), (List<String>)null);
  }

  private static GenericDominatorEngine dominators(BasicBlock root, BasicBlock... reversePostOrder) {
    List<BasicBlock> ordered = new ArrayList<>();
    ordered.add(root);
    ordered.addAll(List.of(reversePostOrder));

    GenericDominatorEngine engine = new GenericDominatorEngine(new IGraph() {
      @Override
      public List<? extends IGraphNode> getReversePostOrderList() {
        return ordered;
      }

      @Override
      public Set<? extends IGraphNode> getRoots() {
        return Set.of(root);
      }
    });
    engine.initialize();
    return engine;
  }

  private static List<Integer> blockIds(Collection<BasicBlock> blocks) {
    return blocks.stream().map(BasicBlock::getId).toList();
  }
}
