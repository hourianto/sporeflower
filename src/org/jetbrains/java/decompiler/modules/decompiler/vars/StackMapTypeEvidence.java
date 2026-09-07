package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.Instruction;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.code.cfg.ControlFlowGraph;
import org.jetbrains.java.decompiler.main.decompiler.CancelationManager;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructStackMapAttribute;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Attaches exact legacy StackMap constraints to the local definitions reaching each frame.
 * This uses original bytecode, independently of CFG rewrites that remove frame offsets. Expression
 * bytecode mappings are for source correspondence, not value identity or instruction order.
 */
public final class StackMapTypeEvidence implements CodeConstants {
  // CFG restructuring may clone instructions. Match the original access, while excluding
  // synthetic instructions and any replacement that addresses a different slot.
  private record Access(int offset, int opcode, int slot) {
    private Access(Instruction instruction) {
      this(instruction.startOffset, instruction.opcode, instruction.operand(0));
    }
  }

  private final Map<Access, Set<VarType>> reads = new HashMap<>();
  private final Map<Access, Set<VarType>> writes = new HashMap<>();

  public Set<VarType> getReadTypes(Instruction instruction) {
    return isLoad(instruction) || instruction.opcode == opc_iinc
      ? reads.getOrDefault(new Access(instruction), Set.of()) : Set.of();
  }

  public Set<VarType> getWriteTypes(Instruction instruction) {
    return isStore(instruction) || instruction.opcode == opc_iinc
      ? writes.getOrDefault(new Access(instruction), Set.of()) : Set.of();
  }

  public static StackMapTypeEvidence analyze(StructMethod method) {
    StackMapTypeEvidence evidence = new StackMapTypeEvidence();
    StructStackMapAttribute frames = method.getAttribute(StructGeneralAttribute.ATTRIBUTE_STACK_MAP);
    if (frames == null || frames.getEntries().isEmpty()) {
      return evidence;
    }

    // CLDC preverification eliminates subroutines. Do not infer through a jsr/ret graph
    // without modeling its return-address context; ordinary expression inference still works.
    for (Instruction instruction : method.getInstructionSequence()) {
      if (instruction.opcode == opc_jsr || instruction.opcode == opc_jsr_w || instruction.opcode == opc_ret) {
        return evidence;
      }
    }

    new Analysis(method, frames).collect(evidence);
    return evidence;
  }

  private static final class Analysis {
    private final StructStackMapAttribute frames;
    private final ControlFlowGraph graph;
    private final Map<Instruction, Integer> definitions = new HashMap<>();
    private final List<Integer> slots = new ArrayList<>();
    private final List<Set<VarType>> types = new ArrayList<>();
    private final Map<Integer, BitSet> startingAt = new HashMap<>();
    private final Map<Integer, BitSet> occupying = new HashMap<>();
    private final Map<BasicBlock, BitSet> inputs = new HashMap<>();

    private Analysis(StructMethod method, StructStackMapAttribute frames) {
      this.frames = frames;
      graph = new ControlFlowGraph(method.getInstructionSequence());
      BitSet parameters = new BitSet();
      int slot = 0;
      if (!method.hasModifier(ACC_STATIC)) {
        parameters.set(addDefinition(slot++, 1));
      }
      for (VarType parameter : MethodDescriptor.parseDescriptor(method.getDescriptor()).params) {
        parameters.set(addDefinition(slot, parameter.stackSize));
        slot += parameter.stackSize;
      }
      for (Instruction instruction : method.getInstructionSequence()) {
        if (isStore(instruction) || instruction.opcode == opc_iinc) {
          definitions.put(instruction, addDefinition(instruction.operand(0), storeSize(instruction)));
        }
      }
      inputs.put(graph.getFirst(), parameters);
    }

    private int addDefinition(int slot, int size) {
      int definition = slots.size();
      slots.add(slot);
      types.add(new HashSet<>());
      startingAt.computeIfAbsent(slot, ignored -> new BitSet()).set(definition);
      for (int i = slot; i < slot + size; i++) {
        occupying.computeIfAbsent(i, ignored -> new BitSet()).set(definition);
      }
      return definition;
    }

