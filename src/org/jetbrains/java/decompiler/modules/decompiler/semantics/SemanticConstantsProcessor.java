// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ArrayExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.SwitchHeadExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.SymbolicExpression;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Value;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.RecordLayout;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.CallBinding;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.IfStatement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;

public final class SemanticConstantsProcessor {
  private final SemanticMappings mappings;
  private final MemberKey method;
  private final String currentOwner;
  private final VarProcessor varProcessor;
  private final VarType returnType;
  private final Map<VarVersionPair, SemanticFacts> variableFacts = new HashMap<>();
  private final Map<VarExprent, SemanticFacts> variableOccurrenceFacts;
  private final SemanticContext context;
  private final Map<Integer, SemanticContext.Key> parameterKeys = new HashMap<>();
  private final List<Exprent> roots = new ArrayList<>();
  private final List<AssignmentExprent> variableAssignments = new ArrayList<>();
  private final Map<ConstExprent, ConstantContext> constantContexts = new IdentityHashMap<>();
  private final Map<ConstExprent, Set<ConstExprent.SemanticOffset>> offsetContexts = new IdentityHashMap<>();
  private final Set<Exprent> wrapGuards = Collections.newSetFromMap(new IdentityHashMap<>());
  private final Set<Exprent> intervalBounds = Collections.newSetFromMap(new IdentityHashMap<>());
  private final Set<ConstExprent> intBitwiseOperands = Collections.newSetFromMap(new IdentityHashMap<>());

  private record ConstantContext(Set<String> domains, VarType expectedType) {}

  private sealed interface DependentDomain {}
  private record ConditionalDomain(SemanticContext.Key selector, List<SemanticMappings.Condition> conditions) implements DependentDomain {}
  private record PackedCase(SemanticContext.Key selector, long value, String domain) {}
  private record PackedDomain(List<PackedCase> cases) implements DependentDomain {}

