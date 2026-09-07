// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.stats;

import org.jetbrains.java.decompiler.modules.decompiler.DecHelper;
import org.jetbrains.java.decompiler.modules.decompiler.ExprProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent.FunctionType;
import org.jetbrains.java.decompiler.struct.match.IMatchable;
import org.jetbrains.java.decompiler.struct.match.MatchEngine;
import org.jetbrains.java.decompiler.struct.match.MatchNode;
import org.jetbrains.java.decompiler.util.StartEndPair;
import org.jetbrains.java.decompiler.util.TextBuffer;
import org.jetbrains.java.decompiler.util.TextUtil;

import java.util.ArrayList;
import java.util.List;


public class IfStatement extends Statement {

  public static final int IFTYPE_IF = 0;
  public static final int IFTYPE_IFELSE = 1;

  public int iftype;

  // *****************************************************************************
  // private fields
  // *****************************************************************************

  private Statement ifstat;
  private Statement elsestat;

  private StatEdge ifedge;
  private StatEdge elseedge;

  private boolean negated = false;
  private boolean patternMatched = false;

  private boolean hasPPMM = false;

  private final List<Exprent> headexprent = new ArrayList<>(1); // contains IfExprent

  // *****************************************************************************
  // constructors
  // *****************************************************************************

  protected IfStatement() {
    super(StatementType.IF);

    headexprent.add(null);
  }

  protected IfStatement(Statement head, int regedges, Statement postst) {

    this();

    first = head;
    stats.addWithKey(head, head.id);

    List<StatEdge> lstHeadSuccs = head.getSuccessorEdges(STATEDGE_DIRECT_ALL);

    switch (regedges) {
      case 0:
        ifstat = null;
        elsestat = null;

        break;
      case 1:
        ifstat = null;
        elsestat = null;

        StatEdge edgeif = lstHeadSuccs.get(1);
        if (edgeif.getType() != StatEdge.TYPE_REGULAR) {
          post = lstHeadSuccs.get(0).getDestination();
        }
        else {
          post = edgeif.getDestination();
          negated = true;
        }
        break;
      case 2:
        elsestat = lstHeadSuccs.get(0).getDestination();
        ifstat = lstHeadSuccs.get(1).getDestination();

        List<StatEdge> lstSucc = ifstat.getSuccessorEdges(StatEdge.TYPE_REGULAR);
        List<StatEdge> lstSucc1 = elsestat.getSuccessorEdges(StatEdge.TYPE_REGULAR);

        if (ifstat.getPredecessorEdges(StatEdge.TYPE_REGULAR).size() > 1 || lstSucc.size() > 1) {
          post = ifstat;
        }
        else if (elsestat.getPredecessorEdges(StatEdge.TYPE_REGULAR).size() > 1 || lstSucc1.size() > 1) {
          post = elsestat;
        }
        else {
          if (lstSucc.size() == 0) {
            post = elsestat;
          }
          else if (lstSucc1.size() == 0) {
            post = ifstat;
          }
        }

        if (ifstat == post) {
          if (elsestat != post) {
            ifstat = elsestat;
            negated = true;
          }
          else {
            ifstat = null;
          }
          elsestat = null;
        }
        else if (elsestat == post) {
          elsestat = null;
        }
        else {
          post = postst;
        }

        if (elsestat == null) {
          regedges = 1;  // if without else
        }
    }

    ifedge = lstHeadSuccs.get(negated ? 0 : 1);
    elseedge = (regedges == 2) ? lstHeadSuccs.get(negated ? 1 : 0) : null;

    iftype = (regedges == 2) ? IFTYPE_IFELSE : IFTYPE_IF;

    if (iftype == IFTYPE_IF) {
      if (regedges == 0) {
        StatEdge edge = lstHeadSuccs.get(0);
        head.removeSuccessor(edge);
        edge.setSource(this);
        this.addSuccessor(edge);
      }
      else if (regedges == 1) {
        StatEdge edge = lstHeadSuccs.get(negated ? 1 : 0);
        head.removeSuccessor(edge);
      }
    }

    if (ifstat != null) {
      stats.addWithKey(ifstat, ifstat.id);
    }

    if (elsestat != null) {
      stats.addWithKey(elsestat, elsestat.id);
    }

    if (post == head) {
      post = this;
    }
  }


