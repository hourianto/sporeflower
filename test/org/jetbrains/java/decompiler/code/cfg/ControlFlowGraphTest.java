package org.jetbrains.java.decompiler.code.cfg;

import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.ExceptionHandler;
import org.jetbrains.java.decompiler.code.ExceptionTable;
import org.jetbrains.java.decompiler.code.FullInstructionSequence;
import org.jetbrains.java.decompiler.code.Instruction;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotSame;
import static org.junit.jupiter.api.Assertions.assertSame;

public class ControlFlowGraphTest {
  private static final BytecodeVersion VERSION = new BytecodeVersion(BytecodeVersion.MAJOR_8, 0);

  @Test
  public void copyKeepsIndependentTopologyAndRangeMetadata() {
    FullInstructionSequence sequence = new FullInstructionSequence(
      List.of(
        instruction(CodeConstants.opc_nop, CodeConstants.GROUP_GENERAL, 0),
        instruction(CodeConstants.opc_return, CodeConstants.GROUP_RETURN, 1),
        instruction(CodeConstants.opc_athrow, CodeConstants.GROUP_RETURN, 2)
      ),
      Map.of(0, 0, 1, 1, 2, 2),
      new ExceptionTable(List.of(new ExceptionHandler(0, 1, 2, null)))
    );

    ControlFlowGraph graph = new ControlFlowGraph(sequence);
    ExceptionRangeCFG range = graph.getExceptions().get(0);
    range.setHandlerCloneGroupId(17);
    graph.addComment("original");

    ControlFlowGraph copy = graph.copy();
    ExceptionRangeCFG rangeCopy = copy.getExceptions().get(0);

    assertSame(sequence, copy.getSequence());
    assertEquals(graph.getBlocks().getLstKeys(), copy.getBlocks().getLstKeys());
    assertNotSame(graph.getFirst(), copy.getFirst());
    assertNotSame(range.getHandler(), rangeCopy.getHandler());
    assertNotSame(range.getProtectedRange().get(0), rangeCopy.getProtectedRange().get(0));
    assertEquals(17, rangeCopy.getHandlerCloneGroupId());
    assertEquals(graph.commentLines, copy.commentLines);
    assertNotSame(graph.commentLines, copy.commentLines);

    rangeCopy.getProtectedRange().clear();
    copy.getFirst().getSeq().clear();
    assertEquals(1, range.getProtectedRange().size());
    assertEquals(1, graph.getFirst().getSeq().length());
  }

  private static Instruction instruction(int opcode, int group, int offset) {
    return Instruction.create(opcode, false, group, VERSION, null, offset, 1);
  }
}
