// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.code;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.code.*;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.code.cfg.ControlFlowGraph;
import org.jetbrains.java.decompiler.code.cfg.ExceptionRangeCFG;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.GenericDominatorEngine;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.IGraph;
import org.jetbrains.java.decompiler.modules.decompiler.decompose.IGraphNode;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.util.*;
import java.util.Map.Entry;

public final class ExceptionDeobfuscator {

  private static final class Range {
    private final BasicBlock handler;
    private final String uniqueStr;
    private final Set<BasicBlock> protectedRange;
    private final List<ExceptionRangeCFG> rangeCFGs;

    private Range(BasicBlock handler, String uniqueStr, Set<BasicBlock> protectedRange, ExceptionRangeCFG rangeCFG) {
      this.handler = handler;
      this.uniqueStr = uniqueStr;
      this.protectedRange = protectedRange;
      this.rangeCFGs = new ArrayList<>();
      this.rangeCFGs.add(rangeCFG);
    }

    private ExceptionRangeCFG getRepresentativeRange() {
      return rangeCFGs.get(0);
    }
  }

  private static List<Range> aggregateRanges(ControlFlowGraph graph) {
    List<Range> lstRanges = new ArrayList<>();

    for (ExceptionRangeCFG range : graph.getExceptions()) {
      boolean found = false;
      for (Range arr : lstRanges) {
        if (arr.handler == range.getHandler() && InterpreterUtil.equalObjects(range.getUniqueExceptionsString(), arr.uniqueStr)) {
          arr.protectedRange.addAll(range.getProtectedRange());
          arr.rangeCFGs.add(range);
          found = true;
          break;
        }
      }

      if (!found) {
        // doesn't matter, which range chosen
        lstRanges.add(new Range(range.getHandler(), range.getUniqueExceptionsString(), new HashSet<>(range.getProtectedRange()), range));
      }
    }

    return lstRanges;
  }

  public static void restorePopRanges(ControlFlowGraph graph) {

    List<Range> lstRanges = aggregateRanges(graph);

    // process aggregated ranges
    for (Range range : lstRanges) {

      if (range.uniqueStr != null) {

        BasicBlock handler = range.handler;
        InstructionSequence seq = handler.getSeq();

        Instruction firstinstr;
        if (seq.length() > 0) {
          firstinstr = seq.getInstr(0);

          if (firstinstr.opcode == CodeConstants.opc_pop ||
              firstinstr.opcode == CodeConstants.opc_astore) {
            Set<BasicBlock> setrange = new HashSet<>(range.protectedRange);

            for (Range range_super : lstRanges) { // finally or strict superset

              if (range != range_super) {

                Set<BasicBlock> setrange_super = new HashSet<>(range_super.protectedRange);

                if (!setrange.contains(range_super.handler) && !setrange_super.contains(handler)
                    && (range_super.uniqueStr == null || setrange_super.containsAll(setrange))) {

                  if (range_super.uniqueStr == null) {
                    setrange_super.retainAll(setrange);
                  }
                  else {
                    setrange_super.removeAll(setrange);
                  }

                  if (!setrange_super.isEmpty()) {

                    BasicBlock newblock = handler;

                    // split the handler
                    if (seq.length() > 1) {
                      newblock = new BasicBlock(++graph.last_id);
                      InstructionSequence newseq = new InstructionSequence();
                      newseq.addInstruction(firstinstr.clone());

                      newblock.setSeq(newseq);
                      newblock.getInstrOldOffsets().add(handler.getOldOffset(0));
                      graph.getBlocks().addWithKey(newblock, newblock.id);


                      List<BasicBlock> lstTemp = new ArrayList<>();
                      lstTemp.addAll(handler.getPreds());
                      lstTemp.addAll(handler.getPredExceptions());

                      // replace predecessors
                      for (BasicBlock pred : lstTemp) {
                        pred.replaceSuccessor(handler, newblock);
                      }

                      // replace handler
                      for (ExceptionRangeCFG range_ext : graph.getExceptions()) {
                        if (range_ext.getHandler() == handler) {
                          range_ext.setHandler(newblock);
                        }
                        else if (range_ext.getProtectedRange().contains(handler)) {
                          newblock.addSuccessorException(range_ext.getHandler());
                          range_ext.getProtectedRange().add(newblock);
                        }
                      }

                      newblock.addSuccessor(handler);
                      if (graph.getFirst() == handler) {
                        graph.setFirst(newblock);
                      }

                      // remove the first pop in the handler
                      seq.removeInstruction(0);
                      handler.getInstrOldOffsets().remove(0);
                    }

                    newblock.addSuccessorException(range_super.handler);
                    range_super.getRepresentativeRange().getProtectedRange().add(newblock);

                    handler = range.getRepresentativeRange().getHandler();
                    seq = handler.getSeq();
                  }
                }
              }
            }
          }
        }
      }
    }
  }

