// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement.EdgeDirection;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;


public final class LoopExtractHelper {


  public static boolean extractLoops(Statement root) {

    boolean res = (extractLoopsRec(root) != 0);

    if (res) {
      SequenceHelper.condenseSequences(root);
    }

    return res;
  }


  private static int extractLoopsRec(Statement stat) {

    boolean res = false;

    while (true) {

      boolean updated = false;

      for (Statement st : new ArrayList<>(stat.getStats())) {
        int extr = extractLoopsRec(st);
        res |= (extr != 0);

        if (extr == 2) {
          updated = true;
          break;
        }
      }

      if (!updated) {
        break;
      }
    }

    if (stat instanceof DoStatement) {
      if (extractLoop((DoStatement)stat)) {
        ValidationHelper.validateStatement(stat.getTopParent());
        return 2;
      }
    }

    return res ? 1 : 0;
  }

  private static boolean extractLoop(DoStatement stat) {
    if (stat.getLooptype() != DoStatement.Type.INFINITE) {
      return false;
    }

    List<Statement> stats = new ArrayList<>();
    for (StatEdge edge : stat.getLabelEdges()) {
      if (edge.getType() != StatEdge.TYPE_CONTINUE && !(edge.getDestination() instanceof DummyExitStatement)) {
        if (edge.getType() == StatEdge.TYPE_BREAK && isExternStatement(stat, edge.getSource(), edge.getSource())) {
          stats.add(edge.getSource());
        }
        else {
          return false;
        }
      }
    }

    if (!stats.isEmpty()) { // In this case prioritize first to help the Loop enhancer
      if (stat.getParent().getStats().getLast() != stat) {
        return false;
      }
    }

    if (!extractFirstIf(stat, stats)) {
      return extractLastIf(stat, stats);
    }
    else {
      return true;
    }
  }

  private static boolean extractLastIf(DoStatement stat, List<Statement> stats) {

    // search for an if condition at the end of the loop
    Statement last = stat.getFirst();
    while (last instanceof SequenceStatement) {
      last = last.getStats().getLast();
    }

    if (last instanceof IfStatement) {
      IfStatement lastif = (IfStatement)last;
      if (lastif.iftype == IfStatement.IFTYPE_IF && lastif.getIfstat() != null) {
        Statement ifstat = lastif.getIfstat();
        if (lastif.getAllSuccessorEdges().isEmpty()) {
          return false;
        }
        StatEdge elseedge = lastif.getAllSuccessorEdges().get(0);

        if (elseedge.getType() == StatEdge.TYPE_CONTINUE && elseedge.closure == stat) {

          Set<Statement> set = stat.getNeighboursSet(StatEdge.TYPE_CONTINUE, EdgeDirection.BACKWARD);
          set.remove(last);

          if (set.isEmpty()) { // no direct continues in a do{}while loop
            if (isExternStatement(stat, ifstat, ifstat)) {
              Statement first = stat.getFirst();
              while (first instanceof SequenceStatement) {
                first = first.getFirst();
              }
              if (first instanceof DoStatement && ((DoStatement)first).getLooptype() == DoStatement.Type.INFINITE) {
                return false;
              }

              for (Statement s : stats) {
                if (!ifstat.containsStatement(s)) {
                  return false;
                }
              }
              extractIfBlock(stat, lastif);
              return true;
            }
          }
        }
      }
    }
    return false;
  }

  // Extracts if statements out of the first if statement and modifies return -> break <label>
  private static boolean extractFirstIf(DoStatement stat, List<Statement> stats) {

    // search for an if condition at the entrance of the loop
    Statement first = stat.getFirst();
    while (first instanceof SequenceStatement) {
      first = first.getFirst();
    }

    // found an if statement
    if (first instanceof IfStatement) {
      IfStatement firstif = (IfStatement)first;

      if (firstif.getFirst().getExprents().isEmpty()) {

        if (firstif.iftype == IfStatement.IFTYPE_IF && firstif.getIfstat() != null) {
          Statement ifstat = firstif.getIfstat();

          if (isExternStatement(stat, ifstat, ifstat)) {
            for (Statement s : stats) {
              if (!ifstat.containsStatement(s)) {
                return false;
              }
            }

            // Extract if block
            extractIfBlock(stat, firstif);

            return true;
          } else {
            return extractExitTail(stat, firstif);
          }
        }
      }
    }

    return false;
  }


  private static boolean isExternStatement(DoStatement loop, Statement block, Statement stat) {

    for (StatEdge edge : stat.getAllSuccessorEdges()) {
      if (loop.containsStatement(edge.getDestination()) &&
          !block.containsStatement(edge.getDestination())) {
        return false;
      }
    }

    for (Statement st : stat.getStats()) {
      if (!isExternStatement(loop, block, st)) {
        return false;
      }
    }

    return true;
  }


