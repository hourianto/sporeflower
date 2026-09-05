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
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
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
  private final List<Exprent> roots = new ArrayList<>();
  private final List<AssignmentExprent> variableAssignments = new ArrayList<>();

  private record SemanticFacts(Set<String> domains, Set<ArraySemantics> arrays) {
    private static final SemanticFacts EMPTY = new SemanticFacts(Set.of(), Set.of());

    private SemanticFacts {
      domains = Set.copyOf(domains);
      arrays = Set.copyOf(arrays);
    }

    private static SemanticFacts of(String domain, ArraySemantics array) {
      return new SemanticFacts(domain == null ? Set.of() : Set.of(domain), array == null ? Set.of() : Set.of(array));
    }

    private SemanticFacts merge(SemanticFacts other) {
      if (domains.containsAll(other.domains) && arrays.containsAll(other.arrays)) return this;
      if (other.domains.containsAll(domains) && other.arrays.containsAll(arrays)) return other;
      Set<String> mergedDomains = new HashSet<>(domains);
      mergedDomains.addAll(other.domains);
      Set<ArraySemantics> mergedArrays = new HashSet<>(arrays);
      mergedArrays.addAll(other.arrays);
      return new SemanticFacts(mergedDomains, mergedArrays);
    }
  }

  public static final class VariableSemanticsSnapshot {
    private final Map<VarExprent, SemanticFacts> facts;

    private VariableSemanticsSnapshot(Map<VarExprent, SemanticFacts> facts) {
      this.facts = facts;
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
  }

  public static void process(Statement root, StructClass owner, StructMethod method, VarProcessor varProcessor,
                             SemanticMappings mappings) {
    SemanticConstantsProcessor processor = new SemanticConstantsProcessor(mappings, owner, method, varProcessor);
    processor.collectRoots(root);
    processor.collectVariableAssignments();
    processor.seedParameterDomains(method);
    processor.propagateVariableDomains();
    for (Exprent exprent : processor.roots) processor.decorate(exprent);
  }

  public static VariableSemanticsSnapshot analyzeVariableSemanticsBeforeMerging(
    Statement root, StructClass owner, StructMethod method, VarProcessor varProcessor, SemanticMappings mappings
  ) {
    SemanticConstantsProcessor processor = new SemanticConstantsProcessor(mappings, owner, method, varProcessor);
    processor.collectRoots(root);
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
  }

  private void collectRoots(Statement statement) {
    List<Exprent> exprents = statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents();
    roots.addAll(exprents);
    for (Statement child : statement.getStats()) collectRoots(child);
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
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      SemanticFacts facts = SemanticFacts.of(
        mappings.parameterDomain(method, parameter),
        mappings.parameterArraySemantics(method, parameter)
      );
      if (!facts.equals(SemanticFacts.EMPTY)) slotFacts.put(slot, facts);
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
        changed |= mergeVariableFacts(variable.getVarVersionPair(), factsOf(assignment.getRight()));
      }
    } while (changed);
  }

  private boolean mergeVariableFacts(VarVersionPair variable, SemanticFacts added) {
    SemanticFacts current = variableFacts.getOrDefault(variable, SemanticFacts.EMPTY);
    SemanticFacts merged = current.merge(added);
    if (merged.equals(current)) return false;
    variableFacts.put(variable, merged);
    return true;
  }

  private void decorate(Exprent exprent) {
    if (exprent instanceof AssignmentExprent assignment) {
      decorate(assignment.getLeft());
      applyDomain(assignment.getRight(), domainOf(assignment.getLeft()), assignment.getLeft().getExprType());
      applyArrayInitializerSemantics(
        assignment.getRight(),
        unique(arraySemanticsOf(assignment.getLeft()))
      );
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
      decorate(exit.getValue());
      return;
    }
    if (exprent instanceof ArrayExprent array) {
      ArraySemantics semantics = unique(arraySemanticsOf(array.getArray()));
      decorate(array.getArray());
      if (semantics != null) {
        String domain = semantics.indexDomains().get(0);
        if (domain == null) domain = semantics.slotDomains().get(0);
        applyDomain(array.getIndex(), domain, VarType.VARTYPE_INT);
      }
      decorate(array.getIndex());
      return;
    }
    if (exprent instanceof FunctionExprent function && isComparison(function) && function.getLstOperands().size() == 2) {
      Exprent left = function.getLstOperands().get(0);
      Exprent right = function.getLstOperands().get(1);
      applyDomain(left, domainOf(right), right.getExprType());
      applyDomain(right, domainOf(left), left.getExprType());
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      String domain = flagDomainOf(function);
      if (domain != null) {
        for (Exprent operand : function.getLstOperands()) applyDomain(operand, domain, function.getExprType());
      }
    }
    for (Exprent child : exprent.getAllExprents()) decorate(child);
  }

  private void decorateInvocationParameters(InvocationExprent invocation) {
    MemberKey invoked = invocationKey(invocation);
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(invocation.getStringDescriptor());
    for (int i = 0; i < invocation.getLstParameters().size(); i++) {
      Exprent parameter = invocation.getLstParameters().get(i);
      applyDomain(parameter, mappings.parameterDomain(invoked, i), descriptor.params[i]);
      applyArrayInitializerSemantics(parameter, mappings.parameterArraySemantics(invoked, i));
      decorate(parameter);
    }
  }

  private String domainOf(Exprent exprent) {
    return unique(factsOf(exprent).domains());
  }

  private Set<String> invocationDomains(InvocationExprent invocation) {
    MemberKey invoked = invocationKey(invocation);
    String declaredDomain = mappings.returnDomain(invoked);
    if (declaredDomain != null) return Set.of(declaredDomain);

    Integer sourceParameter = mappings.returnDomainSource(invoked);
    if (sourceParameter == null || sourceParameter < 0 || sourceParameter >= invocation.getLstParameters().size()) return Set.of();
    // The mapping explicitly promises that the result keeps the argument's
    // semantic meaning, including ambiguity between multiple possible domains.
    return factsOf(invocation.getLstParameters().get(sourceParameter)).domains();
  }

  private SemanticFacts factsOf(Exprent exprent) {
    if (exprent instanceof FieldExprent field) {
      MemberKey fieldMember = fieldKey(field);
      return SemanticFacts.of(mappings.fieldDomain(fieldMember), mappings.fieldArraySemantics(fieldMember));
    }
    if (exprent instanceof InvocationExprent invocation) {
      ArraySemantics array = mappings.returnArraySemantics(invocationKey(invocation));
      return new SemanticFacts(invocationDomains(invocation), array == null ? Set.of() : Set.of(array));
    }
    if (exprent instanceof VarExprent variable) {
      return variableOccurrenceFacts == null
        ? variableFacts.getOrDefault(variable.getVarVersionPair(), SemanticFacts.EMPTY)
        : variableOccurrenceFacts.getOrDefault(variable, SemanticFacts.EMPTY);
    }
    if (exprent instanceof FunctionExprent function && function.getLstOperands().size() == 1 && function.getFuncType().castType != null) {
      return factsOf(function.getLstOperands().get(0));
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      return factsOf(function.getLstOperands().get(1)).merge(factsOf(function.getLstOperands().get(2)));
    }
    if (exprent instanceof AssignmentExprent assignment) {
      // An assignment expression evaluates to its RHS.
      return factsOf(assignment.getRight());
    }
    if (exprent instanceof ArrayExprent array) {
      return arrayElementFacts(array);
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      String domain = flagDomainOf(function);
      return domain == null ? SemanticFacts.EMPTY : SemanticFacts.of(domain, null);
    }
    return SemanticFacts.EMPTY;
  }

  private SemanticFacts arrayElementFacts(ArrayExprent array) {
    Set<String> domains = new HashSet<>();
    Set<ArraySemantics> arrays = new HashSet<>();
    Long slot = literal(array.getIndex());
    for (ArraySemantics semantics : factsOf(array.getArray()).arrays()) {
      if (array.getExprType().arrayDim == 0) {
        String domain = slot == null ? null : slotElementDomain(semantics, slot);
        if (domain == null) domain = semantics.elementDomain();
        if (domain != null) domains.add(domain);
      }
      else {
        ArraySemantics element = semantics.element();
        String domain = slot == null ? null : slotElementDomain(semantics, slot);
        if (domain != null) element = element.withElementDomain(domain);
        if (!element.isEmpty()) arrays.add(element);
      }
    }
    return new SemanticFacts(domains, arrays);
  }

  private void applyDomain(Exprent exprent, String domain, VarType expectedType) {
    if (domain == null) return;
    if (exprent instanceof FunctionExprent function && function.getLstOperands().size() == 1 && function.getFuncType().castType != null) {
      applyDomain(function.getLstOperands().get(0), domain, expectedType);
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
      applyDomain(assignment.getRight(), domain, expectedType);
      return;
    }
    if (!(exprent instanceof ConstExprent constant)) return;
    Long literal = literal(constant);
    if (literal == null) return;
    SymbolicExpression expression = mappings.symbolicExpression(domain, literal, currentOwner, bitWidth(expectedType));
    if (expression == null) return;
    constant.setSymbolicExpression(new ConstExprent.SymbolicExpression(expression.values().stream()
      .map(value -> new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), value.value()))
      .toList(), expression.residual(), expression.complemented(), expression.longLiteral(), primitiveDescriptor(expectedType)));
  }

  private void applyArrayInitializerSemantics(Exprent exprent, ArraySemantics semantics) {
    if (semantics == null || !(exprent instanceof NewExprent array) || array.getLstArrayElements().isEmpty()) return;

    VarType elementType = array.getNewType().decreaseArrayDim();
    ArraySemantics nestedSemantics = semantics.element();
    for (int index = 0; index < array.getLstArrayElements().size(); index++) {
      Exprent element = array.getLstArrayElements().get(index);
      if (elementType.arrayDim > 0) {
        applyArrayInitializerSemantics(element, nestedSemantics);
      }
      else {
        String domain = semantics.elementDomain();
        if (semantics.slotDomains().containsKey(0)) {
          String slotDomain = slotElementDomain(semantics, index);
          if (slotDomain != null) domain = slotDomain;
        }
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

  private static boolean isBitwise(FunctionExprent function) {
    return switch (function.getFuncType()) {
      case AND, OR, XOR, BIT_NOT -> true;
      default -> false;
    };
  }

  private String flagDomainOf(FunctionExprent function) {
    Set<String> domains = new HashSet<>();
    for (Exprent operand : function.getLstOperands()) domains.addAll(factsOf(operand).domains());
    if (domains.size() != 1) return null;
    String domain = domains.iterator().next();
    return "flags".equals(mappings.domainKind(domain)) ? domain : null;
  }

  private static MemberKey fieldKey(FieldExprent field) {
    return new MemberKey(field.getClassname(), field.getName(), field.getDescriptor().descriptorString);
  }

  private static MemberKey invocationKey(InvocationExprent invocation) {
    return new MemberKey(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor());
  }

  private Set<ArraySemantics> arraySemanticsOf(Exprent exprent) {
    return factsOf(exprent).arrays();
  }

  private static <T> T unique(Set<T> candidates) {
    return candidates.size() == 1 ? candidates.iterator().next() : null;
  }

  private String slotElementDomain(ArraySemantics semantics, long slot) {
    String slotDomain = semantics.slotDomains().get(0);
    if (slotDomain == null) return null;
    Value value = mappings.value(slotDomain, slot, currentOwner);
    return value == null ? null : value.elementDomain();
  }

  private static Long literal(Exprent exprent) {
    if (!(exprent instanceof ConstExprent constant) || !(constant.getValue() instanceof Number number)) return null;
    return number.longValue();
  }

  private void collectVariableAssignments() {
    for (Exprent root : roots) {
      walk(root, exprent -> {
        if (exprent instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent) {
          variableAssignments.add(assignment);
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
    return new VariableSemanticsSnapshot(factsByOccurrence);
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