  public static void insertEmptyExceptionHandlerBlocks(ControlFlowGraph graph) {

    Set<BasicBlock> setVisited = new HashSet<>();

    for (ExceptionRangeCFG range : graph.getExceptions()) {
      BasicBlock handler = range.getHandler();

      if (setVisited.contains(handler)) {
        continue;
      }
      setVisited.add(handler);

      BasicBlock emptyblock = new BasicBlock(++graph.last_id);
      graph.getBlocks().addWithKey(emptyblock, emptyblock.id);

      // only exception predecessors considered
      List<BasicBlock> lstTemp = new ArrayList<>(handler.getPredExceptions());

      // replace predecessors
      for (BasicBlock pred : lstTemp) {
        pred.replaceSuccessor(handler, emptyblock);
      }

      // replace handler
      for (ExceptionRangeCFG range_ext : graph.getExceptions()) {
        if (range_ext.getHandler() == handler) {
          range_ext.setHandler(emptyblock);
        }
        else if (range_ext.getProtectedRange().contains(handler)) {
          emptyblock.addSuccessorException(range_ext.getHandler());
          range_ext.getProtectedRange().add(emptyblock);
        }
      }

      emptyblock.addSuccessor(handler);
      if (graph.getFirst() == handler) {
        graph.setFirst(emptyblock);
      }
    }
  }

  public static void removeEmptyRanges(ControlFlowGraph graph) {

    List<ExceptionRangeCFG> lstRanges = graph.getExceptions();
    for (int i = lstRanges.size() - 1; i >= 0; i--) {
      ExceptionRangeCFG range = lstRanges.get(i);

      boolean isEmpty = true;
      for (BasicBlock block : range.getProtectedRange()) {
        if (!block.getSeq().isEmpty()) {
          isEmpty = false;
          break;
        }
      }

      if (isEmpty) {
        for (BasicBlock block : range.getProtectedRange()) {
          block.removeSuccessorException(range.getHandler());
        }

        lstRanges.remove(i);
        graph.addComment("$VF: Removed empty exception range");
      }
    }
  }

  public static void removeCircularRanges(final ControlFlowGraph graph) {

    GenericDominatorEngine engine = new GenericDominatorEngine(new IGraph() {
      @Override
      public List<? extends IGraphNode> getReversePostOrderList() {
        return graph.getReversePostOrder();
      }

      @Override
      public Set<? extends IGraphNode> getRoots() {
        return new HashSet<>(Collections.singletonList(graph.getFirst()));
      }
    });

    engine.initialize();

    List<ExceptionRangeCFG> lstRanges = graph.getExceptions();
    for (int i = lstRanges.size() - 1; i >= 0; i--) {
      ExceptionRangeCFG range = lstRanges.get(i);

      BasicBlock handler = range.getHandler();
      List<BasicBlock> rangeList = range.getProtectedRange();

      if (rangeList.contains(handler)) {  // TODO: better removing strategy

        List<BasicBlock> lstRemBlocks = getReachableBlocksRestricted(range.getHandler(), range, engine);

        if (lstRemBlocks.size() < rangeList.size() || rangeList.size() == 1) {
          for (BasicBlock block : lstRemBlocks) {
            block.removeSuccessorException(handler);
            rangeList.remove(block);
          }
        }

        if (rangeList.isEmpty()) {
          lstRanges.remove(i);
        }
      }
    }
  }

