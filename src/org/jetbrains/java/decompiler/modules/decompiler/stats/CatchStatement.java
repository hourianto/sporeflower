// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.stats;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.cfg.BasicBlock;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.CounterContainer;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.CheckedExceptionAnalyzer;
import org.jetbrains.java.decompiler.modules.decompiler.DecHelper;
import org.jetbrains.java.decompiler.modules.decompiler.ExprProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.StatEdge;
import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.TextBuffer;

import java.util.ArrayList;
import java.util.BitSet;
import java.util.List;
import java.util.Set;

public class CatchStatement extends Statement {
  private final List<List<String>> exctstrings = new ArrayList<>();
  private final List<VarExprent> vars = new ArrayList<>();
  private final List<Exprent> resources = new ArrayList<>();

  // *****************************************************************************
  // constructors
  // *****************************************************************************

  protected CatchStatement() {
    super(StatementType.TRY_CATCH);
  }

  protected CatchStatement(Statement head, Statement next, Set<Statement> setHandlers) {
    this();

    first = head;
    stats.addWithKey(first, first.id);

    for (StatEdge edge : head.getSuccessorEdges(StatEdge.TYPE_EXCEPTION)) {
      Statement stat = edge.getDestination();

      if (setHandlers.contains(stat)) {
        stats.addWithKey(stat, stat.id);
        exctstrings.add(new ArrayList<>(edge.getExceptions()));
        
        vars.add(new VarExprent(DecompilerContext.getCounterContainer().getCounterAndIncrement(CounterContainer.VAR_COUNTER),
                                new VarType(CodeType.OBJECT, 0, edge.getExceptions().get(0)),
                                // FIXME: for now simply the first type. Should get the first common superclass when possible.
                                DecompilerContext.getVarProcessor()));
      }
    }

    if (next != null) {
      post = next;
    }
  }

  // *****************************************************************************
  // public methods
  // *****************************************************************************

  public static Statement isHead(Statement head) {
    if (head.getLastBasicType() != LastBasicType.GENERAL) {
      return null;
    }

    Set<Statement> setHandlers = DecHelper.getUniquePredExceptions(head);
    if (!setHandlers.isEmpty()) {
      int hnextcount = 0; // either no statements with connection to next, or more than 1

      Statement next = null;
      List<StatEdge> lstHeadSuccs = head.getSuccessorEdges(STATEDGE_DIRECT_ALL);
      if (!lstHeadSuccs.isEmpty() && lstHeadSuccs.get(0).getType() == StatEdge.TYPE_REGULAR) {
        next = lstHeadSuccs.get(0).getDestination();
        hnextcount = 2;
      }

      for (StatEdge edge : head.getSuccessorEdges(StatEdge.TYPE_EXCEPTION)) {
        Statement stat = edge.getDestination();

        boolean handlerok = true;

        if (edge.getExceptions() != null && setHandlers.contains(stat)) {
          if (stat.getLastBasicType() != LastBasicType.GENERAL) {
            handlerok = false;
          } else {
            List<StatEdge> lstStatSuccs = stat.getSuccessorEdges(STATEDGE_DIRECT_ALL);
            if (!lstStatSuccs.isEmpty() && lstStatSuccs.get(0).getType() == StatEdge.TYPE_REGULAR) {

              Statement statn = lstStatSuccs.get(0).getDestination();

              if (next == null) {
                next = statn;
              } else if (next != statn) {
                handlerok = false;
              }

              if (handlerok) {
                hnextcount++;
              }
            }
          }
        } else {
          handlerok = false;
        }

        if (!handlerok) {
          setHandlers.remove(stat);
        }
      }

      if (hnextcount != 1 && !setHandlers.isEmpty()) {
        List<Statement> lst = new ArrayList<>();
        lst.add(head);
        lst.addAll(setHandlers);

        for (Statement st : lst) {
          if (st.isMonitorEnter()) {
            return null;
          }
        }

        if (DecHelper.invalidHeadMerge(head)) {
          return null;
        }

        if (DecHelper.checkStatementExceptions(lst)) {
          return new CatchStatement(head, next, setHandlers);
        }
      }
    }
    return null;
  }

