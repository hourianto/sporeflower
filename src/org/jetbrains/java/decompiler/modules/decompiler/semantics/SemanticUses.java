// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ArrayExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.SwitchHeadExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ContainerSemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.RecordLayout;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Value;
import org.jetbrains.java.decompiler.modules.decompiler.stats.IfStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticFacts.*;

/** Identifies semantic uses and their Java storage context without changing expressions. */
final class SemanticUses {
  interface Sink {
    // Missing facets are no-ops, so callers can pass a normalized contract directly.
    void domain(Exprent expression, String domain, VarType type);
    void array(Exprent expression, ArraySemantics shape);
    default void container(Exprent expression, ContainerSemantics shape) {}
    default void bitwise(Exprent expression, VarType type) {}
    default void offset(ConstExprent expression, ConstExprent.SemanticOffset offset) {}
  }
  private final SemanticAnalysis analysis;
  private final SemanticMappings mappings;
  private final SemanticContext context;
  private final Sink sink;
  private final Set<Exprent> wrapGuards = Collections.newSetFromMap(new IdentityHashMap<>());
  private final Set<Exprent> intervalBounds = Collections.newSetFromMap(new IdentityHashMap<>());

  SemanticUses(Statement root, SemanticAnalysis analysis, Sink sink) {
    this.analysis = analysis;
    this.sink = sink;
    mappings = analysis.mappings;
    context = analysis.context;
    collectWrapGuards(root);
  }
  void visit() {
    for (Exprent root : analysis.roots) decorate(root);
  }
  private void applyContract(Exprent expression, SemanticContract contract, VarType type) {
    sink.domain(expression, contract.domain(), type);
    sink.array(expression, contract.array());
    sink.container(expression, contract.container());
  }

  private void collectWrapGuards(Statement statement) {
    if (statement instanceof IfStatement conditional && conditional.getHeadexprent().getCondition() instanceof FunctionExprent comparison
      && isComparison(comparison)) {
      if (isWrapGuard(comparison, conditional.getIfstat(), false) || isWrapGuard(comparison, conditional.getElsestat(), true)) {
        wrapGuards.add(comparison);
      }
    }
    for (Statement child : statement.getStats()) collectWrapGuards(child);
  }

  private static boolean isWrapGuard(FunctionExprent comparison, Statement branch, boolean negated) {
    if (branch == null)
      return false;
    while (branch.getExprents() == null && branch.getFirst() != null) branch = branch.getFirst();
    if (branch.getExprents() == null || branch.getExprents().isEmpty()
      || !(branch.getExprents().get(0) instanceof AssignmentExprent update))
      return false;
    Exprent value = comparison.getLstOperands().get(0);
    Long bound = literal(comparison.getLstOperands().get(1));
    FunctionExprent.FunctionType type = comparison.getFuncType();
    if (bound == null) {
      bound = literal(value);
      value = comparison.getLstOperands().get(1);
      type = switch (type) {
        case GT -> FunctionExprent.FunctionType.LT;
        case LE -> FunctionExprent.FunctionType.GE;
        default -> null;
      };
    }
    if (bound == null || !value.equals(update.getLeft()))
      return false;
    if (negated) {
      type = type == FunctionExprent.FunctionType.LT ? FunctionExprent.FunctionType.GE
        : type == FunctionExprent.FunctionType.GE    ? FunctionExprent.FunctionType.LT
                                                     : null;
    }
    Exprent delta = update.getRight();
    FunctionExprent.FunctionType operation = update.getCondType();
    if (operation == null && delta instanceof FunctionExprent function && function.getLstOperands().size() == 2
      && value.equals(function.getLstOperands().get(0))) {
      operation = function.getFuncType();
      delta = function.getLstOperands().get(1);
    }
    Long amount = literal(delta);
    if (amount == null)
      return false;
    // Recognize x < 0 followed by x += period, and x >= period followed by
    // x -= period. A sentinel such as RANDOM=-1 can otherwise hide the domain's
    // numeric lower edge. Equality and ordinary ordered thresholds stay eligible.
    return type == FunctionExprent.FunctionType.LT && bound == 0 && operation == FunctionExprent.FunctionType.ADD && amount > 0
      || type == FunctionExprent.FunctionType.GE && bound > 0 && operation == FunctionExprent.FunctionType.SUB && amount.equals(bound);
  }