  // *****************************************************************************
  // public methods
  // *****************************************************************************

  public static Statement isHead(Statement head) {

    if (head instanceof BasicBlockStatement && head.getLastBasicType() == LastBasicType.IF) {
      int regsize = head.getSuccessorEdges(StatEdge.TYPE_REGULAR).size();

      Statement p = null;

      boolean ok = (regsize < 2);
      if (!ok) {
        List<Statement> lst = new ArrayList<>();
        if (DecHelper.isChoiceStatement(head, lst)) {
          p = lst.remove(0);

          for (Statement st : lst) {
            if (st.isMonitorEnter()) {
              return null;
            }
          }

          ok = DecHelper.checkStatementExceptions(lst);
        }
      }

      if (ok) {
        return new IfStatement(head, regsize, p);
      }
    }

    return null;
  }

  /** Builds a source-only if statement while preserving the normal statement graph invariants. */
  public static IfStatement createSourceOnly(Exprent condition, Statement ifBody) {
    IfStatement statement = new IfStatement();
    statement.iftype = IFTYPE_IF;
    statement.first = BasicBlockStatement.create();
    statement.ifstat = ifBody;
    statement.stats.addWithKey(statement.first, statement.first.id);
    statement.stats.addWithKey(ifBody, ifBody.id);
    statement.first.setParent(statement);
    ifBody.setParent(statement);
    statement.headexprent.set(0, IfExprent.create(condition));
    statement.ifedge = new StatEdge(StatEdge.TYPE_REGULAR, statement.first, ifBody);
    statement.first.addSuccessor(statement.ifedge);
    return statement;
  }

  @Override
  public TextBuffer toJava(int indent) {
    TextBuffer buf = new TextBuffer();

    buf.append(ExprProcessor.listToJava(varDefinitions, indent));

    buf.append(first.toJava(indent));

    if (isLabeled()) {
      buf.appendIndent(indent).append("label").append(this.id).append(":").appendLineSeparator();
    }

    Exprent condition = headexprent.get(0);
    buf.appendIndent(indent);
    // Condition can be null in early processing stages
    if (condition != null) {
      buf.append(condition.toJava(indent));
    } else {
      buf.append("if <null condition>");
    }
    buf.append(" {").appendLineSeparator();

    if (ifstat == null) {
      boolean semicolon = false;
      if (ifedge.explicit) {
        semicolon = true;
        if (ifedge.getType() == StatEdge.TYPE_BREAK) {
          // break
          buf.appendIndent(indent + 1).append("break");
        }
        else {
          // continue
          buf.appendIndent(indent + 1).append("continue");
        }

        if (ifedge.labeled) {
          buf.append(" label").append(ifedge.closure == null ? "<unknownclosure>" : Integer.toString(ifedge.closure.id));
        }
      }
      if(semicolon) {
        buf.append(";").appendLineSeparator();
      }
    }
    else {
      buf.append(ExprProcessor.jmpWrapper(ifstat, indent + 1, true));
    }

    boolean elseif = false;

    if (elsestat != null) {
      if (elsestat instanceof IfStatement
          && elsestat.varDefinitions.isEmpty() && (elsestat.getFirst().getExprents() != null && elsestat.getFirst().getExprents().isEmpty()) &&
          !elsestat.isLabeled() &&
          (elsestat.getSuccessorEdges(STATEDGE_DIRECT_ALL).isEmpty()
           || !elsestat.getSuccessorEdges(STATEDGE_DIRECT_ALL).get(0).explicit)) { // else if
        buf.appendIndent(indent).append("} else ");

        TextBuffer content = ExprProcessor.jmpWrapper(elsestat, indent, false);
        content.setStart(TextUtil.getIndentString(indent).length());
        buf.append(content);

        elseif = true;
      }
      else {
        TextBuffer content = ExprProcessor.jmpWrapper(elsestat, indent + 1, false);

        if (content.length() > 0) {
          buf.appendIndent(indent).append("} else {").appendLineSeparator();
          buf.append(content);
        }
      }
    }

    if (!elseif) {
      buf.appendIndent(indent).append("}").appendLineSeparator();
    }

    return buf;
  }