  @Override
  public TextBuffer toJava(int indent) {
    TextBuffer buf = new TextBuffer();
    List<String> renderedCatchTypesSoFar = new ArrayList<>();
    List<String> reachabilityMarkerTypes = new ArrayList<>();
    List<RenderedCatchClause> renderedCatchClauses = new ArrayList<>();
    CheckedExceptionAnalyzer exceptionAnalyzer = CheckedExceptionAnalyzer.active();
    if (exceptionAnalyzer == null) {
      exceptionAnalyzer = new CheckedExceptionAnalyzer();
    }

    boolean splitLegacyMultiCatch = shouldSplitLegacyMultiCatch();
    for (int i = 1; i < stats.size(); i++) {
      Statement stat = stats.get(i);
      List<String> exceptionTypes = exctstrings.get(i - 1);
      CheckedExceptionAnalyzer.CatchRewrite rewrite = exceptionAnalyzer.rewriteCatchTypes(
        first,
        exceptionTypes,
        renderedCatchTypesSoFar,
        collectFollowingCatchTypes(exctstrings, i)
      );
      List<String> renderedTypes = rewrite.getRenderedTypes();
      renderedCatchTypesSoFar.addAll(renderedTypes);
      addUniqueReachabilityMarkers(reachabilityMarkerTypes, rewrite.getReachabilityMarkerTypes());

      VarExprent var = vars.get(i - 1);
      validateType(exceptionTypes, var.getVarType());

      Integer bytecodeOffset = getCatchBytecodeOffset(stat);
      if (splitLegacyMultiCatch && renderedTypes.size() > 1) {
        for (int excIndex = 0; excIndex < renderedTypes.size(); ++excIndex) {
          renderedCatchClauses.add(new RenderedCatchClause(stat, var, List.of(renderedTypes.get(excIndex)), rewrite, excIndex == 0, bytecodeOffset));
        }
      }
      else {
        renderedCatchClauses.add(new RenderedCatchClause(stat, var, renderedTypes, rewrite, true, bytecodeOffset));
      }
    }

    buf.append(ExprProcessor.listToJava(varDefinitions, indent));

    if (isLabeled()) {
      buf.appendIndent(indent).append("label").append(this.id).append(":").appendLineSeparator();
    }

    if (resources.isEmpty()) {
      buf.appendIndent(indent).append("try {").appendLineSeparator();
    }
    else {
      buf.appendIndent(indent).append("try (");

      if (resources.size() > 1) {
        buf.appendLineSeparator();
        buf.append(ExprProcessor.listToJava(resources, indent + 1));
        buf.appendIndent(indent);
      }
      else {
        buf.append(resources.get(0).toJava(indent + 1));
      }
      buf.append(") {").appendLineSeparator();
    }

    appendReachabilityMarkers(buf, reachabilityMarkerTypes, indent + 1);
    buf.append(ExprProcessor.jmpWrapper(first, indent + 1, true));
    buf.appendIndent(indent).append("}");

    for (RenderedCatchClause clause : renderedCatchClauses) {
      appendCatchClause(buf, clause, indent);
    }
    buf.appendLineSeparator();

    return buf;
  }

  private static void addUniqueReachabilityMarkers(List<String> reachabilityMarkerTypes, List<String> markerTypes) {
    for (String markerType : markerTypes) {
      if (!reachabilityMarkerTypes.contains(markerType)) {
        reachabilityMarkerTypes.add(markerType);
      }
    }
  }

  private static void appendReachabilityMarkers(TextBuffer buf, List<String> markerTypes, int indent) {
    for (String markerType : markerTypes) {
      // Keep the original checked catch type source-reachable without changing real
      // execution: Thread.currentThread() is specified to return the current thread.
      if (DecompilerContext.getOption(IFernflowerPreferences.DECOMPILER_COMMENTS)) {
        buf.appendIndent(indent)
          .append("// $VF: synthetic checked-catch reachability marker for ")
          .append(ExprProcessor.getCastTypeName(new VarType(CodeType.OBJECT, 0, markerType)))
          .appendLineSeparator();
      }
      buf.appendIndent(indent).append("if (java.lang.Thread.currentThread() == null) {").appendLineSeparator();
      buf.appendIndent(indent + 1)
        .append("throw (")
        .append(ExprProcessor.getCastTypeName(new VarType(CodeType.OBJECT, 0, markerType)))
        .append(")null;")
        .appendLineSeparator();
      buf.appendIndent(indent).append("}").appendLineSeparator();
    }
  }

  private static Integer getCatchBytecodeOffset(Statement stat) {
    BasicBlock block = stat.getBasichead().getBlock();
    if (!block.getSeq().isEmpty() && block.getInstruction(0).opcode == CodeConstants.opc_astore) {
      Integer offset = block.getOldOffset(0);
      if (offset > -1) {
        return offset;
      }
    }
    return null;
  }

