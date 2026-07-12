// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchAllStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarTypeProcessor.FinalType;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;

import org.jetbrains.java.decompiler.modules.decompiler.MethodExceptionSummary.ExceptionFlow;

/** Computes source-visible checked-exception flow over one structured method tree. */
final class CheckedExceptionFlowAnalyzer {
  private final CheckedInvocationResolver invocationResolver;
  private final StructClass ownerClass;
  private final ClassWrapper ownerWrapper;
  private final boolean preciseRethrow;
  private final boolean captureStatementFlows;
  private final boolean captureProtectedFlows;
  private final boolean captureExprentFlows;
  private final VarProcessor varProcessor;
  private final Map<Statement, ExceptionFlow> statementFlows = new IdentityHashMap<>();
  private final Map<CatchStatement, ExceptionFlow> protectedFlows = new IdentityHashMap<>();
  private final Map<Exprent, ExceptionFlow> exprentFlows = new IdentityHashMap<>();

  CheckedExceptionFlowAnalyzer(
    CheckedInvocationResolver invocationResolver,
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    boolean preciseRethrow,
    VarProcessor varProcessor,
    boolean captureStatementFlows,
    boolean captureProtectedFlows,
    boolean captureExprentFlows
  ) {
    this.invocationResolver = invocationResolver;
    this.ownerClass = ownerClass;
    this.ownerWrapper = ownerWrapper;
    this.preciseRethrow = preciseRethrow;
    this.varProcessor = varProcessor;
    this.captureStatementFlows = captureStatementFlows;
    this.captureProtectedFlows = captureProtectedFlows;
    this.captureExprentFlows = captureExprentFlows;
  }

  MethodFlow analyze(Statement statement) {
    ExceptionFlow methodFlow = analyzeStatement(statement, Collections.emptyMap());
    return new MethodFlow(methodFlow, statementFlows, protectedFlows, exprentFlows);
  }

  private ExceptionFlow analyzeStatement(
    Statement statement,
    Map<VarVersionPair, List<String>> preciseCatchVariables
  ) {
    if (statement == null) {
      return ExceptionFlow.EMPTY;
    }

    ExceptionFlow result;
    if (statement instanceof CatchStatement catchStatement) {
      result = analyzeCatch(catchStatement, preciseCatchVariables);
    }
    else if (statement instanceof CatchAllStatement catchAllStatement) {
      result = analyzeCatchAll(catchAllStatement, preciseCatchVariables);
    }
    else {
      result = analyzeExprents(statementExprents(statement), preciseCatchVariables);
      for (Statement child : new ArrayList<>(statement.getStats())) {
        result = result.union(analyzeStatement(child, preciseCatchVariables));
      }
    }

    if (captureStatementFlows) {
      statementFlows.put(statement, result);
    }
    return result;
  }

  private ExceptionFlow analyzeCatch(
    CatchStatement statement,
    Map<VarVersionPair, List<String>> outerPreciseCatchVariables
  ) {
    ExceptionFlow protectedFlow = analyzeExprents(statement.getResources(), outerPreciseCatchVariables)
      .union(analyzeStatement(statement.getFirst(), outerPreciseCatchVariables));
    if (captureProtectedFlows) {
      protectedFlows.put(statement, protectedFlow);
    }

    LinkedHashSet<String> remaining = new LinkedHashSet<>(protectedFlow.checkedExceptions());
    ExceptionFlow escapingHandlers = ExceptionFlow.EMPTY;
    boolean unknownEscapes = protectedFlow.unknownThrowability();
    List<Statement> children = statement.getStats();

    for (int i = 1; i < children.size(); i++) {
      List<String> catchTypes = statement.getExctStrings().get(i - 1);
      List<String> caught = new ArrayList<>();
      for (String exceptionType : remaining) {
        if (CheckedExceptionSupport.isCaughtByActiveCatches(exceptionType, catchTypes)) {
          caught.add(exceptionType);
        }
      }
      remaining.removeAll(caught);

      if (catchesUnknownCheckedException(catchTypes)) {
        unknownEscapes = false;
      }

      VarExprent catchVariable = statement.getVars().get(i - 1);
      Map<VarVersionPair, List<String>> handlerPreciseVariables = outerPreciseCatchVariables;
      if (preciseRethrow
        && varProcessor.getVarFinal(catchVariable.getVarVersionPair()) != FinalType.NON_FINAL) {
        handlerPreciseVariables = new HashMap<>(outerPreciseCatchVariables);
        handlerPreciseVariables.put(catchVariable.getVarVersionPair(), List.copyOf(caught));
      }
      escapingHandlers = escapingHandlers.union(analyzeStatement(children.get(i), handlerPreciseVariables));
    }

    return new ExceptionFlow(new ArrayList<>(remaining), unknownEscapes).union(escapingHandlers);
  }

