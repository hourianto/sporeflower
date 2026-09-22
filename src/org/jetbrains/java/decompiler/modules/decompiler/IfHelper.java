// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.IfNode.EdgeType;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent.FunctionType;
import org.jetbrains.java.decompiler.modules.decompiler.exps.IfExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.util.DotExporter;

import java.util.*;

public final class IfHelper {
  public static boolean mergeAllIfs(RootStatement root) {
    boolean res = mergeAllIfsRec(root, new HashSet<>());
    if (res) {
      SequenceHelper.condenseSequences(root);
    }
    return res;
  }

  private static boolean mergeAllIfsRec(Statement stat, Set<? super Integer> setReorderedIfs) {
    boolean res = false;

    if (stat.getExprents() == null) {
      while (true) {
        boolean changed = false;

        for (Statement st : stat.getStats()) {
          res |= mergeAllIfsRec(st, setReorderedIfs);

          // collapse composed if's
          if (mergeIfs(st, setReorderedIfs)) {
            changed = true;
            break;
          }
        }

        res |= changed;

        if (!changed) {
          break;
        }
      }
    }

    return res;
  }

  public static boolean mergeIfs(Statement statement, Set<? super Integer> setReorderedIfs) {
    if (!(statement instanceof IfStatement) && !(statement instanceof SequenceStatement)) {
      return false;
    }

    boolean res = false;

    loop:
    while (true) {
      List<Statement> lst = new ArrayList<>();
      if (statement instanceof IfStatement) {
        lst.add(statement);
      } else {
        lst.addAll(statement.getStats());
      }

      boolean stsingle = (lst.size() == 1);

      for (Statement stat : lst) {
        if (stat instanceof IfStatement) {
          IfNode rtnode = IfNode.build((IfStatement) stat, stsingle);

          if (collapseIfIf(rtnode)) {
            res = true;
            ValidationHelper.validateStatement(stat.getTopParent());
            continue loop;
          }

          if (!setReorderedIfs.contains(stat.id)) {
            if (collapseIfElse(rtnode)) {
              res = true;
              ValidationHelper.validateStatement(stat.getTopParent());
              continue loop;
            }

            if (collapseElse(rtnode)) {
              res = true;
              ValidationHelper.validateStatement(stat.getTopParent());
              continue loop;
            }

            if (DecompilerContext.getOption(IFernflowerPreferences.TERNARY_CONDITIONS) && collapseTernary(rtnode)) {
              res = true;
              ValidationHelper.validateStatement(stat.getTopParent());
              continue loop;
            }

            // TODO: This should maybe be moved (probably to reorderIf)
            if (ifElseChainDenesting(rtnode)) {
              res = true;
              ValidationHelper.validateStatement(stat.getTopParent());
              continue loop;
            }
          }

          if (reorderIf((IfStatement) stat)) {
            res = true;
            ValidationHelper.validateStatement(stat.getTopParent());
            setReorderedIfs.add(stat.id);
            continue loop;
          }
        }
      }

      return res;
    }
  }