  private void decorate(Exprent exprent) {
    if (exprent instanceof FunctionExprent function
      && (function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND
        || function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_OR)) {
      Exprent left = function.getLstOperands().get(0);
      Exprent right = function.getLstOperands().get(1);
      Exprent leftValue = orderedComparisonValue(left);
      if (leftValue != null && leftValue.equals(orderedComparisonValue(right))) {
        // Two bounds on the same value express an interval of domain values,
        // even when one endpoint happens to be zero or the domain's extreme.
        intervalBounds.add(left);
        intervalBounds.add(right);
      }
    }
    if (exprent instanceof AssignmentExprent assignment) {
      decorate(assignment.getLeft());
      if (assignment.getCondType() == null) {
        sink.domain(assignment.getRight(), analysis.domainOf(assignment.getLeft()), assignment.getLeft().getExprType());
        sink.array(assignment.getRight(), unique(analysis.arraySemanticsOf(assignment.getLeft())));
        SemanticFacts target = analysis.factsOf(assignment.getLeft());
        if (!target.unknown())
          sink.container(assignment.getRight(), unique(target.containers()));
      } else if (isBitwise(assignment.getCondType())) {
        VarType type = assignment.getCompoundOperationType() == null ? assignment.getLeft().getExprType() : assignment.getCompoundOperationType();
        sink.bitwise(assignment.getRight(), type);
        sink.domain(assignment.getRight(), analysis.flagDomainOf(List.of(assignment.getLeft(), assignment.getRight())), type);
      }
      decorate(assignment.getRight());
      return;
    }
    if (exprent instanceof InvocationExprent invocation) {
      if (invocation.getInstance() != null)
        decorate(invocation.getInstance());
      decorateInvocationParameters(invocation);
      return;
    }
    if (exprent instanceof NewExprent creation && creation.getConstructor() != null) {
      // NewExprent exposes constructor arguments as children but deliberately
      // hides the self-referencing constructor invocation from generic walks.
      decorateInvocationParameters(creation.getConstructor());
      return;
    }
    if (exprent instanceof SwitchHeadExprent switchHead) {
      String domain = analysis.domainOf(switchHead.getValue());
      decorate(switchHead.getValue());
      for (List<Exprent> cases : switchHead.getCaseValues()) {
        for (Exprent caseValue : cases) {
          if (caseValue != null) {
            sink.domain(caseValue, domain, switchHead.getValue().getExprType());
            decorate(caseValue);
          }
        }
      }
      return;
    }
    if (exprent instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.RETURN && exit.getValue() != null) {
      SemanticContract contract = mappings.contract(analysis.method, "return", -1);
      applyContract(exit.getValue(), contract, analysis.returnType);
      List<SemanticMappings.Condition> conditions = contract.conditions();
      if (!conditions.isEmpty())
        applyConditionalDomain(exit.getValue(), analysis.parameterKeys.get(conditions.get(0).parameter()), conditions, analysis.returnType);
      decorate(exit.getValue());
      return;
    }
    if (exprent instanceof ArrayExprent array) {
      decorate(array.getArray());
      sink.domain(array.getIndex(), analysis.arrayIndexDomain(array.getArray()), VarType.VARTYPE_INT);
      decorateRecordIndex(array);
      decorate(array.getIndex());
      return;
    }
    if (exprent instanceof FunctionExprent function && isComparison(function) && function.getLstOperands().size() == 2
      && !wrapGuards.contains(function)) {
      Exprent left = function.getLstOperands().get(0);
      Exprent right = function.getLstOperands().get(1);
      applyComparisonDomain(left, right, function.getFuncType(), intervalBounds.contains(function));
      applyComparisonDomain(right, left, function.getFuncType(), intervalBounds.contains(function));
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      for (Exprent operand : function.getLstOperands()) sink.bitwise(operand, function.getExprType());
      String domain = analysis.flagDomainOf(function);
      if (domain != null) {
        for (Exprent operand : function.getLstOperands()) sink.domain(operand, domain, function.getExprType());
      }
    }
    for (Exprent child : exprent.getAllExprents()) decorate(child);
  }

  private void decorateInvocationParameters(InvocationExprent invocation) {
    MemberKey invoked = invocationKey(invocation);
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(invocation.getStringDescriptor());
    for (int i = 0; i < invocation.getLstParameters().size(); i++) {
      Exprent parameter = invocation.getLstParameters().get(i);
      Set<String> scoped = analysis.scopedCallDomains(invocation, i);
      if (!scoped.isEmpty()) {
        sink.domain(parameter, unique(scoped), descriptor.params[i]);
        decorate(parameter);
        continue;
      }
      SemanticContract contract = mappings.contract(invoked, "parameter", i);
      applyContract(parameter, contract, descriptor.params[i]);
      SemanticMappings.SlotSource source = contract.column();
      if (source != null) {
        SemanticFacts facts = analysis.slotSourceFacts(invocation, source);
        if (!facts.unknown())
          sink.domain(parameter, unique(facts.domains()), descriptor.params[i]);
      }
      List<SemanticMappings.Condition> conditions = contract.conditions();
      if (!conditions.isEmpty()) {
        int selector = conditions.get(0).parameter();
        if (selector < invocation.getLstParameters().size()) {
          Long value = context.value(invocation.getLstParameters().get(selector));
          if (value != null)
            sink.domain(parameter, analysis.selectDomain(value, conditions), descriptor.params[i]);
          else
            applyConditionalDomain(parameter, context.key(invocation.getLstParameters().get(selector)), conditions, descriptor.params[i]);
        }
      }
      decorate(parameter);
    }
    decorateContainerCall(invocation);
    if ("java/lang/String".equals(invocation.getClassname()) && invocation.getInstance() != null
      && invocation.getLstParameters().size() == 1 && Set.of("equals", "equalsIgnoreCase", "compareTo").contains(invocation.getName())) {
      Exprent argument = invocation.getLstParameters().get(0);
      String receiver = analysis.domainOf(invocation.getInstance());
      String other = analysis.domainOf(argument);
      if ("string".equals(mappings.domainKind(receiver)))
        sink.domain(argument, receiver, VarType.VARTYPE_STRING);
      if ("string".equals(mappings.domainKind(other)))
        sink.domain(invocation.getInstance(), other, VarType.VARTYPE_STRING);
    }
  }

