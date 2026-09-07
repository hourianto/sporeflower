package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.struct.gen.TypeFamily;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/** A merge-phase index; merges only rename variables and remove standalone declarations. */
final class VariableOccurrences {
  private final Map<VarVersionPair, List<Occurrence>> variables = new HashMap<>();
  private final Map<VarVersionPair, List<ConstExprent>> constants = new HashMap<>();

  VariableOccurrences(Statement root, Consumer<Exprent> collectFacts) {
    collect(root, collectFacts);
  }

  private void collect(Statement statement, Consumer<Exprent> collectFacts) {
    for (Exprent expression : statement.getVarDefinitions()) {
      collect(expression, statement.getVarDefinitions(), collectFacts);
    }
    if (statement.getExprents() != null) {
      for (Exprent expression : statement.getExprents()) {
        collect(expression, statement.getExprents(), collectFacts);
      }
    } else {
      // Header lists have fixed cardinality: a bare variable here is not a
      // removable standalone declaration (for example an enhanced-for binding).
      for (Exprent expression : statement.getStatExprents()) collect(expression, null, collectFacts);
      for (Statement child : statement.getStats()) collect(child, collectFacts);
    }
  }

  private void collect(Exprent expression, List<Exprent> owner, Consumer<Exprent> collectFacts) {
    if (expression == null) return;
    for (Exprent nested : expression.getAllExprents(true, true)) {
      collectFacts.accept(nested);
      if (nested instanceof VarExprent variable) {
        variables.computeIfAbsent(variable.getVarVersionPair(), ignored -> new ArrayList<>())
          .add(new Occurrence(variable, nested == expression ? owner : null));
      } else if (nested instanceof AssignmentExprent assignment
        && assignment.getLeft() instanceof VarExprent variable && assignment.getRight() instanceof ConstExprent constant) {
        constants.computeIfAbsent(variable.getVarVersionPair(), ignored -> new ArrayList<>()).add(constant);
      }
    }
  }

  boolean merge(VarVersionPair from, VarVersionPair to, VarType type) {
    List<Occurrence> source = variables.remove(from);
    if (source == null) return false;
    List<Occurrence> target = variables.computeIfAbsent(to, ignored -> new ArrayList<>());
    for (Occurrence occurrence : source) {
      VarExprent variable = occurrence.variable;
      if (occurrence.owner != null) {
        occurrence.owner.removeIf(expression -> expression == variable);
      } else {
        variable.setIndex(to.var);
        variable.setVersion(to.version);
        variable.setDefinition(false);
        target.add(occurrence);
      }
    }
    for (Occurrence occurrence : target) occurrence.variable.setVarType(type);

    List<ConstExprent> assigned = constants.computeIfAbsent(to, ignored -> new ArrayList<>());
    List<ConstExprent> moved = constants.remove(from);
    if (moved != null) assigned.addAll(moved);
    if (type.typeFamily != TypeFamily.OBJECT || type == VarType.VARTYPE_STRING || type == VarType.VARTYPE_CLASS) {
      for (ConstExprent constant : assigned) {
        if (!constant.isNull() && type.higherEqualInLatticeThan(constant.getConstType())) constant.setConstType(type);
      }
    }
    return true;
  }

  private record Occurrence(VarExprent variable, List<Exprent> owner) { }
}