  @Override
  public void initExprents() {
    IfExprent ifexpr = (IfExprent)first.getExprents().remove(first.getExprents().size() - 1);

    if (negated) {
      ifexpr = (IfExprent)ifexpr.copy();
      ifexpr.negateIf();
    }

    headexprent.set(0, ifexpr);
  }

  @Override
  public List<Exprent> getStatExprents() {
    return new ArrayList<>(headexprent);
  }

  @Override
  public void replaceExprent(Exprent oldexpr, Exprent newexpr) {
    if (headexprent.get(0) == oldexpr) {
      headexprent.set(0, newexpr);
    }
  }

  @Override
  public void replaceStatement(Statement oldstat, Statement newstat) {
    super.replaceStatement(oldstat, newstat);

    if (ifstat == oldstat) {
      ifstat = newstat;
    }

    if (elsestat == oldstat) {
      elsestat = newstat;
    }

    List<StatEdge> lstSuccs = first.getSuccessorEdges(STATEDGE_DIRECT_ALL);

    if (iftype == IFTYPE_IF) {
      ifedge = lstSuccs.get(0);
      elseedge = null;
    }
    else {
      StatEdge edge0 = lstSuccs.get(0);
      StatEdge edge1 = lstSuccs.get(1);
      if (edge0.getDestination() == ifstat) {
        ifedge = edge0;
        elseedge = edge1;
      }
      else {
        ifedge = edge1;
        elseedge = edge0;
      }
    }
  }

  @Override
  public boolean hasBasicSuccEdge() {
    return iftype == IFTYPE_IF;
  }

  @Override
  public Statement getSimpleCopy() {
    IfStatement is = new IfStatement();
    is.iftype = this.iftype;
    is.negated = this.negated;

    return is;
  }

  @Override
  public void initSimpleCopy() {
    first = stats.get(0);

    List<StatEdge> lstSuccs = first.getSuccessorEdges(STATEDGE_DIRECT_ALL);
    ifedge = lstSuccs.get((iftype == IFTYPE_IF || negated) ? 0 : 1);
    if (stats.size() > 1) {
      ifstat = stats.get(1);
    }

    if (iftype == IFTYPE_IFELSE) {
      elseedge = lstSuccs.get(negated ? 1 : 0);
      elsestat = stats.get(2);
    }
  }

  // *****************************************************************************
  // getter and setter methods
  // *****************************************************************************

  public Statement getElsestat() {
    return elsestat;
  }

  public void setElsestat(Statement elsestat) {
    this.elsestat = elsestat;
  }

  public Statement getIfstat() {
    return ifstat;
  }

  public void setIfstat(Statement ifstat) {
    this.ifstat = ifstat;
  }

  public boolean isNegated() {
    return negated;
  }

  public void setNegated(boolean negated) {
    this.negated = negated;
  }

  public List<Exprent> getHeadexprentList() {
    return headexprent;
  }

  public IfExprent getHeadexprent() {
    return (IfExprent)headexprent.get(0);
  }

  public void setElseEdge(StatEdge elseedge) {
    this.elseedge = elseedge;
  }

  public void setIfEdge(StatEdge ifedge) {
    this.ifedge = ifedge;
  }

  public StatEdge getIfEdge() {
    return ifedge;
  }

  public StatEdge getElseEdge() {
    return elseedge;
  }

  public boolean isPatternMatched() {
    return patternMatched;
  }

  public void setPatternMatched(boolean patternMatched) {
    this.patternMatched = patternMatched;
  }

  public boolean hasPPMM() {
    return hasPPMM;
  }

