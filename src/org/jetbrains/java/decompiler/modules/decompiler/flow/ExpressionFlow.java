package org.jetbrains.java.decompiler.modules.decompiler.flow;

import org.jetbrains.java.decompiler.modules.decompiler.exps.*;

import java.util.List;

/**
 * Backward transfer through Java expression evaluation, with separate demands
 * for true and false outcomes. Consumers supply their data-flow domain; reads,
 * writes, short circuiting and evaluation order have one implementation.
 *
 * visit may mutate its continuation. condition and join must leave their input
 * continuations untouched because another outcome can share the same state.
 */
public abstract class ExpressionFlow<S> {
  protected abstract S copy(S state);
  protected abstract S join(S first, S second);
  protected abstract S read(VarExprent variable, S state);
  protected abstract S write(VarExprent variable, Exprent expression, S state);
  protected S declare(VarExprent variable, S state) { return state; }
  protected boolean skip(Exprent expression) { return false; }

  public final S visit(Exprent expression, S state) {
    if (expression == null || skip(expression)) return state;
    if (expression instanceof VarExprent variable) {
      return variable.isDefinition() ? declare(variable, state) : read(variable, state);
    }
    if (expression instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable) {
      state = write(variable, expression, state);
      state = visit(assignment.getRight(), state);
      return assignment.getCondType() == null ? state : read(variable, state);
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      if (function.getFuncType().isPPMM() && operands.get(0) instanceof VarExprent variable) {
        return read(variable, write(variable, expression, state));
      }
      switch (function.getFuncType()) {
        case TERNARY, BOOLEAN_AND, BOOLEAN_OR, BOOL_NOT:
          return condition(function, state, state);
      }
    }
    List<Exprent> children = expression.getAllExprents();
    for (int i = children.size() - 1; i >= 0; i--) state = visit(children.get(i), state);
    return state;
  }

  public final S condition(Exprent expression, S whenTrue, S whenFalse) {
    if (expression == null || skip(expression)) return join(whenTrue, whenFalse);
    if (expression instanceof IfExprent conditional) {
      return condition(conditional.getCondition(), whenTrue, whenFalse);
    }
    if (expression instanceof FunctionExprent function) {
      List<Exprent> operands = function.getLstOperands();
      switch (function.getFuncType()) {
        case BOOLEAN_AND:
          return condition(operands.get(0), condition(operands.get(1), whenTrue, whenFalse), whenFalse);
        case BOOLEAN_OR:
          return condition(operands.get(0), whenTrue, condition(operands.get(1), whenTrue, whenFalse));
        case BOOL_NOT:
          return condition(operands.get(0), whenFalse, whenTrue);
        case TERNARY:
          return condition(operands.get(0), condition(operands.get(1), whenTrue, whenFalse),
            condition(operands.get(2), whenTrue, whenFalse));
      }
    }
    if (expression instanceof AssignmentExprent assignment && assignment.getCondType() == null
        && assignment.getLeft() instanceof VarExprent variable) {
      S positive = write(variable, expression, copy(whenTrue));
      S negative = write(variable, expression, copy(whenFalse));
      return condition(assignment.getRight(), positive, negative);
    }
    return visit(expression, join(whenTrue, whenFalse));
  }
}