  private static List<BasicBlock> getReachableBlocksRestricted(BasicBlock start, ExceptionRangeCFG range, GenericDominatorEngine engine) {

    List<BasicBlock> lstRes = new ArrayList<>();

    LinkedList<BasicBlock> stack = new LinkedList<>();
    Set<BasicBlock> setVisited = new HashSet<>();

    stack.addFirst(start);

    while (!stack.isEmpty()) {
      BasicBlock block = stack.removeFirst();

      if (!setVisited.add(block)) {
        continue;
      }

      if (range.getProtectedRange().contains(block) && engine.isDominator(block, start)) {
        lstRes.add(block);

        List<BasicBlock> lstSuccs = new ArrayList<>(block.getSuccs());
        lstSuccs.addAll(block.getSuccExceptions());

        stack.addAll(lstSuccs);
      }
    }

    return lstRes;
  }

  public static boolean hasObfuscatedExceptions(ControlFlowGraph graph) {
    for (Range range : aggregateRanges(graph)) {
      Set<BasicBlock> setEntries = new HashSet<>();

      for (BasicBlock block : range.protectedRange) {
        Set<BasicBlock> setTemp = new HashSet<>(block.getPreds());
        setTemp.removeAll(range.protectedRange);

        if (!setTemp.isEmpty()) {
          setEntries.add(block);
        }
      }

      if (range.protectedRange.contains(graph.getFirst())) {
        setEntries.add(graph.getFirst());
      }

      if (!setEntries.isEmpty()) {
        if (setEntries.size() > 1 /*|| ent.getValue().contains(first)*/) {
          return true;
        }
      }
    }

    return false;
  }

  // Some compilers leave local/control-only connector blocks outside otherwise
  // logical try/finally regions. These blocks cannot throw into the handler, but
  // keeping them outside can give the structurer a loop whose latch is outside
  // the protected range and whose header is inside it.
  public static boolean normalizeSparseExceptionRanges(ControlFlowGraph graph) {
    boolean changed = false;

    for (Range range : aggregateRanges(graph)) {
      LinkedHashSet<BasicBlock> protectedBlocks = new LinkedHashSet<>(range.protectedRange);

      closeOverSafeConnectors(graph, protectedBlocks);

      if (protectedBlocks.size() == range.protectedRange.size()) {
        continue;
      }

      if (range.rangeCFGs.size() == 1) {
        replaceRangeContents(graph, range.getRepresentativeRange(), protectedBlocks);
        changed = true;
      }
      else if (getRegularRangeEntries(graph, protectedBlocks).size() <= 1) {
        replaceRangeContents(graph, range.getRepresentativeRange(), protectedBlocks);
        graph.getExceptions().removeAll(range.rangeCFGs.subList(1, range.rangeCFGs.size()));
        changed = true;
      }
    }

    return changed;
  }

  private static void closeOverSafeConnectors(ControlFlowGraph graph, Set<BasicBlock> protectedBlocks) {
    boolean changed;
    do {
      changed = false;

      for (BasicBlock block : graph.getBlocks()) {
        if (protectedBlocks.contains(block) || !isSafeExceptionRangeConnector(block)) {
          continue;
        }

        List<BasicBlock> preds = block.getPreds();
        List<BasicBlock> succs = block.getSuccs();
        if (!preds.isEmpty() && !succs.isEmpty() &&
            protectedBlocks.containsAll(preds) &&
            protectedBlocks.containsAll(succs)) {
          protectedBlocks.add(block);
          changed = true;
        }
      }
    }
    while (changed);
  }

  private static Set<BasicBlock> getRegularRangeEntries(ControlFlowGraph graph, Set<BasicBlock> protectedBlocks) {
    Set<BasicBlock> entries = new HashSet<>();

    for (BasicBlock block : protectedBlocks) {
      Set<BasicBlock> preds = new HashSet<>(block.getPreds());
      preds.removeAll(protectedBlocks);

      if (!preds.isEmpty()) {
        entries.add(block);
      }
    }

    if (protectedBlocks.contains(graph.getFirst())) {
      entries.add(graph.getFirst());
    }

    return entries;
  }

  private static void replaceRangeContents(ControlFlowGraph graph, ExceptionRangeCFG range, Set<BasicBlock> protectedBlocks) {
    List<BasicBlock> ordered = new ArrayList<>();
    for (BasicBlock block : graph.getBlocks()) {
      if (protectedBlocks.contains(block)) {
        ordered.add(block);
        block.addSuccessorException(range.getHandler());
      }
    }

    range.getProtectedRange().clear();
    range.getProtectedRange().addAll(ordered);
  }