  private static void appendCatchClause(
    TextBuffer buf,
    RenderedCatchClause clause,
    int indent
  ) {
    if (clause.bytecodeOffset != null) {
      buf.addBytecodeMapping(clause.bytecodeOffset);
    }

    buf.append(" catch (");

    for (int exc_index = 0; exc_index < clause.renderedTypes.size(); ++exc_index) {
      String name = ExprProcessor.getCastTypeName(new VarType(CodeType.OBJECT, 0, clause.renderedTypes.get(exc_index)));
      if (clause.renderedTypes.size() > 1 && exc_index > 0) {
        buf.append(" | ");
      }
      buf.append(name);
    }

    if (clause.renderRewriteComment && clause.rewrite.isRewritten()) {
      buf.append(" /* $VF: ");
      buf.append(clause.rewrite.isFallbackUsed() ? "substituted checked catch types: " : "removed shadowed catch types: ");
      appendTypeListComment(buf, clause.rewrite.getRemovedCheckedTypes());
      buf.append(" */");
    }

    buf.append(" ");

    // Temporarily set variable as not a definition, since we just wrote the type above
    try (var v = clause.var.new DefinitionLocker()) {
      buf.append(clause.var.toJava(indent));
    }

    buf.append(") {").appendLineSeparator();
    buf.append(ExprProcessor.jmpWrapper(clause.stat, indent + 1, false)).appendIndent(indent)
      .append("}");
  }

  private static final class RenderedCatchClause {
    private final Statement stat;
    private final VarExprent var;
    private final List<String> renderedTypes;
    private final CheckedExceptionAnalyzer.CatchRewrite rewrite;
    private final boolean renderRewriteComment;
    private final Integer bytecodeOffset;

    private RenderedCatchClause(
      Statement stat,
      VarExprent var,
      List<String> renderedTypes,
      CheckedExceptionAnalyzer.CatchRewrite rewrite,
      boolean renderRewriteComment,
      Integer bytecodeOffset
    ) {
      this.stat = stat;
      this.var = var;
      this.renderedTypes = renderedTypes;
      this.rewrite = rewrite;
      this.renderRewriteComment = renderRewriteComment;
      this.bytecodeOffset = bytecodeOffset;
    }
  }

  private static boolean shouldSplitLegacyMultiCatch() {
    return DecompilerContext.shouldUseLegacySourceCompatibility(BytecodeVersion.MAJOR_7);
  }

  private void validateType(List<String> exTypes, VarType exVarType) {
    // TODO: join together all types, then check if exVarType instanceof that
    // Not correct!!
    if (ValidationHelper.VALIDATE) {
//      VarType type = new VarType(CodeType.OBJECT, 0, exTypes.get(exTypes.size() - 1));
//      ValidationHelper.validateTrue(type.higherEqualInLatticeThan(exVarType), "Invalid exception type " + exVarType + " " + type);
    }
  }

  private static void appendTypeListComment(TextBuffer buf, List<String> types) {
    for (int i = 0; i < types.size(); i++) {
      if (i > 0) {
        buf.append(" | ");
      }
      buf.append(ExprProcessor.getCastTypeName(new VarType(CodeType.OBJECT, 0, types.get(i))));
    }
  }

  private static List<String> collectFollowingCatchTypes(List<List<String>> catchTypes, int currentCatchIndex) {
    List<String> followingCatchTypes = new ArrayList<>();
    for (int i = currentCatchIndex; i < catchTypes.size(); i++) {
      followingCatchTypes.addAll(catchTypes.get(i));
    }
    return followingCatchTypes;
  }

  @Override
  public List<Exprent> getStatExprents() {
    List<Exprent> lst = new ArrayList<>(resources);
    lst.addAll(vars);
    return lst;
  }

  @Override
  public Statement getSimpleCopy() {
    CatchStatement cs = new CatchStatement();

    for (List<String> exc : this.exctstrings) {
      cs.exctstrings.add(new ArrayList<>(exc));
      cs.vars.add(new VarExprent(DecompilerContext.getCounterContainer().getCounterAndIncrement(CounterContainer.VAR_COUNTER),
                                 new VarType(CodeType.OBJECT, 0, exc.get(0)),
                                 DecompilerContext.getVarProcessor()));
    }

    return cs;
  }

  public void getOffset(BitSet values) {
    super.getOffset(values);

    for (Exprent exp : this.getResources()) {
      exp.getBytecodeRange(values);
    }
  }

  // *****************************************************************************
  // getter and setter methods
  // *****************************************************************************

  public List<List<String>> getExctStrings() {
    return exctstrings;
  }

  public List<VarExprent> getVars() {
    return vars;
  }

  public List<Exprent> getResources() {
    return resources;
  }

  @Override
  public List<VarExprent> getImplicitlyDefinedVars() {
    List<VarExprent> vars = new ArrayList<>(getVars());

    // resource vars must also be included
    for (Exprent exp : getResources()) {
      if (exp instanceof AssignmentExprent assignment) {
        vars.add((VarExprent) assignment.getLeft());
      }
    }

    return vars;
  }
}