    private void collect(StackMapTypeEvidence evidence) {
      ArrayDeque<BasicBlock> work = new ArrayDeque<>();
      Set<BasicBlock> queued = new HashSet<>();
      work.add(graph.getFirst());
      queued.add(graph.getFirst());
      while (!work.isEmpty()) {
        CancelationManager.checkCanceled();
        BasicBlock block = work.removeFirst();
        queued.remove(block);
        BitSet state = (BitSet)inputs.get(block).clone();
        BitSet exceptional = new BitSet();
        for (Instruction instruction : block.getSeq()) {
          // A handler may see a definition from anywhere in the protected block,
          // not just the last store. Using all instruction entries is conservative.
          exceptional.or(state);
          transfer(instruction, state);
        }
        for (BasicBlock successor : block.getSuccs()) {
          enqueue(successor, state, work, queued);
        }
        for (BasicBlock successor : block.getSuccExceptions()) {
          enqueue(successor, exceptional, work, queued);
        }
      }

      Map<Instruction, BitSet> uses = new HashMap<>();
      for (BasicBlock block : graph.getBlocks()) {
        BitSet input = inputs.get(block);
        if (input == null) {
          continue;
        }
        BitSet state = (BitSet)input.clone();
        for (Instruction instruction : block.getSeq()) {
          StructStackMapAttribute.StackMapEntry frame = frames.getFrame(instruction.startOffset);
          if (frame != null) {
            for (int definition = state.nextSetBit(0); definition >= 0; definition = state.nextSetBit(definition + 1)) {
              VarType type = frame.getLocalType(slots.get(definition));
              if (type != null) {
                types.get(definition).add(type);
              }
            }
          }
          if (isLoad(instruction) || instruction.opcode == opc_iinc) {
            BitSet reaching = (BitSet)state.clone();
            reaching.and(startingAt.getOrDefault(instruction.operand(0), new BitSet()));
            uses.put(instruction, reaching);
          }
          // A frame is the incoming state: constrain the previous definition before
          // killing it, never the value that this store is about to define.
          transfer(instruction, state);
        }
      }

      for (Map.Entry<Instruction, BitSet> use : uses.entrySet()) {
        Set<VarType> observed = new HashSet<>();
        BitSet reaching = use.getValue();
        for (int definition = reaching.nextSetBit(0); definition >= 0; definition = reaching.nextSetBit(definition + 1)) {
          observed.addAll(types.get(definition));
        }
        if (!observed.isEmpty()) {
          evidence.reads.put(new Access(use.getKey()), Set.copyOf(observed));
        }
      }
      definitions.forEach((instruction, definition) -> {
        if (!types.get(definition).isEmpty()) {
          evidence.writes.put(new Access(instruction), Set.copyOf(types.get(definition)));
        }
      });
    }

    private void transfer(Instruction instruction, BitSet state) {
      Integer definition = definitions.get(instruction);
      if (definition == null) {
        return;
      }
      int slot = instruction.operand(0);
      for (int i = slot; i < slot + storeSize(instruction); i++) {
        // Writing either half invalidates a previous long/double definition.
        state.andNot(occupying.get(i));
      }
      state.set(definition);
    }

    private void enqueue(BasicBlock block, BitSet incoming, ArrayDeque<BasicBlock> work, Set<BasicBlock> queued) {
      BitSet previous = inputs.get(block);
      boolean changed;
      if (previous == null) {
        inputs.put(block, (BitSet)incoming.clone());
        changed = true;
      } else {
        int size = previous.cardinality();
        previous.or(incoming);
        changed = size != previous.cardinality();
      }
      if (changed && queued.add(block)) {
        work.addLast(block);
      }
    }
  }

  private static boolean isLoad(Instruction instruction) {
    return instruction.opcode >= opc_iload && instruction.opcode <= opc_aload;
  }

  private static boolean isStore(Instruction instruction) {
    return instruction.opcode >= opc_istore && instruction.opcode <= opc_astore;
  }

  private static int storeSize(Instruction instruction) {
    return instruction.opcode == opc_lstore || instruction.opcode == opc_dstore ? 2 : 1;
  }
}