  // if-if branch (&& operation)
  // if (cond1) {
  //   if (cond2) {
  //     A // or goto A
  //   }
  //   goto B
  // }
  // B // or goto B
  // into
  // if (cond1 && cond2) {
  //   A // or goto A
  // }
  // B // or goto B
  private static boolean collapseIfIf(IfNode rtnode) {
    if (rtnode.innerType == EdgeType.DIRECT) {
      IfNode ifbranch = rtnode.innerNode;
      if (ifbranch.innerNode != null) { // make sure that ifBranch is an ifStatement
        if (rtnode.successorNode.value == ifbranch.successorNode.value) {

          IfStatement ifparent = (IfStatement) rtnode.value;
          IfStatement ifchild = (IfStatement) ifbranch.value;
          Statement ifinner = ifbranch.innerNode.value;

          if (ifchild.getFirst().getExprents().isEmpty() && !ifchild.hasPPMM()) {

            ifparent.getIfEdge().remove();
            ifchild.getFirstSuccessor().remove();
            ifparent.getStats().removeWithKey(ifchild.id);

            if (ifbranch.innerType == EdgeType.INDIRECT) { // inner code is remote (goto B case)
              ifparent.setIfstat(null);

              StatEdge ifedge = ifchild.getIfEdge();

              ifedge.changeSource(ifparent.getFirst());

              if (ifedge.closure == ifchild) {
                ifedge.closure = null;
              }

              ifparent.setIfEdge(ifedge);
            } else { // inner code is actually code (B case)
              ifchild.getIfEdge().remove();

              StatEdge ifedge = new StatEdge(StatEdge.TYPE_REGULAR, ifparent.getFirst(), ifinner);
              ifparent.getFirst().addSuccessor(ifedge);
              ifparent.setIfEdge(ifedge);
              ifparent.setIfstat(ifinner);

              ifparent.getStats().addWithKey(ifinner, ifinner.id);
              ifinner.setParent(ifparent);

              if (ifinner.hasAnySuccessor()) {
                StatEdge edge = ifinner.getFirstSuccessor();
                if (edge.closure == ifchild) {
                  edge.closure = ifparent;
                }
              }
            }

            // merge if conditions
            IfExprent statexpr = ifparent.getHeadexprent();

            List<Exprent> lstOperands = new ArrayList<>();
            lstOperands.add(statexpr.getCondition());
            lstOperands.add(ifchild.getHeadexprent().getCondition());

            statexpr.setCondition(new FunctionExprent(FunctionType.BOOLEAN_AND, lstOperands, null));
            statexpr.addBytecodeOffsets(ifchild.getHeadexprent().bytecode);

            return true;
          }
        }
      }
    }

    return false;
  }

  // if-else branch
  // if (cond1) {
  //   if (cond2) {
  //     goto A
  //   }
  //   goto B
  // }
  // A // or goto A
  // into
  // if (cond1 && !cond2) {
  //   goto B
  // }
  // A // or goto A
  private static boolean collapseIfElse(IfNode rtnode) {
    if (rtnode.innerType == EdgeType.DIRECT) {
      IfNode ifbranch = rtnode.innerNode;
      if (ifbranch.innerNode != null) { // make sure that ifBranch is an ifStatement
        if (rtnode.successorNode.value == ifbranch.innerNode.value) {

          IfStatement ifparent = (IfStatement) rtnode.value;
          IfStatement ifchild = (IfStatement) ifbranch.value;

          if (ifchild.getFirst().getExprents().isEmpty()) {

            ifparent.getIfEdge().remove();
            ifchild.getIfEdge().remove();
            ifparent.getStats().removeWithKey(ifchild.id);

            // if (cond1) {
            //   if (cond2) {
            //     goto A
            //   }
            //   goto B
            // }
            // A // or goto A

            ifparent.setIfstat(null);

            StatEdge ifedge = ifchild.getFirstSuccessor();
            ifedge.changeSource(ifparent.getFirst());
            ifparent.setIfEdge(ifedge);

            // merge if conditions
            IfExprent statexpr = ifparent.getHeadexprent();

            List<Exprent> lstOperands = new ArrayList<>();
            lstOperands.add(statexpr.getCondition());
            lstOperands.add(new FunctionExprent(FunctionType.BOOL_NOT, ifchild.getHeadexprent().getCondition(), null));
            statexpr.setCondition(new FunctionExprent(FunctionType.BOOLEAN_AND, lstOperands, null));
            statexpr.addBytecodeOffsets(ifchild.getHeadexprent().bytecode);

            return true;
          }
        }
      }
    }

    return false;
  }