  private ExceptionFlow analyzeCatchAll(
    CatchAllStatement statement,
    Map<VarVersionPair, List<String>> preciseCatchVariables
  ) {
    ExceptionFlow protectedFlow = analyzeStatement(statement.getFirst(), preciseCatchVariables);
    ExceptionFlow handlerFlow = analyzeStatement(statement.getHandler(), preciseCatchVariables);
    return statement.isFinally() ? protectedFlow.union(handlerFlow) : handlerFlow;
  }

  private ExceptionFlow analyzeExprents(
    List<Exprent> exprents,
    Map<VarVersionPair, List<String>> preciseCatchVariables
  ) {
    if (exprents == null || exprents.isEmpty()) {
      return ExceptionFlow.EMPTY;
    }

    ExceptionFlow result = ExceptionFlow.EMPTY;
    for (Exprent exprent : InterpreterUtil.snapshotNonNullList(exprents, "checked-exception flow exprents")) {
      result = result.union(analyzeExprentTree(exprent, preciseCatchVariables));
    }
    return result;
  }

  private ExceptionFlow analyzeExprentTree(
    Exprent exprent,
    Map<VarVersionPair, List<String>> preciseCatchVariables
  ) {
    ExceptionFlow result = ExceptionFlow.EMPTY;
    for (Exprent child : exprent.getAllExprents()) {
      result = result.union(analyzeExprentTree(child, preciseCatchVariables));
    }

    InvocationExprent invocation = null;
    if (exprent instanceof InvocationExprent direct) {
      invocation = direct;
    }
    else if (exprent instanceof NewExprent created && created.getConstructor() != null) {
      invocation = created.getConstructor();
    }
    if (invocation != null) {
      CheckedInvocationResolver.InvocationThrowInfo info =
        invocationResolver.resolveInvocationThrowInfo(invocation, ownerClass, ownerWrapper);
      result = result.union(new ExceptionFlow(info.checkedExceptions, info.unknownThrowability));
    }

    if (exprent instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.THROW) {
      result = result.union(explicitThrowFlow(exit.getValue(), preciseCatchVariables));
    }
    if (captureExprentFlows) {
      exprentFlows.put(exprent, result);
    }
    return result;
  }

  private static ExceptionFlow explicitThrowFlow(
    Exprent value,
    Map<VarVersionPair, List<String>> preciseCatchVariables
  ) {
    if (value instanceof VarExprent variable) {
      List<String> preciseTypes = preciseCatchVariables.get(variable.getVarVersionPair());
      if (preciseTypes != null) {
        return new ExceptionFlow(preciseTypes, false);
      }
    }
    if (value != null) {
      VarType thrownType = value.getExprType();
      if (thrownType.type == CodeType.OBJECT
        && thrownType.value != null
        && CheckedExceptionSupport.isCheckedExceptionType(thrownType.value)) {
        return new ExceptionFlow(List.of(thrownType.value), false);
      }
    }
    return ExceptionFlow.EMPTY;
  }

  private static List<Exprent> statementExprents(Statement statement) {
    return statement.getExprents() != null ? statement.getExprents() : statement.getStatExprents();
  }

  private static boolean catchesUnknownCheckedException(List<String> catchTypes) {
    return catchTypes.contains("java/lang/Exception") || catchTypes.contains("java/lang/Throwable");
  }

  record MethodFlow(
    ExceptionFlow methodFlow,
    Map<Statement, ExceptionFlow> statementFlows,
    Map<CatchStatement, ExceptionFlow> protectedFlows,
    Map<Exprent, ExceptionFlow> exprentFlows
  ) {
    MethodFlow {
      statementFlows = Collections.unmodifiableMap(new IdentityHashMap<>(statementFlows));
      protectedFlows = Collections.unmodifiableMap(new IdentityHashMap<>(protectedFlows));
      exprentFlows = Collections.unmodifiableMap(new IdentityHashMap<>(exprentFlows));
    }
  }

}
