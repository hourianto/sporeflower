package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.Instruction;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertSame;

public class FinallyProcessorDeterminismTest {
  private static final BytecodeVersion JAVA_1 = new BytecodeVersion(BytecodeVersion.MAJOR_1_0_2, 3);

  @Test
  public void testNearestTrueExitWinsOverLaterSharedTailExit() {
    BasicBlock earlyReturn = block(19, 47);
    BasicBlock tailReturn = block(17, 69);
    BasicBlock[] earlyCandidate = new BasicBlock[]{new BasicBlock(18), earlyReturn, earlyReturn};
    BasicBlock[] tailCandidate = new BasicBlock[]{new BasicBlock(12), tailReturn, tailReturn};

    List<BasicBlock[]> candidates = new ArrayList<>(List.of(tailCandidate, earlyCandidate));
    candidates.sort(FinallyProcessor::compareNextCandidates);

    assertSame(earlyCandidate, candidates.get(0));
  }

  @Test
  public void testTrueExitWinsOverEarlierSideExit() {
    BasicBlock sideExit = block(7, 12);
    BasicBlock trueExit = block(19, 47);
    BasicBlock[] sideCandidate = new BasicBlock[]{new BasicBlock(6), sideExit, null};
    BasicBlock[] trueCandidate = new BasicBlock[]{new BasicBlock(18), trueExit, trueExit};

    List<BasicBlock[]> candidates = new ArrayList<>(List.of(sideCandidate, trueCandidate));
    candidates.sort(FinallyProcessor::compareNextCandidates);

    assertSame(trueCandidate, candidates.get(0));
  }

  private static BasicBlock block(int id, int offset) {
    BasicBlock block = new BasicBlock(id);
    block.getSeq().addInstruction(
      Instruction.create(CodeConstants.opc_return, false, CodeConstants.GROUP_GENERAL, JAVA_1, null, offset, 1)
    );
    block.getInstrOldOffsets().add(offset);
    return block;
  }
}