  private static boolean collapseElse(IfNode rtnode) {
    if (rtnode.successorType == EdgeType.DIRECT) {
      IfNode elsebranch = rtnode.successorNode;
      if (elsebranch.innerNode != null) { // make sure that elseBranch is an ifStatement

        int path = elsebranch.successorNode.value == rtnode.innerNode.value ? 2 :
          (elsebranch.innerNode.value == rtnode.innerNode.value ? 1 : 0);

        if (path > 0) {
          // path == 1
          // if (cond1) {
          //   goto A
          // }
          // if (cond2) {
          //   goto A
          // } else {
          //   // inner code
          // }
          // into
          // if (cond1 || cond2) {
          //   goto A
          // } else {
          //   // inner code
          // }

          // path == 2
          // if (cond1) {
          //   goto A
          // }
          // if (cond2) {
          //   // inner code
          // }
          // A // or goto A
          // into
          // if (!cond1 && cond2) {
          //   // inner code
          // }
          // A // or goto A

          IfStatement firstif = (IfStatement) rtnode.value;
          IfStatement secondif = (IfStatement) elsebranch.value;
          Statement parent = firstif.getParent();

          if (secondif.getFirst().getExprents().isEmpty()) {

            firstif.getIfEdge().remove();

            // remove first if
            firstif.removeAllSuccessors(secondif);

            for (StatEdge edge : firstif.getAllPredecessorEdges()) {
              if (!firstif.containsStatementStrict(edge.getSource())) {
                // TODO: why is this check here? If this check were to fail, this if
                //  should have been a loop instead of an if statement.
                edge.changeDestination(secondif);
              }
            }

            parent.getStats().removeWithKey(firstif.id);
            if (parent.getFirst() == firstif) {
              parent.setFirst(secondif);
            }

            // merge if conditions
            IfExprent statexpr = secondif.getHeadexprent();

            List<Exprent> lstOperands = new ArrayList<>();
            lstOperands.add(firstif.getHeadexprent().getCondition());

            if (path == 2) {
              lstOperands.set(0, new FunctionExprent(FunctionType.BOOL_NOT, lstOperands.get(0), null));
            }

            lstOperands.add(statexpr.getCondition());

            statexpr
              .setCondition(new FunctionExprent(path == 1 ? FunctionType.BOOLEAN_OR : FunctionType.BOOLEAN_AND, lstOperands, null));

            if (secondif.getFirst().getExprents().isEmpty() && // second is guranteed to be empty already
                !firstif.getFirst().getExprents().isEmpty()) {

              secondif.replaceStatement(secondif.getFirst(), firstif.getFirst());
            }

            return true;
          }
        }
      } else if (elsebranch.successorNode != null) { // else branch is not an if statement, but is direct
        if (elsebranch.successorNode.value == rtnode.innerNode.value) {
          // if (cond1) {
          //   goto A
          // }
          // B
          // goto A;
          //
          // into
          //
          // if (!cond1) {
          //   B
          // }
          // goto A;

          IfStatement firstif = (IfStatement) rtnode.value;
          Statement second = elsebranch.value;

          firstif.removeAllSuccessors(second);

          for (StatEdge edge : second.getAllSuccessorEdges()) {
            edge.changeSource(firstif);
          }

          StatEdge ifedge = firstif.getIfEdge();
          ifedge.remove();

          second.addSuccessor(new StatEdge(ifedge.getType(), second, ifedge.getDestination(), ifedge.closure));

          StatEdge newifedge = new StatEdge(StatEdge.TYPE_REGULAR, firstif.getFirst(), second);
          firstif.getFirst().addSuccessor(newifedge);
          firstif.setIfEdge(newifedge);
          firstif.setIfstat(second);

          firstif.getStats().addWithKey(second, second.id);
          second.setParent(firstif);

          firstif.getParent().getStats().removeWithKey(second.id);

          // negate the if condition
          IfExprent statexpr = firstif.getHeadexprent();
          statexpr
            .setCondition(new FunctionExprent(FunctionType.BOOL_NOT, statexpr.getCondition(), null));

          return true;
        }
      }
    }
    return false;
  }

  // convert
  // if (cond1) {
  //   if (cond2) {
  //     goto A
  //   }
  //   goto B
  // } else {
  //   if (cond3) {
  //     goto A
  //   }
  //   goto B
  // }
  //
  // into
  // if (cond1 ? cond2 : cond3) {
  //   goto A
  // }
  // goto B