  private void applyConditionalDomain(
    Exprent expression, SemanticContext.Key selector, List<SemanticMappings.Condition> conditions, VarType type) {
    if (expression instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      applyConditionalDomain(function.getLstOperands().get(1), selector, conditions, type);
      applyConditionalDomain(function.getLstOperands().get(2), selector, conditions, type);
      return;
    }
    sink.domain(expression, analysis.selectDomain(expression, selector, conditions), type);
  }

  private void decorateContainerCall(InvocationExprent invocation) {
    if (invocation.getInstance() == null)
      return;
    SemanticFacts source = analysis.factsOf(invocation.getInstance());
    if (source.unknown())
      return;
    for (ContainerSemantics container : source.containers()) containerUse(invocation, container, sink);
  }

  /** Shared by ordinary call uses and backward requirements on local containers. */
  static void containerUse(InvocationExprent invocation, ContainerSemantics container, Sink sink) {
    List<Exprent> arguments = invocation.getLstParameters();
    String owner = invocation.getClassname(), name = invocation.getName();
    if (arguments.isEmpty())
      return;
    Exprent first = arguments.get(0);
    if (owner.equals("java/util/Vector")) {
      if (Set.of("addElement", "insertElementAt", "setElementAt", "contains", "indexOf", "lastIndexOf", "removeElement").contains(name))
        sink.domain(first, container.elements(), first.getExprType());
      if (name.equals("copyInto") && arguments.size() == 1 && container.elements() != null)
        sink.array(first, new ArraySemantics(Map.of(), Map.of(), container.elements(), Map.of()));
    } else if (owner.equals("java/util/Hashtable")) {
      if (Set.of("get", "put", "remove", "containsKey").contains(name))
        sink.domain(first, container.keys(), first.getExprType());
      if (name.equals("put") && arguments.size() == 2)
        sink.domain(arguments.get(1), container.values(), arguments.get(1).getExprType());
      if (name.equals("contains"))
        sink.domain(first, container.values(), first.getExprType());
    }
  }

  private void decorateRecordIndex(ArrayExprent array) {
    ArraySemantics semantics = unique(analysis.arraySemanticsOf(array.getArray()));
    RecordLayout layout = semantics == null ? null : semantics.records().get(0);
    if (layout == null)
      return;
    if (layout.planes() && array.getIndex() instanceof FunctionExprent function
      && function.getFuncType() == FunctionExprent.FunctionType.ADD) {
      for (Exprent operand : function.getLstOperands()) {
        if (!(operand instanceof ConstExprent constant))
          continue;
        Long number = literal(constant);
        if (number == null || number < layout.offset() || (number - layout.offset()) % layout.stride() != 0)
          continue;
        Value value = mappings.value(layout.domain(), (number - layout.offset()) / layout.stride(), analysis.currentOwner);
        if (value != null)
          sink.offset(constant,
            new ConstExprent.SemanticOffset(new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), value.value()),
              layout.stride(), layout.offset()));
      }
    }
    Integer slot = SemanticRecordAccess.slot(array.getIndex(), layout, context);
    if (slot == null)
      return;
    ConstExprent offset = SemanticRecordAccess.offsetLiteral(array.getIndex(), layout, slot);
    if (offset != null)
      sink.domain(offset, layout.domain(), VarType.VARTYPE_INT);
  }

  private void applyComparisonDomain(Exprent literal, Exprent value, FunctionExprent.FunctionType comparison, boolean intervalBound) {
    String domain = analysis.domainOf(value);
    Long number = literal(literal);
    if (domain == null)
      return;
    if (number != null && comparison != FunctionExprent.FunctionType.EQ && comparison != FunctionExprent.FunctionType.NE) {
      // A standalone zero test describes sign/validity, even if the named
      // domain contains negative sentinels. Explicit intervals retain endpoints.
      if (!intervalBound && number == 0)
        return;
    }
    sink.domain(literal, domain, value.getExprType());
  }
}