  public void setHasPPMM(boolean hasPPMM) {
    this.hasPPMM = hasPPMM;
  }

  @Override
  public List<VarExprent> getImplicitlyDefinedVars() {
    return List.of();
  }

  @Override
  public StartEndPair getStartEndRange() {
    return StartEndPair.join(super.getStartEndRange(),
      ifstat != null ? ifstat.getStartEndRange() : null,
      elsestat != null ? elsestat.getStartEndRange(): null);
  }

  // *****************************************************************************
  // IMatchable implementation
  // *****************************************************************************

  @Override
  public IMatchable findObject(MatchNode matchNode, int index) {
    IMatchable object = super.findObject(matchNode, index);
    if (object != null) {
      return object;
    }

    if (matchNode.getType() == MatchNode.MATCHNODE_EXPRENT) {
      String position = (String)matchNode.getRuleValue(MatchProperties.EXPRENT_POSITION);
      if ("head".equals(position)) {
        return getHeadexprent();
      }
    }

    return null;
  }

  @Override
  public boolean match(MatchNode matchNode, MatchEngine engine) {
    if (!super.match(matchNode, engine)) {
      return false;
    }

    Integer type = (Integer) matchNode.getRuleValue(MatchProperties.STATEMENT_IFTYPE);
    return type == null || this.iftype == type;
  }

  /** Rewrites if (condition) {} else {body}, retaining the empty branch's continuation. */
  public boolean simplifyEmptyIfBranch() {
    Statement empty = ifstat;
    if (iftype != IFTYPE_IFELSE || !(empty instanceof BasicBlockStatement) || empty.getExprents() == null ||
        !empty.getExprents().isEmpty() || !empty.getVarDefinitions().isEmpty() || !empty.getLabelEdges().isEmpty()) {
      return false;
    }

    List<StatEdge> exits = empty.getAllSuccessorEdges();
    List<StatEdge> successors = getAllSuccessorEdges();
    if (exits.size() > 1 || successors.size() > 1 || exits.isEmpty() && successors.isEmpty()) {
      return false;
    }
    StatEdge continuation = exits.isEmpty() ? successors.get(0) : exits.get(0);
    if (continuation.explicit || continuation.getType() == StatEdge.TYPE_EXCEPTION ||
        continuation.getDestination() == empty ||
        !successors.isEmpty() && successors.get(0).getDestination() != continuation.getDestination()) {
      return false;
    }
    List<StatEdge> incoming = empty.getAllPredecessorEdges();
    boolean shared = incoming.size() > 1;
    if (incoming.stream().anyMatch(edge -> edge.getType() == StatEdge.TYPE_EXCEPTION) || shared && exits.isEmpty()) {
      return false;
    }

    ifedge.remove();
    stats.removeWithKey(empty.id);
    iftype = IFTYPE_IF;
    ifstat = elsestat;
    elsestat = null;
    ifedge = elseedge;
    elseedge = null;
    negated = !negated;
    headexprent.set(0, ((IfExprent)getHeadexprent().copy()).negateIf());

    if (shared) {
      // This block is also a join, often a shared terminal return whose expression
      // was removed. Keep it after the if: bypassing it with edges to the dummy
      // exit would suppress explicit switch/labeled breaks during Java emission.
      SequenceStatement sequence = new SequenceStatement(this, empty);
      parent.replaceStatement(this, sequence);
      sequence.setAllParent();
      addSuccessor(new StatEdge(StatEdge.TYPE_REGULAR, this, empty));
      // replaceStatement lifts this if's labels to the sequence. Breaks to the
      // join still leave just the if, whereas breaks beyond it leave the sequence.
      for (StatEdge edge : new ArrayList<>(sequence.getLabelEdges())) {
        if (edge.getDestination() == empty) edge.changeClosure(this);
      }
    } else {
      if (successors.isEmpty()) {
        continuation.changeSource(this);
        if (continuation.closure == this) continuation.changeClosure(parent);
      } else if (!exits.isEmpty()) {
        continuation.remove();
      }
      empty.setParent(null);
    }
    return true;
  }
}