  // if goto A and goto B are swapped in `if (cond2)`, then cond2 is negated
  private static boolean collapseTernary(IfNode rtnode) {
    if (rtnode.innerType == EdgeType.DIRECT && rtnode.successorType == EdgeType.ELSE) {
      // if (cond1) {
      //   if (cond2) {
      //     goto A
      //   }
      //   goto B
      // } else {
      //   if (cond3) {
      //     goto A
      //   }
      //   goto B
      // }
      IfNode ifBranch = rtnode.innerNode;
      IfNode elseBranch = rtnode.successorNode;

      if (ifBranch.innerType == EdgeType.INDIRECT && ifBranch.successorType == EdgeType.INDIRECT &&
          elseBranch.innerType == EdgeType.INDIRECT && elseBranch.successorType == EdgeType.INDIRECT &&
          ifBranch.value.getFirst().getExprents().isEmpty() && elseBranch.value.getFirst().getExprents().isEmpty()) {

        boolean inverted;
        if (ifBranch.innerNode.value == elseBranch.innerNode.value &&
            ifBranch.successorNode.value == elseBranch.successorNode.value) {
          inverted = false;
        } else if (ifBranch.innerNode.value == elseBranch.successorNode.value &&
                   ifBranch.successorNode.value == elseBranch.innerNode.value) {
          inverted = true;
        } else {
          return false;
        }

        IfStatement mainIf = (IfStatement) rtnode.value;
        IfStatement firstIf = (IfStatement) ifBranch.value;
        IfStatement secondIf = (IfStatement) elseBranch.value;

        // remove first if
        mainIf.getStats().removeWithKey(firstIf.id);
        mainIf.getIfEdge().remove();
        mainIf.setIfstat(null);

        // remove second if
        mainIf.getStats().removeWithKey(secondIf.id);
        mainIf.getElseEdge().remove();
        mainIf.setElsestat(null);

        // remove unused first if's edges
        firstIf.getIfEdge().remove();
        firstIf.getFirstSuccessor().remove();

        // move second if jump to main if
        mainIf.setIfEdge(secondIf.getIfEdge());
        mainIf.getIfEdge().changeSource(mainIf.getFirst());

        // remove (weird?) if else successor, produced by dom parsing
        // TODO: remove when dom parsing is fixed
        if (mainIf.hasAnySuccessor()) {
          mainIf.getFirstSuccessor().remove();
        }

        // Check for closure, remove it
        if (secondIf.getFirstSuccessor().closure == secondIf) {
          secondIf.getFirstSuccessor().removeClosure();
        }

        // move second's successor to be the if's successor
        secondIf.getFirstSuccessor().changeSource(mainIf);
        if (mainIf.getFirstSuccessor().closure == mainIf) {
          // TODO: always removing causes some <unknownclosure> bugs, while not removing causes issues with invalid closure validations
          if (mainIf.getFirstSuccessor().getSource() != mainIf) {
            mainIf.getFirstSuccessor().removeClosure();
          }

          mainIf.getFirstSuccessor().changeType(StatEdge.TYPE_REGULAR);
        }

        // mark the if as an if and not an if else
        mainIf.iftype = IfStatement.IFTYPE_IF;
        mainIf.setElseEdge(null);

        // merge if conditions
        IfExprent statexpr = mainIf.getHeadexprent();

        List<Exprent> lstOperands = new ArrayList<>();
        lstOperands.add(mainIf.getHeadexprent().getCondition());
        lstOperands.add(firstIf.getHeadexprent().getCondition());

        if (inverted) {
          lstOperands.set(1, new FunctionExprent(FunctionType.BOOL_NOT, lstOperands.get(1), null));
        }

        lstOperands.add(secondIf.getHeadexprent().getCondition());

        statexpr.setCondition(new FunctionExprent(FunctionType.TERNARY, lstOperands, null));
        return true;
      }
    }

    return false;
  }


