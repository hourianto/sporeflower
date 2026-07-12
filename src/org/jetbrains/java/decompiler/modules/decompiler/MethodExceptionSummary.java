// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;

import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;

/** Immutable source-level checked-exception facts for one fully structured method. */
public final class MethodExceptionSummary {
  public static final MethodExceptionSummary EMPTY = new MethodExceptionSummary(
    ExceptionFlow.EMPTY,
    List.of(),
    List.of(),
    Map.of(),
    Map.of(),
    Map.of()
  );

  private final ExceptionFlow escapingFlow;
  private final List<String> inferredThrows;
  private final List<String> wrappedExceptions;
  private final Map<Statement, ExceptionFlow> statementFlows;
  private final Map<CatchStatement, ExceptionFlow> protectedFlows;
  private final Map<Exprent, ExceptionFlow> exprentFlows;

  public MethodExceptionSummary(
    ExceptionFlow escapingFlow,
    List<String> inferredThrows,
    List<String> wrappedExceptions,
    Map<Statement, ExceptionFlow> statementFlows,
    Map<CatchStatement, ExceptionFlow> protectedFlows,
    Map<Exprent, ExceptionFlow> exprentFlows
  ) {
    this.escapingFlow = escapingFlow;
    this.inferredThrows = List.copyOf(inferredThrows);
    this.wrappedExceptions = List.copyOf(wrappedExceptions);
    this.statementFlows = immutableIdentityMap(statementFlows);
    this.protectedFlows = immutableIdentityMap(protectedFlows);
    this.exprentFlows = immutableIdentityMap(exprentFlows);
  }

  private static <K, V> Map<K, V> immutableIdentityMap(Map<K, V> source) {
    return Collections.unmodifiableMap(new IdentityHashMap<>(source));
  }

  public ExceptionFlow escapingFlow() {
    return escapingFlow;
  }

  public List<String> inferredThrows() {
    return inferredThrows;
  }

  public List<String> wrappedExceptions() {
    return wrappedExceptions;
  }

  public ExceptionFlow protectedFlowOf(CatchStatement statement) {
    return protectedFlows.getOrDefault(statement, ExceptionFlow.EMPTY);
  }

  public ExceptionFlow flowOf(Statement statement) {
    return statementFlows.getOrDefault(statement, ExceptionFlow.EMPTY);
  }

  public ExceptionFlow flowOf(Exprent exprent) {
    return exprentFlows.getOrDefault(exprent, ExceptionFlow.EMPTY);
  }

  public record ExceptionFlow(List<String> checkedExceptions, boolean unknownThrowability) {
    public static final ExceptionFlow EMPTY = new ExceptionFlow(List.of(), false);

    public ExceptionFlow {
      checkedExceptions = List.copyOf(new LinkedHashSet<>(checkedExceptions));
    }

    public boolean hasVisibleThrow(String exceptionType) {
      for (String thrown : checkedExceptions) {
        if (CheckedExceptionSupport.isSubtypeOf(thrown, exceptionType)) {
          return true;
        }
      }
      return false;
    }

    ExceptionFlow union(ExceptionFlow other) {
      if (this == EMPTY) {
        return other;
      }
      if (other == EMPTY) {
        return this;
      }
      LinkedHashSet<String> combined = new LinkedHashSet<>(checkedExceptions);
      combined.addAll(other.checkedExceptions);
      return new ExceptionFlow(List.copyOf(combined), unknownThrowability || other.unknownThrowability);
    }
  }
}