  private record SemanticFacts(Set<String> domains, Set<ArraySemantics> arrays,
                               Set<SemanticMappings.ContainerSemantics> containers, Set<DependentDomain> dependent, boolean unknown) {
    private static final SemanticFacts BOTTOM = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), false);
    private static final SemanticFacts UNKNOWN = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), true);

    private SemanticFacts {
      domains = Set.copyOf(domains); arrays = Set.copyOf(arrays);
      containers = Set.copyOf(containers); dependent = Set.copyOf(dependent);
    }

    private static SemanticFacts of(String domain, ArraySemantics array) {
      return declaration(domain, array, null);
    }

    private static SemanticFacts declaration(String domain, ArraySemantics array, SemanticMappings.ContainerSemantics container) {
      return new SemanticFacts(domain == null ? Set.of() : Set.of(domain), array == null ? Set.of() : Set.of(array),
        container == null ? Set.of() : Set.of(container), Set.of(), domain == null && array == null && container == null);
    }

    private static SemanticFacts conditional(SemanticContext.Key selector, List<SemanticMappings.Condition> conditions) {
      return selector == null ? UNKNOWN : new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(new ConditionalDomain(selector, conditions)), false);
    }

    private SemanticFacts merge(SemanticFacts other) {
      return new SemanticFacts(union(domains, other.domains), union(arrays, other.arrays),
        union(containers, other.containers), union(dependent, other.dependent), unknown || other.unknown);
    }

    private static <T> Set<T> union(Set<T> a, Set<T> b) {
      if (a.containsAll(b)) return a;
      if (b.containsAll(a)) return b;
      Set<T> result = new HashSet<>(a); result.addAll(b); return result;
    }
  }

  public static final class VariableSemanticsSnapshot {
    private final Map<VarExprent, SemanticFacts> facts;
    private final SemanticContext context;
    private final Map<Integer, SemanticContext.Key> parameters;

    private VariableSemanticsSnapshot(Map<VarExprent, SemanticFacts> facts, SemanticContext context, Map<Integer, SemanticContext.Key> parameters) {
      this.facts = facts; this.context = context; this.parameters = Map.copyOf(parameters);
    }
  }

  private SemanticConstantsProcessor(SemanticMappings mappings, StructClass owner, StructMethod method, VarProcessor varProcessor) {
    this(mappings, owner, method, varProcessor, null);
  }

  private SemanticConstantsProcessor(SemanticMappings mappings, StructClass owner, StructMethod method,
                                     VarProcessor varProcessor, VariableSemanticsSnapshot snapshot) {
    this.mappings = mappings;
    this.method = mappings.namedMember(new MemberKey(owner.qualifiedName, method.getName(), method.getDescriptor()));
    this.currentOwner = mappings.namedOwner(owner.qualifiedName);
    this.varProcessor = varProcessor;
    this.returnType = MethodDescriptor.parseDescriptor(this.method.desc()).ret;
    this.variableOccurrenceFacts = snapshot == null ? null : snapshot.facts;
    this.context = snapshot == null ? new SemanticContext() : snapshot.context;
    if (snapshot != null) parameterKeys.putAll(snapshot.parameters);
  }

  public static void process(Statement root, StructClass owner, StructMethod method, VarProcessor varProcessor,
                             SemanticMappings mappings) {
    SemanticConstantsProcessor processor = new SemanticConstantsProcessor(mappings, owner, method, varProcessor);
    processor.collectRoots(root);
    processor.context.analyze(root);
    processor.collectVariableAssignments();
    processor.seedParameterDomains(method);
    processor.propagateVariableDomains();
    for (Exprent exprent : processor.roots) processor.decorate(exprent);
    processor.renderConstants();
  }

  public static VariableSemanticsSnapshot analyzeVariableSemanticsBeforeMerging(
    Statement root, StructClass owner, StructMethod method, VarProcessor varProcessor, SemanticMappings mappings
  ) {
    SemanticConstantsProcessor processor = new SemanticConstantsProcessor(mappings, owner, method, varProcessor);
    processor.collectRoots(root);
    processor.context.analyze(root);
    processor.collectVariableAssignments();
    processor.seedParameterDomainsBySlot(method);
    processor.propagateVariableDomains();
    return processor.captureVariableSemantics();
  }

  public static void process(Statement root, StructClass owner, StructMethod method, VarProcessor varProcessor,
                             SemanticMappings mappings, VariableSemanticsSnapshot snapshot) {
    SemanticConstantsProcessor processor = new SemanticConstantsProcessor(mappings, owner, method, varProcessor, snapshot);
    processor.collectRoots(root);
    for (Exprent exprent : processor.roots) processor.decorate(exprent);
    processor.renderConstants();
  }

  private void collectRoots(Statement statement) {
    List<Exprent> exprents = statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents();
    roots.addAll(exprents);
    if (statement instanceof IfStatement conditional
        && conditional.getHeadexprent().getCondition() instanceof FunctionExprent comparison && isComparison(comparison)) {
      if (isWrapGuard(comparison, conditional.getIfstat(), false) || isWrapGuard(comparison, conditional.getElsestat(), true)) {
        wrapGuards.add(comparison);
      }
    }
    for (Statement child : statement.getStats()) collectRoots(child);
  }

  private static boolean isWrapGuard(FunctionExprent comparison, Statement branch, boolean negated) {
    if (branch == null) return false;
    while (branch.getExprents() == null && branch.getFirst() != null) branch = branch.getFirst();
    if (branch.getExprents() == null || branch.getExprents().isEmpty()
        || !(branch.getExprents().get(0) instanceof AssignmentExprent update)) return false;
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
    if (bound == null || !value.equals(update.getLeft())) return false;
    if (negated) {
      type = type == FunctionExprent.FunctionType.LT ? FunctionExprent.FunctionType.GE
        : type == FunctionExprent.FunctionType.GE ? FunctionExprent.FunctionType.LT : null;
    }
    Exprent delta = update.getRight();
    FunctionExprent.FunctionType operation = update.getCondType();
    if (operation == null && delta instanceof FunctionExprent function && function.getLstOperands().size() == 2
        && value.equals(function.getLstOperands().get(0))) {
      operation = function.getFuncType();
      delta = function.getLstOperands().get(1);
    }
    Long amount = literal(delta);
    if (amount == null) return false;
    // Recognize x < 0 followed by x += period, and x >= period followed by
    // x -= period. A sentinel such as RANDOM=-1 can otherwise hide the domain's
    // numeric lower edge. Equality and ordinary ordered thresholds stay eligible.
    return type == FunctionExprent.FunctionType.LT && bound == 0 && operation == FunctionExprent.FunctionType.ADD && amount > 0
      || type == FunctionExprent.FunctionType.GE && bound > 0 && operation == FunctionExprent.FunctionType.SUB && amount.equals(bound);
  }

  private void seedParameterDomains(StructMethod structMethod) {
    Map<Integer, SemanticFacts> slotFacts = parameterSlotFacts(structMethod);
    for (VarVersionPair parameter : varProcessor.getParams()) {
      Integer original = varProcessor.getVarOriginalIndex(parameter.var);
      SemanticFacts facts = slotFacts.get(original == null ? parameter.var : original);
      if (facts != null) mergeVariableFacts(parameter, facts);
    }
  }

  private void seedParameterDomainsBySlot(StructMethod structMethod) {
    for (Map.Entry<Integer, SemanticFacts> entry : parameterSlotFacts(structMethod).entrySet()) {
      // Before VarDefinitionHelper runs, the first SSA definition of a JVM
      // parameter has already been mapped back to its raw slot with version 0.
      mergeVariableFacts(new VarVersionPair(entry.getKey(), 0), entry.getValue());
    }
  }

  private Map<Integer, SemanticFacts> parameterSlotFacts(StructMethod structMethod) {
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(structMethod.getDescriptor());
    int slot = structMethod.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    Map<Integer, SemanticFacts> slotFacts = new HashMap<>();
    int parameterSlot = slot;
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      int index = parameterSlot;
      for (VarVersionPair pair : varProcessor.getParams()) {
        Integer original = varProcessor.getVarOriginalIndex(pair.var);
        if (original != null && original == parameterSlot) { index = pair.var; break; }
      }
      parameterKeys.put(parameter, SemanticContext.variable(index, 0));
      context.markParameter(parameterKeys.get(parameter));
      parameterSlot += descriptor.params[parameter].stackSize;
    }
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      SemanticFacts facts = SemanticFacts.declaration(
        mappings.parameterDomain(method, parameter),
        mappings.parameterArraySemantics(method, parameter),
        mappings.container(method, "parameter", parameter)
      );
      List<SemanticMappings.Condition> conditions = mappings.conditions(method, parameter);
      if (!conditions.isEmpty()) facts = SemanticFacts.conditional(parameterKeys.get(conditions.get(0).parameter()), conditions);
      slotFacts.put(slot, facts);
      slot += descriptor.params[parameter].stackSize;
    }
    return slotFacts;
  }

  private void propagateVariableDomains() {
    boolean changed;
    do {
      changed = false;
      for (AssignmentExprent assignment : variableAssignments) {
        VarExprent variable = (VarExprent)assignment.getLeft();
        changed |= mergeVariableFacts(variable.getVarVersionPair(), resolveFacts(assignment, factsOf(assignment), false));
      }
    } while (changed);
  }

  private boolean mergeVariableFacts(VarVersionPair variable, SemanticFacts added) {
    SemanticFacts current = variableFacts.getOrDefault(variable, SemanticFacts.BOTTOM);
    SemanticFacts merged = current.merge(added);
    if (merged.equals(current)) return false;
    variableFacts.put(variable, merged);
    return true;
  }

  private void decorate(Exprent exprent) {
    if (exprent instanceof FunctionExprent function
        && (function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_AND || function.getFuncType() == FunctionExprent.FunctionType.BOOLEAN_OR)) {
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
        applyDomain(assignment.getRight(), domainOf(assignment.getLeft()), assignment.getLeft().getExprType());
        applyArrayInitializerSemantics(assignment.getRight(), unique(arraySemanticsOf(assignment.getLeft())));
      }
      else if (isBitwise(assignment.getCondType())) {
        markIntBitwiseOperand(assignment.getRight(), assignment.getCompoundOperationType() == null
          ? assignment.getLeft().getExprType() : assignment.getCompoundOperationType());
        applyDomain(assignment.getRight(), flagDomainOf(List.of(assignment.getLeft(), assignment.getRight())),
          assignment.getCompoundOperationType() == null ? assignment.getLeft().getExprType() : assignment.getCompoundOperationType());
      }
      decorate(assignment.getRight());
      return;
    }
    if (exprent instanceof InvocationExprent invocation) {
      if (invocation.getInstance() != null) decorate(invocation.getInstance());
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
      String domain = domainOf(switchHead.getValue());
      decorate(switchHead.getValue());
      for (List<Exprent> cases : switchHead.getCaseValues()) {
        for (Exprent caseValue : cases) {
          if (caseValue != null) {
            applyDomain(caseValue, domain, switchHead.getValue().getExprType());
            decorate(caseValue);
          }
        }
      }
      return;
    }
    if (exprent instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.RETURN && exit.getValue() != null) {
      applyDomain(exit.getValue(), mappings.returnDomain(method), returnType);
      List<SemanticMappings.Condition> conditions = mappings.conditions(method, -1);
      if (!conditions.isEmpty()) applyConditionalDomain(exit.getValue(), parameterKeys.get(conditions.get(0).parameter()), conditions, returnType);
      applyArrayInitializerSemantics(exit.getValue(), mappings.returnArraySemantics(method));
      decorate(exit.getValue());
      return;
    }
    if (exprent instanceof ArrayExprent array) {
      decorate(array.getArray());
      applyDomain(array.getIndex(), arrayIndexDomain(array.getArray()), VarType.VARTYPE_INT);
      decorateRecordIndex(array);
      decorate(array.getIndex());
      return;
    }
    if (exprent instanceof FunctionExprent function && isComparison(function) && function.getLstOperands().size() == 2
        && !wrapGuards.contains(function)) {
      Exprent left = function.getLstOperands().get(0);
      Exprent right = function.getLstOperands().get(1);
      applyComparisonDomain(left, right, function.getFuncType(), true, intervalBounds.contains(function));
      applyComparisonDomain(right, left, function.getFuncType(), false, intervalBounds.contains(function));
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      for (Exprent operand : function.getLstOperands()) markIntBitwiseOperand(operand, function.getExprType());
      String domain = flagDomainOf(function);
      if (domain != null) {
        for (Exprent operand : function.getLstOperands()) applyDomain(operand, domain, function.getExprType());
      }
    }
    for (Exprent child : exprent.getAllExprents()) decorate(child);
  }

  private void markIntBitwiseOperand(Exprent operand, VarType operationType) {
    if (operationType.equals(VarType.VARTYPE_INT) && operand instanceof ConstExprent constant) intBitwiseOperands.add(constant);
  }

  private void decorateInvocationParameters(InvocationExprent invocation) {
    MemberKey invoked = invocationKey(invocation);
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(invocation.getStringDescriptor());
    for (int i = 0; i < invocation.getLstParameters().size(); i++) {
      Exprent parameter = invocation.getLstParameters().get(i);
      applyDomain(parameter, mappings.parameterDomain(invoked, i), descriptor.params[i]);
      SemanticMappings.SlotSource source = mappings.slotSource(invoked, i);
      if (source != null) {
        SemanticFacts facts = slotSourceFacts(invocation, source);
        if (!facts.unknown()) applyDomain(parameter, unique(facts.domains()), descriptor.params[i]);
      }
      List<SemanticMappings.Condition> conditions = mappings.conditions(invoked, i);
      if (!conditions.isEmpty()) {
        int selector = conditions.get(0).parameter();
        if (selector < invocation.getLstParameters().size()) {
          Long value = context.value(invocation.getLstParameters().get(selector));
          if (value != null) applyDomain(parameter, selectDomain(value, conditions), descriptor.params[i]);
          else applyConditionalDomain(parameter, context.key(invocation.getLstParameters().get(selector)), conditions, descriptor.params[i]);
        }
      }
      applyArrayInitializerSemantics(parameter, mappings.parameterArraySemantics(invoked, i));
      decorate(parameter);
    }
    decorateContainerCall(invocation);
    if ("java/lang/String".equals(invocation.getClassname()) && invocation.getInstance() != null
        && invocation.getLstParameters().size() == 1 && Set.of("equals", "equalsIgnoreCase", "compareTo").contains(invocation.getName())) {
      Exprent argument = invocation.getLstParameters().get(0);
      String receiver = domainOf(invocation.getInstance());
      String other = domainOf(argument);
      if ("string".equals(mappings.domainKind(receiver))) applyDomain(argument, receiver, VarType.VARTYPE_STRING);
      if ("string".equals(mappings.domainKind(other))) applyDomain(invocation.getInstance(), other, VarType.VARTYPE_STRING);
    }
  }

  private SemanticFacts resolveFacts(Exprent at, SemanticFacts facts, boolean requireKnown) {
    if (facts.dependent().isEmpty()) return facts;
    Set<String> domains = new HashSet<>(facts.domains());
    Set<DependentDomain> pending = new HashSet<>();
    boolean unknown = facts.unknown();
    for (DependentDomain dependent : facts.dependent()) {
      if (dependent instanceof ConditionalDomain conditional) {
        Long value = context.known(at, conditional.selector());
        String domain = selectDomain(at, conditional.selector(), conditional.conditions());
        if (domain != null) domains.add(domain);
        else if (value == null && !requireKnown) pending.add(dependent);
        else unknown = true;
      } else if (dependent instanceof PackedDomain packed) {
        Set<String> matching = new HashSet<>();
        boolean unresolved = false;
        for (PackedCase field : packed.cases()) {
          Long value = context.known(at, field.selector());
          if (value != null && value == field.value()) matching.add(field.domain());
          else if (value == null && !context.excludes(at, field.selector(), field.value())) unresolved = true;
        }
        // Validated overlapping fields have mutually exclusive selectors. A
        // proven match therefore rules out all other alternatives for the slice.
        if (!matching.isEmpty()) domains.addAll(matching);
        else if (unresolved && !requireKnown) pending.add(dependent);
        else unknown = true;
      }
    }
    return new SemanticFacts(domains, facts.arrays(), facts.containers(), pending, unknown);
  }

  private static String selectDomain(Long value, List<SemanticMappings.Condition> conditions) {
    if (value == null) return null;
    Set<String> domains = new HashSet<>();
    String fallback = null;
    for (SemanticMappings.Condition condition : conditions) {
      if (condition.otherwise()) fallback = condition.domain();
      else if (value.equals(condition.equalsValue()) || condition.notEqualsValue() != null && !value.equals(condition.notEqualsValue())) {
        domains.add(condition.domain());
      }
    }
    return domains.isEmpty() ? fallback : unique(domains);
  }

  private String selectDomain(Exprent at, SemanticContext.Key selector, List<SemanticMappings.Condition> conditions) {
    Long value = context.known(at, selector);
    if (value != null) return selectDomain(value, conditions);
    for (SemanticMappings.Condition condition : conditions) {
      if (condition.notEqualsValue() != null && context.excludes(at, selector, condition.notEqualsValue())) return condition.domain();
      if (condition.otherwise() && conditions.stream().filter(c -> c.equalsValue() != null)
          .allMatch(c -> context.excludes(at, selector, c.equalsValue()))) return condition.domain();
    }
    return null;
  }

  private void applyConditionalDomain(Exprent expression, SemanticContext.Key selector,
                                      List<SemanticMappings.Condition> conditions, VarType type) {
    if (expression instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      applyConditionalDomain(function.getLstOperands().get(1), selector, conditions, type);
      applyConditionalDomain(function.getLstOperands().get(2), selector, conditions, type);
      return;
    }
    applyDomain(expression, selectDomain(expression, selector, conditions), type);
  }

  private String domainOf(Exprent exprent) {
    SemanticFacts facts = resolveFacts(exprent, factsOf(exprent), true);
    return facts.unknown() ? null : unique(facts.domains());
  }

  private SemanticFacts invocationFacts(InvocationExprent invocation) {
    MemberKey invoked = invocationKey(invocation);
    if (invocation.getExprType().arrayDim == 0 && !invocation.getExprType().equals(VarType.VARTYPE_VOID)) {
      Set<String> scoped = new HashSet<>();
      for (CallBinding binding : mappings.callBindings(method)) {
        if (invocation.bytecode != null && binding.offset() >= 0 && invocation.bytecode.get(binding.offset())
            && binding.callee().equals(mappings.namedMember(invoked)) && !childOwnsCallOffset(invocation, binding.offset())) {
          scoped.add(binding.domain());
        }
      }
      if (!scoped.isEmpty()) return new SemanticFacts(scoped, Set.of(), Set.of(), Set.of(), false);
    }
    String declaredDomain = mappings.returnDomain(invoked);
    ArraySemantics array = mappings.returnArraySemantics(invoked);
    SemanticMappings.ContainerSemantics container = mappings.container(invoked, "return", -1);
    if (declaredDomain != null || array != null || container != null) return SemanticFacts.declaration(declaredDomain, array, container);
    List<SemanticMappings.Condition> conditions = mappings.conditions(invoked, -1);
    if (!conditions.isEmpty() && conditions.get(0).parameter() < invocation.getLstParameters().size()) {
      Exprent selector = invocation.getLstParameters().get(conditions.get(0).parameter());
      Long value = context.value(selector);
      if (value != null) return SemanticFacts.of(selectDomain(value, conditions), null);
      return SemanticFacts.conditional(context.key(selector), conditions);
    }
    SemanticMappings.SlotSource tableSource = mappings.slotSource(invoked, -1);
    if (tableSource != null) return slotSourceFacts(invocation, tableSource);
    Exprent boxed = boxedArgument(invocation);
    if (boxed != null) return factsOf(boxed);
    if (isUnboxing(invocation)) {
      if (unboxingPreservesStorage(invocation)) return factsOf(invocation.getInstance());
      SemanticFacts source = resolveFacts(invocation.getInstance(), factsOf(invocation.getInstance()), true);
      String target = primitiveDescriptor(invocation.getExprType());
      if (source.domains().stream().allMatch(domain -> mappings.fitsIntegralType(domain, target))) return source;
      return SemanticFacts.UNKNOWN;
    }
    SemanticFacts contents = containerCallFacts(invocation);
    if (contents != null) return contents;

    Integer sourceParameter = mappings.returnDomainSource(invoked);
    if (sourceParameter == null || sourceParameter < 0 || sourceParameter >= invocation.getLstParameters().size()) return SemanticFacts.UNKNOWN;
    // The mapping explicitly promises that the result keeps the argument's
    // semantic meaning, including ambiguity between multiple possible domains.
    return factsOf(invocation.getLstParameters().get(sourceParameter));
  }

  private SemanticFacts slotSourceFacts(InvocationExprent invocation, SemanticMappings.SlotSource source) {
    if (source.parameter() < 0 || source.parameter() >= invocation.getLstParameters().size()) return SemanticFacts.UNKNOWN;
    SemanticFacts table = factsOf(invocation.getLstParameters().get(source.parameter()));
    if (table.unknown() || table.arrays().isEmpty()) return SemanticFacts.UNKNOWN;
    SemanticFacts result = SemanticFacts.BOTTOM;
    for (ArraySemantics array : table.arrays()) {
      RecordLayout layout = array.records().get(source.dimension());
      // The source names a column, not an absolute array index after a header.
      if (layout != null && !layout.planes() && source.slot() >= layout.stride()) return SemanticFacts.UNKNOWN;
      String slots = layout == null ? array.slotDomains().get(source.dimension()) : layout.domain();
      String domain = slotElementDomain(slots, source.slot());
      result = result.merge(SemanticFacts.of(domain == null ? array.elementDomain() : domain, null));
      if (domain == null) {
        // A leaf column overrides every row's default. Without one, an unknown
        // row can select any outer slot's meaning, not just the array default.
        for (int dimension = 0; dimension < source.dimension(); dimension++) {
          for (String rowDomain : mappings.slotElementDomains(array.slotDomains().get(dimension))) {
            result = result.merge(SemanticFacts.of(rowDomain, null));
          }
          RecordLayout outer = array.records().get(dimension);
          if (outer != null) for (String rowDomain : mappings.slotElementDomains(outer.domain())) {
            result = result.merge(SemanticFacts.of(rowDomain, null));
          }
        }
      }
    }
    return result;
  }

  private static boolean childOwnsCallOffset(Exprent expression, int offset) {
    for (Exprent child : expression.getAllExprents()) {
      if (child instanceof InvocationExprent && child.bytecode != null && child.bytecode.get(offset)
          || childOwnsCallOffset(child, offset)) return true;
    }
    return false;
  }

  private SemanticFacts factsOf(Exprent exprent) {
    if (exprent instanceof ConstExprent) return SemanticFacts.BOTTOM;
    if (exprent instanceof NewExprent array && array.getNewType().arrayDim > 0) {
      // A fresh zero-filled array or numeric initializer has no competing
      // contract, like a scalar literal. An initializer fed by dynamic values
      // must not silently adopt a different branch's array semantics.
      return array.getLstArrayElements().stream().allMatch(element -> factsOf(element).equals(SemanticFacts.BOTTOM))
        ? SemanticFacts.BOTTOM : SemanticFacts.UNKNOWN;
    }
    if (exprent instanceof FieldExprent field) {
      MemberKey fieldMember = fieldKey(field);
      return SemanticFacts.declaration(mappings.fieldDomain(fieldMember), mappings.fieldArraySemantics(fieldMember), mappings.container(fieldMember, "field", -1));
    }
    if (exprent instanceof InvocationExprent invocation) {
      return invocationFacts(invocation);
    }
    if (exprent instanceof NewExprent creation && creation.getConstructor() != null) {
      Exprent boxed = boxedArgument(creation.getConstructor());
      if (boxed != null) return factsOf(boxed);
    }
    if (exprent instanceof VarExprent variable) {
      return variableOccurrenceFacts == null
        ? variableFacts.getOrDefault(variable.getVarVersionPair(), SemanticFacts.UNKNOWN)
        : variableOccurrenceFacts.getOrDefault(variable, SemanticFacts.UNKNOWN);
    }
    if (exprent instanceof FunctionExprent) {
      SemanticFacts extracted = extractionFacts(exprent);
      if (extracted != null) return extracted;
    }
    if (exprent instanceof FunctionExprent function && (isValuePreservingCast(function) || isIntegralCast(function))) {
      SemanticFacts source = factsOf(function.getLstOperands().get(0));
      if (isValuePreservingCast(function) || source.equals(SemanticFacts.BOTTOM)) return source;
      // Preserve the declared numeric values (or flag bits) across an integral
      // narrowing cast. Never turn a not-yet-evaluated definition into UNKNOWN:
      // that would make the fixed point depend on assignment traversal order.
      return !source.unknown() && !source.domains().isEmpty()
        && source.domains().stream().allMatch(domain -> mappings.fitsIntegralType(domain, primitiveDescriptor(function.getExprType())))
        ? source : SemanticFacts.UNKNOWN;
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      return factsOf(function.getLstOperands().get(1)).merge(factsOf(function.getLstOperands().get(2)));
    }
    if (exprent instanceof AssignmentExprent assignment) {
      if (assignment.getCondType() == null) return factsOf(assignment.getRight());
      // A compound assignment evaluates to the updated LHS, not to its delta.
      return isBitwise(assignment.getCondType())
        ? bitwiseFacts(List.of(assignment.getLeft(), assignment.getRight())) : SemanticFacts.UNKNOWN;
    }
    if (exprent instanceof ArrayExprent array) {
      return arrayElementFacts(array);
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      return bitwiseFacts(function.getLstOperands());
    }
    return SemanticFacts.UNKNOWN;
  }

  private static final Map<String, String> BOXES = Map.of(
    "java/lang/Byte", "B", "java/lang/Short", "S", "java/lang/Character", "C", "java/lang/Integer", "I", "java/lang/Long", "J");

  private static Exprent boxedArgument(InvocationExprent invocation) {
    String storage = BOXES.get(invocation.getClassname());
    if (storage == null || invocation.getLstParameters().size() != 1
        || !Set.of("<init>", "valueOf").contains(invocation.getName())) return null;
    // String parsing overloads do not preserve a numeric domain.
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(invocation.getStringDescriptor());
    return storage.equals(primitiveDescriptor(descriptor.params[0])) ? invocation.getLstParameters().get(0) : null;
  }

  private static boolean isUnboxing(InvocationExprent invocation) {
    return invocation.getInstance() != null && BOXES.containsKey(invocation.getClassname()) && invocation.getLstParameters().isEmpty()
      && Set.of("byteValue", "shortValue", "charValue", "intValue", "longValue").contains(invocation.getName())
      && primitiveDescriptor(invocation.getExprType()) != null;
  }

  private static boolean unboxingPreservesStorage(InvocationExprent invocation) {
    String storage = BOXES.get(invocation.getClassname());
    String target = primitiveDescriptor(invocation.getExprType());
    return storage.equals(target) || "J".equals(target) || "I".equals(target) && !"J".equals(storage)
      || "S".equals(target) && "B".equals(storage);
  }

  private SemanticFacts containerCallFacts(InvocationExprent invocation) {
    if (invocation.getInstance() == null || !Set.of("java/util/Vector", "java/util/Hashtable", "java/util/Enumeration").contains(invocation.getClassname())) return null;
    SemanticFacts source = factsOf(invocation.getInstance());
    if (source.containers().isEmpty()) return null;
    SemanticFacts result = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), source.unknown());
    String owner = invocation.getClassname(), name = invocation.getName();
    boolean enumeration = name.equals("elements") || name.equals("keys");
    boolean read = owner.equals("java/util/Vector") && Set.of("elementAt", "firstElement", "lastElement").contains(name)
      || owner.equals("java/util/Hashtable") && Set.of("get", "put", "remove").contains(name)
      || owner.equals("java/util/Enumeration") && name.equals("nextElement");
    if (!enumeration && !read) return null;
    for (SemanticMappings.ContainerSemantics container : source.containers()) {
      String domain = owner.equals("java/util/Hashtable") ? name.equals("keys") ? container.keys() : container.values() : container.elements();
      result = result.merge(enumeration && domain != null
        ? SemanticFacts.declaration(null, null, new SemanticMappings.ContainerSemantics(domain, null, null)) : SemanticFacts.of(domain, null));
    }
    return result;
  }

  private void decorateContainerCall(InvocationExprent invocation) {
    if (invocation.getInstance() == null) return;
    SemanticFacts source = factsOf(invocation.getInstance());
    if (source.unknown()) return;
    List<Exprent> arguments = invocation.getLstParameters();
    String owner = invocation.getClassname(), name = invocation.getName();
    for (SemanticMappings.ContainerSemantics container : source.containers()) {
      if (owner.equals("java/util/Vector")) {
        if (!arguments.isEmpty() && Set.of("addElement", "insertElementAt", "setElementAt", "contains", "indexOf", "lastIndexOf", "removeElement").contains(name)) {
          applyDomain(arguments.get(0), container.elements(), arguments.get(0).getExprType());
        }
        if (name.equals("copyInto") && arguments.size() == 1) applyArrayInitializerSemantics(arguments.get(0),
          new ArraySemantics(Map.of(), Map.of(), container.elements(), Map.of()));
      } else if (owner.equals("java/util/Hashtable") && !arguments.isEmpty()) {
        if (Set.of("get", "put", "remove", "containsKey").contains(name)) applyDomain(arguments.get(0), container.keys(), arguments.get(0).getExprType());
        if (name.equals("put") && arguments.size() == 2) applyDomain(arguments.get(1), container.values(), arguments.get(1).getExprType());
        if (name.equals("contains")) applyDomain(arguments.get(0), container.values(), arguments.get(0).getExprType());
      }
    }
  }

  private SemanticFacts extractionFacts(Exprent expression) {
    SemanticBitAccess.Extraction extraction = SemanticBitAccess.extraction(expression);
    if (extraction == null) return null;
    SemanticFacts source = resolveFacts(extraction.source(), factsOf(extraction.source()), true);
    if (source.domains().stream().allMatch(domain -> mappings.bitFields(domain).isEmpty())) return null;
    SemanticFacts result = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), source.unknown());
    boolean matchesShape = false;
    for (String domain : source.domains()) {
      List<PackedCase> cases = new ArrayList<>();
      for (var field : mappings.bitFields(domain)) {
        if (field.shift() != extraction.shift() || field.bits() != extraction.bits() || field.signed() != extraction.signed()) continue;
        matchesShape = true;
        SemanticContext.Key selector = SemanticContext.constant(0);
        if (field.selectorMask() != 0) {
          selector = SemanticContext.operation(FunctionExprent.FunctionType.AND,
            context.key(extraction.source()), SemanticContext.constant(field.selectorMask()));
        }
        cases.add(new PackedCase(selector, field.selectorValue(), field.domain()));
      }
      result = result.merge(cases.isEmpty() ? SemanticFacts.UNKNOWN
        : new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(new PackedDomain(List.copyOf(cases))), false));
    }
    // Masks that do not select a declared field may simply preserve or clear
    // bits in the original word (e.g. unsigned-byte normalization with &255).
    // Keep the packed source's immutable definition key through copies. A
    // later guard can select the field, but a guard on a rewritten source cannot.
    return matchesShape ? resolveFacts(expression, result, false) : null;
  }

  private void decoratePacking(Exprent expression, String domain) {
    if (expression instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.OR) {
      for (Exprent operand : function.getLstOperands()) decoratePacking(operand, domain);
      return;
    }
    for (var field : mappings.bitFields(domain)) {
      if (field.selectorMask() != 0) continue;
      Exprent value = SemanticBitAccess.packingValue(expression, field.shift(), field.bits());
      if (value != null) applyDomain(value, field.domain(), value.getExprType());
    }
  }

  private SemanticFacts arrayElementFacts(ArrayExprent array) {
    Long slot = literal(array.getIndex());
    SemanticFacts source = factsOf(array.getArray());
    SemanticFacts result = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), source.unknown());
    for (ArraySemantics semantics : source.arrays()) {
      RecordLayout layout = semantics.records().get(0);
      Integer recordSlot = layout == null ? null : SemanticRecordAccess.slot(array.getIndex(), layout, context);
      String override = recordSlot != null ? slotElementDomain(layout.domain(), recordSlot)
        : slot == null ? null : slotElementDomain(semantics, slot);
      result = result.merge(elementFacts(semantics, override, array.getExprType()));
      if (slot == null && recordSlot == null) {
        // A dynamic index can select either the default element domain or any
        // slot-specific override. Choosing only the default would hide ambiguity.
        for (String domain : mappings.slotElementDomains(semantics.slotDomains().get(0))) {
          result = result.merge(elementFacts(semantics, domain, array.getExprType()));
        }
        if (layout != null) {
          for (String domain : mappings.slotElementDomains(layout.domain())) {
            result = result.merge(elementFacts(semantics, domain, array.getExprType()));
          }
        }
      }
    }
    return result;
  }

  private void decorateRecordIndex(ArrayExprent array) {
    ArraySemantics semantics = unique(arraySemanticsOf(array.getArray()));
    RecordLayout layout = semantics == null ? null : semantics.records().get(0);
    if (layout == null) return;
    if (layout.planes() && array.getIndex() instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.ADD) {
      for (Exprent operand : function.getLstOperands()) {
        if (!(operand instanceof ConstExprent constant)) continue;
        Long number = literal(constant);
        if (number == null || number < layout.offset() || (number - layout.offset()) % layout.stride() != 0) continue;
        Value value = mappings.value(layout.domain(), (number - layout.offset()) / layout.stride(), currentOwner);
        if (value != null) offsetContexts.computeIfAbsent(constant, ignored -> new HashSet<>()).add(new ConstExprent.SemanticOffset(
          new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), value.value()), layout.stride(), layout.offset()));
      }
    }
    Integer slot = SemanticRecordAccess.slot(array.getIndex(), layout, context);
    if (slot == null) return;
    ConstExprent offset = SemanticRecordAccess.offsetLiteral(array.getIndex(), layout, slot);
    if (offset != null) applyDomain(offset, layout.domain(), VarType.VARTYPE_INT);
  }

  private static SemanticFacts elementFacts(ArraySemantics semantics, String override, VarType type) {
    if (type.arrayDim == 0) return SemanticFacts.of(override == null ? semantics.elementDomain() : override, null);
    ArraySemantics element = semantics.element();
    if (override != null) element = element.withElementDomain(override);
    return SemanticFacts.of(null, element.isEmpty() ? null : element);
  }

  private String arrayIndexDomain(Exprent array) {
    SemanticFacts facts = factsOf(array);
    if (facts.unknown()) return null;
    Set<String> domains = new HashSet<>();
    for (ArraySemantics semantics : facts.arrays()) {
      String domain = semantics.indexDomains().get(0);
      if (domain == null) domain = semantics.slotDomains().get(0);
      if (domain == null) return null;
      domains.add(domain);
    }
    // Index meaning can agree even when possible rows have different leaf domains.
    return unique(domains);
  }

  private void applyDomain(Exprent exprent, String domain, VarType expectedType) {
    if (domain == null) return;
    if (exprent instanceof NewExprent creation && creation.getConstructor() != null) {
      Exprent boxed = boxedArgument(creation.getConstructor());
      if (boxed != null) { applyDomain(boxed, domain, boxed.getExprType()); return; }
    }
    if (exprent instanceof InvocationExprent invocation) {
      Exprent boxed = boxedArgument(invocation);
      if (boxed != null) { applyDomain(boxed, domain, boxed.getExprType()); return; }
    }
    if ("packed".equals(mappings.domainKind(domain))) decoratePacking(exprent, domain);
    if (exprent instanceof FunctionExprent function && (isValuePreservingCast(function)
        || isIntegralCast(function) && mappings.fitsIntegralType(domain, primitiveDescriptor(function.getExprType())))) {
      Exprent operand = function.getLstOperands().get(0);
      applyDomain(operand, domain, operand.getExprType());
      return;
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      applyDomain(function.getLstOperands().get(1), domain, expectedType);
      applyDomain(function.getLstOperands().get(2), domain, expectedType);
      return;
    }
    if (exprent instanceof FunctionExprent function
        && isBitwise(function)
        && "flags".equals(mappings.domainKind(domain))) {
      // A consumer binding describes the entire mask expression, even when no
      // operand already carries the domain. Propagate it into every bitwise term.
      for (Exprent operand : function.getLstOperands()) {
        applyDomain(operand, domain, function.getExprType());
      }
      return;
    }
    if (exprent instanceof AssignmentExprent assignment) {
      if (assignment.getCondType() == null || isBitwise(assignment.getCondType()) && "flags".equals(mappings.domainKind(domain))) {
        applyDomain(assignment.getRight(), domain, expectedType);
      }
      return;
    }
    if (!(exprent instanceof ConstExprent constant)) return;
    Long literal = literal(constant);
    if (literal == null && !(constant.getExprType().equals(VarType.VARTYPE_STRING) && constant.getValue() instanceof String && "string".equals(mappings.domainKind(domain)))) return;
    // Comparisons promote narrow values to int (or long). The other operand
    // may lie outside the annotated value's storage range; narrowing that
    // literal while rendering would change the comparison's result.
    if (literal != null && !fitsType(literal, expectedType)) expectedType = constant.getExprType();
    ConstantContext context = constantContexts.computeIfAbsent(constant, ignored -> new ConstantContext(new HashSet<>(), null));
    context.domains().add(domain);
    constantContexts.put(constant, new ConstantContext(context.domains(), expectedType));
  }

  private void renderConstants() {
    // Consumer and producer contexts can disagree. Collect both before rendering
    // so traversal order never decides which conflicting name wins.
    constantContexts.forEach((constant, context) -> {
      String domain = unique(context.domains());
      if (domain == null) return;
      if (constant.getValue() instanceof String text) {
        Value value = mappings.stringValue(domain, text, currentOwner);
        if (value != null) constant.setSymbolicExpression(new ConstExprent.SymbolicExpression(
          List.of(new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), 0)), null, false, false, "Ljava/lang/String;"));
        return;
      }
      String formatted = mappings.formattedLiteral(domain, literal(constant), constant.getValue() instanceof Long);
      if (formatted != null) { constant.setSemanticLiteral(formatted); return; }
      SymbolicExpression expression = mappings.symbolicExpression(domain, literal(constant), currentOwner, bitWidth(context.expectedType()));
      if (expression == null) return;
      String target = primitiveDescriptor(context.expectedType());
      if (intBitwiseOperands.contains(constant) && literal(constant) < 0 && expression.residual() != null) {
        // A byte/short cast can express sign extension without a numeric high-bit
        // residual. Only do this inside int bitwise operations: Java promotes the
        // cast back to int there, so overload selection and long widths cannot change.
        for (VarType narrow : List.of(VarType.VARTYPE_BYTE, VarType.VARTYPE_SHORT)) {
          if (!fitsType(literal(constant), narrow)) continue;
          SymbolicExpression candidate = mappings.symbolicExpression(domain, literal(constant), currentOwner, bitWidth(narrow));
          if (candidate != null && candidate.residual() == null && !candidate.complemented()
              && candidate.values().size() <= expression.values().size()) {
            expression = candidate;
            target = primitiveDescriptor(narrow);
            break;
          }
        }
      }
      constant.setSymbolicExpression(new ConstExprent.SymbolicExpression(expression.values().stream()
        .map(value -> new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), value.value()))
        .toList(), expression.residual(), expression.complemented(), expression.longLiteral(), target));
    });
    offsetContexts.forEach((constant, offsets) -> {
      ConstExprent.SemanticOffset offset = unique(offsets);
      if (offset != null && !constantContexts.containsKey(constant)) constant.setSemanticOffset(offset);
    });
  }

  private void applyArrayInitializerSemantics(Exprent exprent, ArraySemantics semantics) {
    if (semantics == null) return;
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      applyArrayInitializerSemantics(function.getLstOperands().get(1), semantics);
      applyArrayInitializerSemantics(function.getLstOperands().get(2), semantics);
      return;
    }
    if (exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
      applyArrayInitializerSemantics(assignment.getRight(), semantics);
      return;
    }
    if (!(exprent instanceof NewExprent array) || array.getLstArrayElements().isEmpty()) return;

    VarType elementType = array.getNewType().decreaseArrayDim();
    ArraySemantics nestedSemantics = semantics.element();
    for (int index = 0; index < array.getLstArrayElements().size(); index++) {
      Exprent element = array.getLstArrayElements().get(index);
      String slotDomain = slotElementDomain(semantics, index);
      if (elementType.arrayDim > 0) {
        applyArrayInitializerSemantics(element, slotDomain == null ? nestedSemantics : nestedSemantics.withElementDomain(slotDomain));
      }
      else {
        String domain = semantics.elementDomain();
        if (slotDomain != null) domain = slotDomain;
        applyDomain(element, domain, elementType);
      }
    }
  }

  private static String primitiveDescriptor(VarType type) {
    if (type == null || type.arrayDim != 0) return null;
    return switch (type.type) {
      case BYTE -> "B";
      case SHORT -> "S";
      case CHAR -> "C";
      case INT -> "I";
      case LONG -> "J";
      default -> null;
    };
  }

  private static int bitWidth(VarType type) {
    if (type == null) return 0;
    return switch (type.type) {
      case BYTE -> 8;
      case SHORT, CHAR -> 16;
      case INT -> 32;
      case LONG -> 64;
      default -> 0;
    };
  }

  private static boolean isComparison(FunctionExprent function) {
    return switch (function.getFuncType()) {
      case EQ, NE, LT, LE, GT, GE -> true;
      default -> false;
    };
  }

  private void applyComparisonDomain(Exprent literal, Exprent value, FunctionExprent.FunctionType comparison,
                                     boolean literalOnLeft, boolean intervalBound) {
    String domain = domainOf(value);
    Long number = literal(literal);
    if (domain == null) return;
    if (number != null && comparison != FunctionExprent.FunctionType.EQ && comparison != FunctionExprent.FunctionType.NE) {
      boolean lowerBound = literalOnLeft
        ? comparison == FunctionExprent.FunctionType.GT || comparison == FunctionExprent.FunctionType.LE
        : comparison == FunctionExprent.FunctionType.LT || comparison == FunctionExprent.FunctionType.GE;
      // A standalone sign test on a nonnegative domain checks validity. Other
      // thresholds and explicit intervals can legitimately name domain values.
      if (!intervalBound && lowerBound && number == 0 && mappings.isRangeBoundary(domain, number, true)) return;
    }
    applyDomain(literal, domain, value.getExprType());
  }

  private static Exprent orderedComparisonValue(Exprent exprent) {
    if (!(exprent instanceof FunctionExprent function) || !isComparison(function)
        || function.getFuncType() == FunctionExprent.FunctionType.EQ || function.getFuncType() == FunctionExprent.FunctionType.NE) return null;
    if (literal(function.getLstOperands().get(0)) != null) return function.getLstOperands().get(1);
    return literal(function.getLstOperands().get(1)) != null ? function.getLstOperands().get(0) : null;
  }

  private static boolean isIntegralCast(FunctionExprent function) {
    return switch (function.getFuncType()) {
      case I2L, I2B, I2S, I2C, L2I -> true;
      default -> false;
    };
  }

  private static boolean isValuePreservingCast(FunctionExprent function) {
    if (function.getFuncType() == FunctionExprent.FunctionType.CAST) return true;
    if (function.getFuncType() == FunctionExprent.FunctionType.I2L) return true;
    if (function.getFuncType().castType == null) return false;
    Long value = literal(function.getLstOperands().get(0));
    if (value == null) return false;
    // A narrowing cast of a variable can change the represented value. Only
    // pass an integral domain through when this particular operand is unchanged.
    return switch (function.getFuncType()) {
      case I2B -> value == (byte)value.longValue();
      case I2S -> value == (short)value.longValue();
      case I2C -> value == (char)value.longValue();
      case L2I -> value == (int)value.longValue();
      default -> false;
    };
  }

  private static boolean fitsType(long value, VarType type) {
    if (type == null || type.arrayDim != 0) return false;
    return switch (type.type) {
      case BYTE -> value == (byte)value;
      case SHORT -> value == (short)value;
      case CHAR -> value == (char)value;
      case INT -> value == (int)value;
      case LONG -> true;
      default -> false;
    };
  }

  private static boolean isBitwise(FunctionExprent function) {
    return isBitwise(function.getFuncType());
  }

  private static boolean isBitwise(FunctionExprent.FunctionType type) {
    return switch (type) {
      case AND, OR, XOR, BIT_NOT -> true;
      default -> false;
    };
  }

  private String flagDomainOf(FunctionExprent function) {
    return flagDomainOf(function.getLstOperands());
  }

  private String flagDomainOf(List<Exprent> operands) {
    SemanticFacts facts = bitwiseFacts(operands);
    String domain = facts.unknown() ? null : unique(facts.domains());
    return "flags".equals(mappings.domainKind(domain)) ? domain : null;
  }

  private SemanticFacts bitwiseFacts(List<Exprent> operands) {
    SemanticFacts facts = SemanticFacts.BOTTOM;
    for (Exprent operand : operands) {
      // Numeric masks are neutral terms; unknown variables and conflicting
      // domains are not. Keep conflicts even inside a larger bitwise tree.
      if (literal(operand) == null) facts = facts.merge(resolveFacts(operand, factsOf(operand), true));
    }
    boolean unknown = facts.unknown() || facts.domains().stream().anyMatch(domain -> !Set.of("flags", "packed").contains(mappings.domainKind(domain)));
    return new SemanticFacts(facts.domains(), Set.of(), Set.of(), Set.of(), unknown);
  }

  private static MemberKey fieldKey(FieldExprent field) {
    return new MemberKey(field.getClassname(), field.getName(), field.getDescriptor().descriptorString);
  }

  private static MemberKey invocationKey(InvocationExprent invocation) {
    return new MemberKey(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor());
  }

  private Set<ArraySemantics> arraySemanticsOf(Exprent exprent) {
    SemanticFacts facts = factsOf(exprent);
    return facts.unknown() ? Set.of() : facts.arrays();
  }

  private static <T> T unique(Set<T> candidates) {
    return candidates.size() == 1 ? candidates.iterator().next() : null;
  }

  private String slotElementDomain(ArraySemantics semantics, long slot) {
    RecordLayout layout = semantics.records().get(0);
    if (layout != null) {
      Integer recordSlot = SemanticRecordAccess.slot(slot, layout);
      if (recordSlot != null) return slotElementDomain(layout.domain(), recordSlot);
    }
    String slotDomain = semantics.slotDomains().get(0);
    return slotElementDomain(slotDomain, slot);
  }

  private String slotElementDomain(String slotDomain, long slot) {
    if (slotDomain == null) return null;
    Value value = mappings.value(slotDomain, slot, currentOwner);
    return value == null ? null : value.elementDomain();
  }

  private static Long literal(Exprent exprent) {
    if (!(exprent instanceof ConstExprent constant) || !(constant.getValue() instanceof Number number)
        || number instanceof Float || number instanceof Double) return null;
    return number.longValue();
  }

  private void collectVariableAssignments() {
    for (Exprent root : roots) {
      walk(root, exprent -> {
        if (exprent instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent) {
          variableAssignments.add(assignment);
          variableFacts.putIfAbsent(((VarExprent)assignment.getLeft()).getVarVersionPair(), SemanticFacts.BOTTOM);
        }
      });
    }
  }

  private VariableSemanticsSnapshot captureVariableSemantics() {
    Map<VarExprent, SemanticFacts> factsByOccurrence = new IdentityHashMap<>();
    for (Exprent root : roots) {
      walk(root, exprent -> {
        if (exprent instanceof VarExprent variable) {
          SemanticFacts facts = variableFacts.get(variable.getVarVersionPair());
          if (facts != null) factsByOccurrence.put(variable, facts);
        }
      });
    }

    // VarDefinitionHelper may merge unrelated SSA-derived indices into one
    // printable Java local. Keeping facts on the surviving expression objects
    // preserves their definition-scoped meaning across that renumbering.
    return new VariableSemanticsSnapshot(factsByOccurrence, context, parameterKeys);
  }

  private static void walk(Exprent exprent, Consumer<Exprent> consumer) {
    consumer.accept(exprent);
    if (exprent instanceof SwitchHeadExprent switchHead) {
      for (List<Exprent> cases : switchHead.getCaseValues()) {
        for (Exprent caseValue : cases) if (caseValue != null) walk(caseValue, consumer);
      }
    }
    for (Exprent child : exprent.getAllExprents()) walk(child, consumer);
  }

}