  // Convert
  //
  // if (condA) {
  //   if (condB) {
  //     X
  //   } else {
  //     Y
  //   }
  //   goto end
  // }
  // Z
  // end
  //
  // To
  //
  // if (!condA) {
  //   Z
  // }
  // else {
  //   if (condB) {
  //     X
  //   } else {
  //     Y
  //   }
  // }
  //
  // (Which is rendered as if/elseif/else)
  private static boolean ifElseChainDenesting(IfNode rtnode) {
    if (rtnode.innerType != EdgeType.DIRECT || rtnode.successorType == EdgeType.ELSE) {
      return false;
    }

    IfStatement outerIf = (IfStatement)rtnode.value;
    if (!(outerIf.getParent() instanceof SequenceStatement parent)) {
      return false;
    }

    Statement nestedStat = rtnode.innerNode.value;
    IfStatement nestedIf = leadingIf(nestedStat);
    if (nestedIf == null || !nestedIf.getFirst().getExprents().isEmpty() || nestedIf.getIfstat() == null) {
      return false;
    }

    // X, Y and Z must have exits past the sequence. Such an exit is not proof
    // that every path exits: a catch handler can still enter the shared tail.
    if (!hasDirectEndEdge(nestedStat, parent) || !hasDirectEndEdge(parent.getStats().getLast(), parent)
        || !hasDirectEndEdge(nestedIf.getIfstat(), parent)) {
      return false;
    }

    StatEdge successor = outerIf.getFirstSuccessor();
    if (successor.getType() != StatEdge.TYPE_REGULAR || hasIncomingEdgeFromWithin(outerIf, successor.getDestination())) {
      return false;
    }

    // Leave an existing if chain in the tail in place unless it has a prologue.
    IfStatement nextIf = leadingIf(successor.getDestination());
    if (nextIf != null && nextIf.getFirst().getExprents().isEmpty()) {
      return false;
    }

    IfExprent condition = outerIf.getHeadexprent();
    condition.setCondition(new FunctionExprent(FunctionType.BOOL_NOT, condition.getCondition(), null));
    swapBranches(outerIf, false, parent);
    return true;
  }

  private static IfStatement leadingIf(Statement statement) {
    if (statement instanceof SequenceStatement) {
      statement = statement.getFirst();
    }
    return statement instanceof IfStatement ifstat ? ifstat : null;
  }