  private static boolean isSafeExceptionRangeConnector(BasicBlock block) {
    for (Instruction instr : block.getSeq()) {
      if (!isLocalControlInstruction(instr)) {
        return false;
      }
    }

    return true;
  }

  private static boolean isLocalControlInstruction(Instruction instr) {
    int opcode = instr.opcode;

    if (opcode == CodeConstants.opc_nop ||
        opcode == CodeConstants.opc_iinc ||
        opcode == CodeConstants.opc_goto ||
        opcode == CodeConstants.opc_goto_w) {
      return true;
    }

    if (opcode >= CodeConstants.opc_aconst_null && opcode <= CodeConstants.opc_sipush) {
      return true;
    }

    if (opcode >= CodeConstants.opc_iload && opcode <= CodeConstants.opc_aload_3) {
      return true;
    }

    if (opcode >= CodeConstants.opc_istore && opcode <= CodeConstants.opc_astore_3) {
      return true;
    }

    if (opcode >= CodeConstants.opc_pop && opcode <= CodeConstants.opc_swap) {
      return true;
    }

    if (opcode >= CodeConstants.opc_iadd && opcode <= CodeConstants.opc_lxor &&
        opcode != CodeConstants.opc_idiv &&
        opcode != CodeConstants.opc_ldiv &&
        opcode != CodeConstants.opc_irem &&
        opcode != CodeConstants.opc_lrem) {
      return true;
    }

    if (opcode >= CodeConstants.opc_i2l && opcode <= CodeConstants.opc_dcmpg) {
      return true;
    }

    return opcode >= CodeConstants.opc_ifeq && opcode <= CodeConstants.opc_goto ||
           opcode == CodeConstants.opc_ifnull ||
           opcode == CodeConstants.opc_ifnonnull;
  }

  public static boolean handleMultipleEntryExceptionRanges(ControlFlowGraph graph) {
    GenericDominatorEngine engine = new GenericDominatorEngine(new IGraph() {
      @Override
      public List<? extends IGraphNode> getReversePostOrderList() {
        return graph.getReversePostOrder();
      }

      @Override
      public Set<? extends IGraphNode> getRoots() {
        return new HashSet<>(Collections.singletonList(graph.getFirst()));
      }
    });

    engine.initialize();

    boolean found;

    while (true) {
      found = false;
      boolean splitted = false;

      for (ExceptionRangeCFG range : graph.getExceptions()) {
        // map of entry points to entry sources (null indicating method start)
        LinkedHashMap<BasicBlock, List<@Nullable BasicBlock>> setEntries = getRangeEntries(range, graph.getFirst());

        if (setEntries.size() > 1) { // multiple-entry protected range
          found = true;

          if (growExceptionRange(range, setEntries)) {
            splitted = true;
            graph.addComment("$VF: Handled exception range with multiple entry points by expanding it");
            break;
          } else if (splitExceptionRange(range, setEntries.keySet(), graph, engine)) {
            splitted = true;
            graph.addComment("$VF: Handled exception range with multiple entry points by splitting it");
            break;
          }
        }
      }

      if (!splitted) {
        break;
      }
    }

    return !found;
  }

  static LinkedHashMap<BasicBlock, List<@Nullable BasicBlock>> getRangeEntries(ExceptionRangeCFG range, BasicBlock first) {
    LinkedHashMap<BasicBlock, List<@Nullable BasicBlock>> setEntries = new LinkedHashMap<>();
    Set<BasicBlock> setRange = new HashSet<>(range.getProtectedRange());

    for (BasicBlock block : range.getProtectedRange()) {
      List<@Nullable BasicBlock> setPreds = new ArrayList<>(block.getPreds());
      setPreds.removeAll(setRange);
      if (block == first) {
        setPreds.add(null);
      }

      if (!setPreds.isEmpty()) {
        setEntries.put(block, setPreds);
      }
    }

    return setEntries;
  }

