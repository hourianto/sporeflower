package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdge;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectEdgeType;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectGraph;
import org.jetbrains.java.decompiler.modules.decompiler.flow.DirectNode;
import org.jetbrains.java.decompiler.modules.decompiler.stats.DummyExitStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statements;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructField;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.util.ArrayDeque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.function.Predicate;

/** Conservative source definite-assignment check for a blank final instance field. */
public final class FinalFieldAssignmentAnalyzer {
  // Bits describe the possible assignment counts on incoming paths. Joins take
  // their union; a second write or an unassigned normal exit rejects final.
  // This finite, monotone state also accounts for writes repeated by a loop.
  private static final int UNASSIGNED = 1;
  private static final int ASSIGNED = 2;
  private static final int INVALID = 4;

  private final ClassWrapper wrapper;
  private final Map<String, Constructor> constructors = new HashMap<>();
  private final Map<StructField, Boolean> fieldResults = new HashMap<>();
  private boolean complete = true;

  /** Snapshot constructor relationships after initializer extraction and structural repairs. */
  public FinalFieldAssignmentAnalyzer(ClassWrapper wrapper) {
    this.wrapper = wrapper;
    for (MethodWrapper method : wrapper.getMethods()) {
      if (!CodeConstants.INIT_NAME.equals(method.methodStruct.getName()) || !method.methodStruct.containsCode()) continue;
      if (method.root == null) {
        complete = false;
        continue;
      }
      String delegated = null;
      Statement first = Statements.findFirstData(method.root);
      if (first != null && first.getExprents() != null && !first.getExprents().isEmpty()
        && first.getExprents().get(0) instanceof InvocationExprent call
        && Statements.isInvocationInitConstructor(call, method, wrapper, true)
        && wrapper.getClassStruct().qualifiedName.equals(call.getClassname())) {
        delegated = call.getStringDescriptor();
      }
      constructors.put(method.methodStruct.getDescriptor(), new Constructor(method, delegated));
    }
  }

  public boolean isAssignedOnce(StructField field) {
    return fieldResults.computeIfAbsent(field, ignored -> {
      if (!complete || constructors.isEmpty()) return false;
      Map<String, Boolean> results = new HashMap<>();
      for (String descriptor : constructors.keySet()) {
        if (!initializes(descriptor, field, results, new HashSet<>())) return false;
      }
      return true;
    });
  }

  private boolean initializes(String descriptor, StructField field, Map<String, Boolean> results, Set<String> visiting) {
    Boolean known = results.get(descriptor);
    if (known != null) return known;
    Constructor constructor = constructors.get(descriptor);
    if (constructor == null || !visiting.add(descriptor)) return false;
    boolean valid = constructor.delegated == null || initializes(constructor.delegated, field, results, visiting);
    if (valid) {
      boolean initialized = constructor.delegated != null || wrapper.getDynamicFieldInitializers()
        .containsKey(InterpreterUtil.makeUniqueKey(field.getName(), field.getDescriptor()));
      valid = isAssignedOnce(constructor.method.getOrBuildGraph(), initialized, access ->
        !access.isStatic() && wrapper.getClassStruct().qualifiedName.equals(access.getClassname())
          && field.getName().equals(access.getName()) && field.getDescriptor().equals(access.getDescriptor().descriptorString)
          && access.getInstance() instanceof VarExprent receiver
          && constructor.method.varproc.isReceiverEquivalent(new VarVersionPair(receiver)));
    }
    visiting.remove(descriptor);
    results.put(descriptor, valid);
    return valid;
  }

  private record Constructor(MethodWrapper method, String delegated) { }

  public static boolean isAssignedOnce(DirectGraph graph, boolean delegatedAssignment, Predicate<FieldExprent> target) {
    if (graph == null) return false;
    Map<DirectNode, Integer> inputs = new HashMap<>();
    ArrayDeque<DirectNode> work = new ArrayDeque<>();
    inputs.put(graph.first, delegatedAssignment ? ASSIGNED : UNASSIGNED);
    work.add(graph.first);
    while (!work.isEmpty()) {
      DirectNode node = work.removeFirst();
      int state = inputs.get(node);
      int exceptional = state;
      if (node.statement instanceof DummyExitStatement && state != ASSIGNED) return false;
      for (Exprent expression : node.exprents) {
        state = transfer(expression, state, target);
        exceptional |= state;
        if ((state & INVALID) != 0) return false;
        if (expression instanceof ExitExprent exit) {
          // A throwing path need not assign a blank final. Checking an explicit
          // return before its enclosing finally is conservative: unsupported
          // cleanup shapes lose final rather than being certified incorrectly.
          if (exit.getExitType() == ExitExprent.Type.RETURN && state != ASSIGNED) return false;
          state = 0;
          break;
        }
      }
      for (DirectEdgeType type : DirectEdgeType.TYPES) {
        // A handler may be entered before or after a write in this block.
        int outgoing = type == DirectEdgeType.EXCEPTION ? exceptional : state;
        if (outgoing == 0) continue;
        for (DirectEdge edge : node.getSuccessors(type)) {
          DirectNode destination = edge.getDestination();
          int old = inputs.getOrDefault(destination, 0);
          int merged = old | outgoing;
          if (old != merged) {
            inputs.put(destination, merged);
            work.add(destination);
          }
        }
      }
    }
    return true;
  }

  private static int transfer(Exprent expression, int state, Predicate<FieldExprent> target) {
    if (expression instanceof AssignmentExprent assignment) {
      if (assignment.getLeft() instanceof FieldExprent field && target.test(field)) {
        state = transfer(assignment.getRight(), state, target);
        if (assignment.getCondType() != null) return INVALID;
        return ((state & UNASSIGNED) != 0 ? ASSIGNED : 0) | ((state & (ASSIGNED | INVALID)) != 0 ? INVALID : 0);
      }
    }
    if (expression instanceof FieldExprent field && target.test(field)) {
      // Reading a blank final before assignment also fails Java's source rules.
      return (state & UNASSIGNED) != 0 ? INVALID : state;
    }
    if (expression instanceof FunctionExprent function) {
      var operands = function.getLstOperands();
      switch (function.getFuncType()) {
        case TERNARY: {
          int condition = transfer(operands.get(0), state, target);
          return transfer(operands.get(1), condition, target) | transfer(operands.get(2), condition, target);
        }
        case BOOLEAN_AND, BOOLEAN_OR: {
          int left = transfer(operands.get(0), state, target);
          return left | transfer(operands.get(1), left, target);
        }
        case PPI, IPP, MMI, IMM: {
          if (operands.get(0) instanceof FieldExprent field && target.test(field)) return INVALID;
          break;
        }
        default: break;
      }
    }
    for (Exprent child : expression.getAllExprents()) state = transfer(child, state, target);
    return state;
  }
}