  // FIXME: rewrite the entire method!!! keep in mind finally exits!!
  private static boolean reorderIf(IfStatement ifstat) {
    if (ifstat.iftype == IfStatement.IFTYPE_IFELSE) {
      return false;
    }

    boolean ifdirect, elsedirect;
    boolean noifstat = false, noelsestat;
    boolean ifdirectpath = false, elsedirectpath = false;

    Statement parent = ifstat.getParent();
    Statement from = parent instanceof SequenceStatement ? parent : ifstat;

    Statement next = getNextStatement(from);

    if (ifstat.getIfstat() == null) {
      noifstat = true;

      ifdirect = ifstat.getIfEdge().getType() == StatEdge.TYPE_FINALLYEXIT ||
                 MergeHelper.isDirectPath(from, ifstat.getIfEdge().getDestination());
    } else {
      List<StatEdge> lstSuccs = ifstat.getIfstat().getAllSuccessorEdges();
      ifdirect = !lstSuccs.isEmpty() && lstSuccs.get(0).getType() == StatEdge.TYPE_FINALLYEXIT ||
                 hasDirectEndEdge(ifstat.getIfstat(), from);
    }

    Statement last = parent instanceof SequenceStatement ? parent.getStats().getLast() : ifstat;
    noelsestat = (last == ifstat);

    elsedirect = !last.getAllSuccessorEdges().isEmpty() && last.getAllSuccessorEdges().get(0).getType() == StatEdge.TYPE_FINALLYEXIT ||
                 hasDirectEndEdge(last, from);

    List<StatEdge> successors = ifstat.getAllSuccessorEdges();

    if (successors.isEmpty()) {
      // Can't have no successors- something went wrong horribly somewhere!
      throw new IllegalStateException("If statement " + ifstat + " has no successors!");
    }

    if (!noelsestat && hasIncomingEdgeFromWithin(ifstat, successors.get(0).getDestination())) {
      return false;
    }

    if (!ifdirect && !noifstat) {
      ifdirectpath = hasIncomingEdgeFromWithin(ifstat, next);
    }

    if (!elsedirect && !noelsestat) {
      SequenceStatement sequence = (SequenceStatement) parent;

      for (int i = sequence.getStats().size() - 1; i >= 0; i--) {
        Statement sttemp = sequence.getStats().get(i);
        if (sttemp == ifstat) {
          break;
        } else if (hasIncomingEdgeFromWithin(sttemp, next)) {
          elsedirectpath = true;
          break;
        }
      }
    }

    if ((ifdirect || ifdirectpath) && (elsedirect || elsedirectpath) && !noifstat && !noelsestat) {  // if - then - else

      SequenceStatement sequence = (SequenceStatement) parent;
      List<Statement> tail = followingStatements(ifstat, sequence);
      if (ifdirect && sequence.getParent() instanceof RootStatement root && allowsGuardReturn(root) && next instanceof DummyExitStatement
          && preferGuard(ifstat, sequence, tail)) {
        return false;
      }

      Statement stelse = extractTail(ifstat, sequence, tail);
      StatEdge elseedge = new StatEdge(StatEdge.TYPE_REGULAR, ifstat.getFirst(), stelse);
      ifstat.getFirst().addSuccessor(elseedge);
      ifstat.setElsestat(stelse);
      ifstat.setElseEdge(elseedge);

      ifstat.getStats().addWithKey(stelse, stelse.id);
      stelse.setParent(ifstat);

      ifstat.iftype = IfStatement.IFTYPE_IFELSE;
    } else if (ifdirect && (!elsedirect || (noifstat && !noelsestat)) && !ifstat.getAllSuccessorEdges().isEmpty()) {  // if - then
      // negate the if condition
      IfExprent statexpr = ifstat.getHeadexprent();
      statexpr.setCondition(new FunctionExprent(FunctionType.BOOL_NOT, statexpr.getCondition(), null));

      if (noelsestat) {
        StatEdge ifedge = ifstat.getIfEdge();
        StatEdge elseedge = ifstat.getFirstSuccessor();

        if (noifstat) {
          ifstat.getFirst().removeSuccessor(ifedge);
          ifstat.removeSuccessor(elseedge);

          ifedge.setSource(ifstat);
          elseedge.setSource(ifstat.getFirst());

          ifstat.addSuccessor(ifedge);
          ifstat.getFirst().addSuccessor(elseedge);

          ifstat.setIfEdge(elseedge);
        } else {
          Statement ifbranch = ifstat.getIfstat();
          SequenceStatement newseq = new SequenceStatement(Arrays.asList(ifstat, ifbranch));

          ifstat.getFirst().removeSuccessor(ifedge);
          ifstat.getStats().removeWithKey(ifbranch.id);
          ifstat.setIfstat(null);

          ifstat.removeSuccessor(elseedge);
          elseedge.setSource(ifstat.getFirst());
          ifstat.getFirst().addSuccessor(elseedge);

          ifstat.setIfEdge(elseedge);

          ifstat.getParent().replaceStatement(ifstat, newseq);
          newseq.setAllParent();

          ifstat.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, ifstat, ifbranch));
        }
      } else {
        swapBranches(ifstat, noifstat, (SequenceStatement) parent);
      }
    } else {
      return false;
    }

    return true;
  }

  private static void swapBranches(IfStatement ifstat, boolean noifstat, SequenceStatement parent) {
    ValidationHelper.validateTrue(ifstat.iftype == IfStatement.IFTYPE_IF, "This method is meant for swapping the branches of non if-else IfStatements");
    Statement stelse = extractTail(ifstat, parent, followingStatements(ifstat, parent));

    if (noifstat) {
      StatEdge ifedge = ifstat.getIfEdge();

      ifstat.getFirst().removeSuccessor(ifedge);
      ifedge.setSource(ifstat);
      ifstat.addSuccessor(ifedge);
    } else {
      Statement ifbranch = ifstat.getIfstat();

      ifstat.getFirst().removeSuccessor(ifstat.getIfEdge());
      ifstat.getStats().removeWithKey(ifbranch.id);

      ifstat.addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, ifstat, ifbranch));

      parent.getStats().addWithKey(ifbranch, ifbranch.id);
      ifbranch.setParent(parent);
    }

    StatEdge newifedge = new StatEdge(StatEdge.TYPE_REGULAR, ifstat.getFirst(), stelse);
    ifstat.getFirst().addSuccessor(newifedge);
    ifstat.setIfstat(stelse);
    ifstat.setIfEdge(newifedge);

    ifstat.getStats().addWithKey(stelse, stelse.id);
    stelse.setParent(ifstat);
  }

  private static List<Statement> followingStatements(IfStatement ifstat, SequenceStatement sequence) {
    List<Statement> statements = sequence.getStats();
    return new ArrayList<>(statements.subList(statements.indexOf(ifstat) + 1, statements.size()));
  }

  private static Statement extractTail(IfStatement ifstat, SequenceStatement sequence, List<Statement> tail) {
    Statement result = tail.size() == 1 ? tail.get(0) : new SequenceStatement(tail);
    if (tail.size() > 1) result.setAllParent();
    ifstat.removeSuccessor(ifstat.getFirstSuccessor());
    for (Statement statement : tail) sequence.getStats().removeWithKey(statement.id);
    return result;
  }

  private static int statementSize(Statement statement) {
    if (statement.getExprents() != null) return statement.getExprents().size();
    int size = statement.getStatExprents().size();
    for (Statement child : statement.getStats()) size += statementSize(child);
    return size;
  }

  private static boolean preferGuard(IfStatement ifstat, SequenceStatement sequence, List<Statement> tail) {
    // An else-if chain does not indent the following alternatives. Keep it
    // available to condition simplification, unlike a non-terminating if at
    // the start of a method's remaining work.
    IfStatement nextIf = leadingIf(tail.get(0));
    if (nextIf != null && nextIf.getFirst().getExprents().isEmpty()
        && (nextIf.getIfstat() == null || hasDirectEndEdge(nextIf.getIfstat(), sequence))) {
      return false;
    }
    // A guard needs an explicit return, whereas an else can share the method's
    // terminal return. Account for that extra statement before choosing to
    // leave the longer continuation at its existing indentation level.
    return tail.stream().mapToInt(IfHelper::statementSize).sum() > statementSize(ifstat.getIfstat()) + 1;
  }

  private static boolean allowsGuardReturn(RootStatement root) {
    // A static initializer cannot contain return statements in Java. Keep
    // value-returning branches available for conditional-value reconstruction;
    // this layout choice concerns only an early return from a void method.
    return root.mt != null && !CodeConstants.CLINIT_NAME.equals(root.mt.getName()) && root.mt.getDescriptor().endsWith(")V");
  }

  private static boolean hasDirectEndEdge(Statement stat, Statement from) {

    for (StatEdge edge : stat.getAllSuccessorEdges()) {
      if (MergeHelper.isDirectPath(from, edge.getDestination())) {
        return true;
      }
    }

    if (stat.getExprents() == null) {
      switch (stat.type) {
        case SEQUENCE:
          return hasDirectEndEdge(stat.getStats().getLast(), from);
        case CATCH_ALL:
        case TRY_CATCH:
          for (Statement st : stat.getStats()) {
            if (hasDirectEndEdge(st, from)) {
              return true;
            }
          }
          break;
        case IF:
          IfStatement ifstat = (IfStatement) stat;
          if (ifstat.iftype == IfStatement.IFTYPE_IFELSE) {
            return hasDirectEndEdge(ifstat.getIfstat(), from) ||
                   hasDirectEndEdge(ifstat.getElsestat(), from);
          }
          break;
        case SYNCHRONIZED:
          return hasDirectEndEdge(stat.getStats().get(1), from);
        case SWITCH:
          for (Statement st : stat.getStats()) {
            if (hasDirectEndEdge(st, from)) {
              return true;
            }
          }
      }
    }

    return false;
  }

  private static Statement getNextStatement(Statement stat) {
    Statement parent = stat.getParent();
    switch (parent.type) {
      case ROOT:
        return ((RootStatement) parent).getDummyExit();
      case DO:
        return parent;
      case SEQUENCE:
        SequenceStatement sequence = (SequenceStatement) parent;
        if (sequence.getStats().getLast() != stat) {
          for (int i = sequence.getStats().size() - 1; i >= 0; i--) {
            if (sequence.getStats().get(i) == stat) {
              return sequence.getStats().get(i + 1);
            }
          }
        }
    }

    return getNextStatement(parent);
  }

  // A structured successor may be shared by exits inside the preceding if,
  // even after label cleanup makes their breaks implicit or lowers their closure.
  // Exclude the enclosing statement's own edge, which is ordinary sequencing.
  private static boolean hasIncomingEdgeFromWithin(Statement from, Statement to) {
    for (StatEdge edge : to.getAllPredecessorEdges()) {
      if (from.containsStatementStrict(edge.getSource())) {
        return true;
      }
    }

    return false;
  }

  // Try to make if/else statements that unconditionally return more pleasing to read.
  // TODO: swap branches with negated float ops
  public static boolean prettifyIfs(Statement stat) {
    boolean changed = false;
    for (Statement st : new ArrayList<>(stat.getStats())) {
      // Don't bother inside switches
      if (st instanceof SwitchStatement) {
        continue;
      }

      prettifyIfs(st);

      if (st instanceof IfStatement ifSt) {
        changed |= prettifyIf(ifSt);
      }
    }

    return changed;
  }

  private static boolean prettifyIf(IfStatement ifSt) {
    if (ifSt.iftype == IfStatement.IFTYPE_IFELSE && !ifSt.isPatternMatched()) {
      Statement ifStat = ifSt.getIfstat();
      Statement elseStat = ifSt.getElsestat();
      if (ifStat != null && isExit(ifStat)) {
        // TODO: if the same var exprents are found in the if and an if-inside-the-else, try to keep as else if

        // If the inside of the if is only one return/throw, be more aggressive in reordering
        boolean force = ifStat.getExprents().size() == 1 && elseStat.getExprents() != null && elseStat.getExprents().size() > 1;
        if (!force) {
          if (isExit(elseStat)) {
            return false;
          }

          if (elseStat instanceof IfStatement elseIf && elseIf.getIfstat() != null && isExit(elseIf.getIfstat())) {
            return false;
          }
        }

        ifSt.iftype = IfStatement.IFTYPE_IF;
        SequenceStatement seq = new SequenceStatement(ifSt, elseStat);

        StatEdge elseEdge = ifSt.getElseEdge();
        ifSt.getStats().removeWithKey(elseStat.id);

        ifSt.setElsestat(null);
        ifSt.setElseEdge(null);
        ifSt.replaceWith(seq);
        elseEdge.changeSource(ifSt);
        seq.setAllParent();

        return true;
      }
    }

    return false;
  }

  private static boolean isExit(Statement stat) {
    if (!stat.hasAnyDirectSuccessor()) {
      return false;
    }

    StatEdge suc = stat.getFirstDirectSuccessor();

    if (!(suc.getType() == StatEdge.TYPE_BREAK && suc.getDestination() instanceof DummyExitStatement)) {
      return false;
    }

    return stat instanceof BasicBlockStatement bb && !bb.getExprents().isEmpty() && bb.getExprents().get(bb.getExprents().size() - 1) instanceof ExitExprent;
  }
}