  static boolean growExceptionRange(
    ExceptionRangeCFG range,
    LinkedHashMap<BasicBlock, List<@Nullable BasicBlock>> setEntries
  ) {
    // Try to inline basic blocks that can't throw, but are only reachable from within this handler
    //  and have successors in this handler
    // Each try handler must have at least one entry point that can't be removed
    @Nullable BasicBlock missed = null;

    for (var entry : setEntries.entrySet()) {
      BasicBlock destination = entry.getKey();
      if (!validateExceptionGrow(range, entry.getValue())) {
        if (missed == null) {
          missed = destination;
          continue;
        }
        return false; // At least 2 entry points exist that can't be fixed
      }
    }

    // A closed connector cycle can make every apparent entry expandable. Growing all of them would leave no canonical
    // entry for the range, so retain the original range and let the structural splitter handle it instead.
    if (missed == null) {
      return false;
    }

    // Cleanup is possible. Add blocks to exception range.
    Set<BasicBlock> handled = new HashSet<>();
    for (var entry : setEntries.entrySet()) {
      if (missed == entry.getKey()) {
        continue;
      }
      for (BasicBlock block : entry.getValue()) {
        ValidationHelper.notNull(block);
        if (!handled.add(block)) { // prevent handling twice
          continue;
        }
        block.addSuccessorException(range.getHandler());
        range.getProtectedRange().add(block);
      }
    }
    return true;
  }

  private static boolean validateExceptionGrow(
    ExceptionRangeCFG range,
    List<@Nullable BasicBlock> blocks
  ) {
    Set<BasicBlock> protectedRange = new HashSet<>(range.getProtectedRange());
    for (BasicBlock block : blocks) {
      // can't optimize away method entry
      if (block == null) {
        return false;
      }

      if (!protectedRange.containsAll(block.getPredecessors())) {
        // limit to inlining single blocks, not sequences of blocks
        return false;
      }

      for (Instruction inst : block.getSeq()) {
        if (!inst.canNotThrow()) {
          // We don't want to catch extra exceptions
          //  many instructions can only throw a very small number of exceptions
          //  so we could actually filter more lenient based on the caught exception types
          return false;
        }
      }
    }
    return true;
  }

  private static boolean splitExceptionRange(ExceptionRangeCFG range,
                                             Set<BasicBlock> setEntries,
                                             ControlFlowGraph graph,
                                             GenericDominatorEngine engine) {
    List<BasicBlock> subrangeBlocks = selectSubrangeToSplit(range, setEntries, engine);
    if (subrangeBlocks == null) {
      DecompilerContext.getLogger().writeMessage("Inconsistency found while splitting protected range", IFernflowerLogger.Severity.WARN);
      return false;
    }

    ExceptionRangeCFG subRange = new ExceptionRangeCFG(subrangeBlocks, range.getHandler(), range.getExceptionTypes());
    graph.getExceptions().add(subRange);
    range.getProtectedRange().removeAll(subrangeBlocks);
    return true;
  }

  static List<BasicBlock> selectSubrangeToSplit(ExceptionRangeCFG range,
                                                 Collection<BasicBlock> entries,
                                                 GenericDominatorEngine engine) {
    BasicBlock selectedEntry = null;
    List<BasicBlock> selectedSubrange = null;
    int selectedEntryDepth = -1;

    for (BasicBlock entry : entries) {
      List<BasicBlock> subrange = getReachableBlocksRestricted(entry, range, engine);
      if (subrange.isEmpty() || subrange.size() >= range.getProtectedRange().size()) {
        continue;
      }

      int entryDepth = 0;
      for (BasicBlock otherEntry : entries) {
        if (entry != otherEntry && engine.isDominator(entry, otherEntry)) {
          entryDepth++;
        }
      }

      // Sparse protected ranges can contain nested entries separated by an unprotected connector. Carve out the deepest
      // entry first so an outer entry cannot claim its region merely because an identity-based set happened to iterate
      // that entry first. Incomparable entries have disjoint dominator regions; the block ID only stabilizes their order.
      if (selectedEntry == null ||
          entryDepth > selectedEntryDepth ||
          entryDepth == selectedEntryDepth && entry.getId() > selectedEntry.getId()) {
        selectedEntry = entry;
        selectedSubrange = subrange;
        selectedEntryDepth = entryDepth;
      }
    }

    return selectedSubrange;
  }

