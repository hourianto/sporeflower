// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchAllStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.util.ArrayList;
import java.util.List;

final class CheckedStatementWalker {
  enum CatchAllPolicy {
    CATCH_ALL_CATCHES_THROWABLE,
    CATCH_ALL_IGNORED
  }

  @FunctionalInterface
  interface ExprentVisitor {
    boolean visit(List<Exprent> exprents, List<String> activeCatchTypes);
  }

  private CheckedStatementWalker() {
  }

  static boolean walk(
    Statement statement,
    List<String> activeCatchTypes,
    CatchAllPolicy catchAllPolicy,
    ExprentVisitor exprentVisitor
  ) {
    // Single structural traversal shared across all checked-exception analyses.
    if (statement == null) {
      return false;
    }

    if (statement instanceof CatchStatement catchStatement) {
      List<String> tryCatchTypes = mergeCatchTypes(activeCatchTypes, catchStatement.getExctStrings());
      if (exprentVisitor.visit(snapshotExprents(catchStatement.getResources()), tryCatchTypes)) {
        return true;
      }
      if (walk(catchStatement.getFirst(), tryCatchTypes, catchAllPolicy, exprentVisitor)) {
        return true;
      }
      List<Statement> catchChildren = new ArrayList<>(catchStatement.getStats());
      for (int i = 1; i < catchChildren.size(); i++) {
        if (walk(catchChildren.get(i), activeCatchTypes, catchAllPolicy, exprentVisitor)) {
          return true;
        }
      }
      return false;
    }

    if (statement instanceof CatchAllStatement catchAllStatement) {
      List<String> tryCatchTypes = activeCatchTypes;
      if (catchAllPolicy == CatchAllPolicy.CATCH_ALL_CATCHES_THROWABLE && !catchAllStatement.isFinally()) {
        tryCatchTypes = new ArrayList<>(activeCatchTypes);
        tryCatchTypes.add("java/lang/Throwable");
      }

      if (walk(catchAllStatement.getFirst(), tryCatchTypes, catchAllPolicy, exprentVisitor)) {
        return true;
      }
      return walk(catchAllStatement.getHandler(), activeCatchTypes, catchAllPolicy, exprentVisitor);
    }

    List<Exprent> exprents = statement.getExprents() != null ? statement.getExprents() : statement.getStatExprents();
    exprents = snapshotExprents(exprents);
    if (exprentVisitor.visit(exprents, activeCatchTypes)) {
      return true;
    }

    // Re-entrant checked-exception inference can trigger statement tree rewrites while walking.
    // Iterate a stable snapshot to avoid fail-fast iterator crashes.
    for (Statement child : new ArrayList<>(statement.getStats())) {
      if (walk(child, activeCatchTypes, catchAllPolicy, exprentVisitor)) {
        return true;
      }
    }

    return false;
  }

  private static List<String> mergeCatchTypes(List<String> activeCatchTypes, List<List<String>> localCatchTypeGroups) {
    if (localCatchTypeGroups == null || localCatchTypeGroups.isEmpty()) {
      return activeCatchTypes;
    }

    List<String> merged = new ArrayList<>(activeCatchTypes);
    for (List<String> catchTypes : localCatchTypeGroups) {
      merged.addAll(catchTypes);
    }
    return merged;
  }

  private static List<Exprent> snapshotExprents(List<Exprent> exprents) {
    return InterpreterUtil.snapshotNonNullList(exprents, "statement exprents");
  }
}