  // Moves the body of the if statement to be after the loop, and replaces the body with a break to get to get there
  private static void extractIfBlock(DoStatement loop, IfStatement ifstat) {
    // If body is the target we want to extract
    Statement target = ifstat.getIfstat();
    // Edge from head to if body
    StatEdge ifedge = ifstat.getIfEdge();

    // Remove if body
    ifstat.setIfstat(null);
    // Add break, remove if statement
    ifedge.getSource().changeEdgeType(EdgeDirection.FORWARD, ifedge, StatEdge.TYPE_BREAK);
    ifedge.closure = loop;
    ifstat.getStats().removeWithKey(target.id);

    // label the break edge
    loop.addLabeledEdge(ifedge);

    // Lift target statement from inside if statement to neighbor of loop
    // Makes a sequence with loop and target statement and replaces the loop with it
    SequenceStatement block = new SequenceStatement(Arrays.asList(loop, target));
    loop.getParent().replaceStatement(loop, block);
    block.setAllParent();

    // Add regular successor from the loop to the extracted block
    loop.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, loop, target));

    // Add label edges to loop from created block
    for (StatEdge edge : new ArrayList<>(block.getLabelEdges())) {
      if (edge.getType() == StatEdge.TYPE_CONTINUE || edge == ifedge) {
        loop.addLabeledEdge(edge);
      }
    }

    // Find predecessor continues and replace their source from the block to the loop, makes continues from the loop body flow to the proper statement
    for (StatEdge edge : block.getPredecessorEdges(StatEdge.TYPE_CONTINUE)) {
      if (loop.containsStatementStrict(edge.getSource())) {
        block.removePredecessor(edge);
        edge.getSource().changeEdgeNode(EdgeDirection.FORWARD, edge, loop);
        loop.addPredecessor(edge);
      }
    }

    List<StatEdge> link = target.getPredecessorEdges(StatEdge.TYPE_BREAK);
    if (link.size() == 1) {
      link.get(0).canInline = false;
    }
  }

  // while (true) { if (condition) { body; continue; } tail; }
  // The false edge enters tail, not an arbitrary successor of an enclosing statement.
  // Move the complete tail out before letting MergeHelper promote the loop guard.
  private static boolean extractExitTail(DoStatement loop, IfStatement guard) {
    if (!(loop.getFirst() instanceof SequenceStatement sequence) || sequence.getFirst() != guard ||
        sequence.getStats().size() < 2 || guard.getAllSuccessorEdges().size() != 1) {
      return false;
    }

    StatEdge exit = guard.getFirstSuccessor();
    Statement next = sequence.getStats().get(1);
    if (exit.getType() != StatEdge.TYPE_REGULAR || exit.getDestination() != next) {
      return false;
    }

    // The true branch must repeat the loop or leave it altogether. A path into
    // the tail would need a separate break and cannot become ordinary fallthrough.
    Set<StatEdge> bodyExits = new HashSet<>();
    TryWithResourcesProcessor.findEdgesLeaving(guard.getIfstat(), guard.getIfstat(), bodyExits);
    boolean repeats = false;
    for (StatEdge edge : bodyExits) {
      if (edge.getDestination() == loop && edge.getType() == StatEdge.TYPE_CONTINUE) {
        repeats = true;
      } else if (loop.containsStatement(edge.getDestination())) {
        return false;
      }
    }
    if (!repeats) {
      return false;
    }

    List<Statement> tail = new ArrayList<>(sequence.getStats().subList(1, sequence.getStats().size()));
    Set<Statement> tailTree = new HashSet<>();
    List<Statement> pending = new ArrayList<>(tail);
    for (int i = 0; i < pending.size(); i++) {
      Statement statement = pending.get(i);
      tailTree.add(statement);
      pending.addAll(statement.getStats());
    }
    for (Statement statement : tailTree) {
      for (StatEdge edge : statement.getAllSuccessorEdges()) {
        if (loop.containsStatement(edge.getDestination()) && !tailTree.contains(edge.getDestination())) {
          return false; // The tail still participates in this loop, including through an exception handler.
        }
      }
    }

    guard.changeEdgeType(EdgeDirection.FORWARD, exit, StatEdge.TYPE_BREAK);
    loop.addLabeledEdge(exit);
    for (Statement statement : tail) {
      sequence.getStats().removeWithKey(statement.id);
    }

    List<Statement> extracted = new ArrayList<>();
    extracted.add(loop);
    extracted.addAll(tail);
    SequenceStatement replacement = new SequenceStatement(extracted);
    loop.replaceWith(replacement);
    replacement.setAllParent();

    // Replacing the loop redirects its predecessors and labels to the wrapper.
    // Backedges must still reach the loop itself, while exits keep their scope.
    for (StatEdge edge : replacement.getPredecessorEdges(StatEdge.TYPE_CONTINUE)) {
      if (loop.containsStatementStrict(edge.getSource())) {
        replacement.removePredecessor(edge);
        edge.getSource().changeEdgeNode(EdgeDirection.FORWARD, edge, loop);
        loop.addPredecessor(edge);
        loop.addLabeledEdge(edge);
      }
    }
    loop.addLabeledEdge(exit);
    loop.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, loop, next));
    exit.canInline = false;
    return true;
  }
}