  public static void insertDummyExceptionHandlerBlocks(ControlFlowGraph graph, BytecodeVersion bytecode_version) {
    Map<BasicBlock, List<ExceptionRangeCFG>> mapRanges = new HashMap<>();
    for (ExceptionRangeCFG range : graph.getExceptions()) {
      mapRanges.computeIfAbsent(range.getHandler(), k -> new ArrayList<>()).add(range);
    }

    for (Entry<BasicBlock, List<ExceptionRangeCFG>> ent : mapRanges.entrySet()) {
      BasicBlock handler = ent.getKey();
      List<ExceptionRangeCFG> ranges = ent.getValue();

      if (ranges.size() == 1) {
        continue;
      }

      if (!DecompilerContext.getOption(IFernflowerPreferences.OLD_TRY_DEDUP)) {
        for (int i = 1; i < ranges.size(); i++) {
          ExceptionRangeCFG range = ranges.get(i);

          // Duplicate block now
          BasicBlock newBlock = new BasicBlock(++graph.last_id);
          newBlock.setSeq(handler.getSeq().clone());
          newBlock.getInstrOldOffsets().addAll(handler.getInstrOldOffsets());

          graph.getBlocks().addWithKey(newBlock, newBlock.id);

          // only exception predecessors from this range considered
          List<BasicBlock> lstPredExceptions = new ArrayList<>(handler.getPredExceptions());
          lstPredExceptions.retainAll(range.getProtectedRange());

          // replace predecessors
          for (BasicBlock pred : lstPredExceptions) {
            pred.replaceSuccessor(handler, newBlock);
          }
          range.setHandler(newBlock);

          // Add successors

          for (BasicBlock succ : handler.getSuccs()) {
            newBlock.addSuccessor(succ);
          }

          for (BasicBlock succ : handler.getSuccExceptions()) {
            newBlock.addSuccessorException(succ);
            var excRange = graph.getExceptionRange(succ, handler);
            excRange.getProtectedRange().add(newBlock);
          }
        }

        if (!isMatchException(handler)) {
          graph.addComment("$VF: Duplicated exception handlers to handle obfuscated exceptions");
        }

      } else {
        for (ExceptionRangeCFG range : ranges) {

          // add some dummy instructions to prevent optimizing away the empty block
          InstructionSequence seq = new InstructionSequence();
          seq.addInstruction(Instruction.create(CodeConstants.opc_bipush, false, CodeConstants.GROUP_GENERAL, bytecode_version, new int[]{0}, -1, 1));
          seq.addInstruction(Instruction.create(CodeConstants.opc_pop, false, CodeConstants.GROUP_GENERAL, bytecode_version, null, -1, 1));

          BasicBlock dummyBlock = new BasicBlock(++graph.last_id);
          dummyBlock.setSeq(seq);

          graph.getBlocks().addWithKey(dummyBlock, dummyBlock.id);

          // only exception predecessors from this range considered
          List<BasicBlock> lstPredExceptions = new ArrayList<>(handler.getPredExceptions());
          lstPredExceptions.retainAll(range.getProtectedRange());

          // replace predecessors
          for (BasicBlock pred : lstPredExceptions) {
            pred.replaceSuccessor(handler, dummyBlock);
          }

          // replace handler
          range.setHandler(dummyBlock);
          // add common exception edges
          Set<BasicBlock> commonHandlers = new HashSet<>(handler.getSuccExceptions());
          for (BasicBlock pred : lstPredExceptions) {
            commonHandlers.retainAll(pred.getSuccExceptions());
          }
          // TODO: more sanity checks?
          for (BasicBlock commonHandler : commonHandlers) {
            ExceptionRangeCFG commonRange = graph.getExceptionRange(commonHandler, handler);

            dummyBlock.addSuccessorException(commonHandler);
            commonRange.getProtectedRange().add(dummyBlock);
          }

          dummyBlock.addSuccessor(handler);

          graph.addComment("$VF: Inserted dummy exception handlers to handle obfuscated exceptions");
        }
      }
    }
  }

  private static boolean isMatchException(BasicBlock block) {
    StructClass cl = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS);

    // Check if block has any "new MatchException;"
    for (Instruction instr : block.getSeq()) {
      if (instr.opcode == CodeConstants.opc_new) {
        if ("java/lang/MatchException".equals(cl.getPool().getPrimitiveConstant(instr.operand(0)).getString())) {
          return true;

        }
      }
    }

    return false;
  }
}
