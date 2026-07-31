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
import org.jetbrains.java.decompiler.modules.decompiler.exps.SwitchHeadExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Value;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;

public final class SemanticConstantsProcessor {
  private final SemanticMappings mappings;
  private final MemberKey method;
  private final String currentOwner;
  private final VarProcessor varProcessor;
  private final Map<VarVersionPair, Set<String>> variableDomains = new HashMap<>();
  private final Map<VarVersionPair, Set<ArraySemantics>> variableArraySemantics = new HashMap<>();
  private final List<Exprent> roots = new ArrayList<>();

  private SemanticConstantsProcessor(SemanticMappings mappings, StructClass owner, StructMethod method, VarProcessor varProcessor) {
    this.mappings = mappings;
    this.method = new MemberKey(owner.qualifiedName, method.getName(), method.getDescriptor());
    this.currentOwner = owner.qualifiedName;
    this.varProcessor = varProcessor;
  }

  public static void process(Statement root, StructClass owner, StructMethod method, VarProcessor varProcessor,
                             SemanticMappings mappings) {
    SemanticConstantsProcessor processor = new SemanticConstantsProcessor(mappings, owner, method, varProcessor);
    processor.collectRoots(root);
    processor.seedParameterDomains(method);
    processor.propagateVariableDomains();
    for (Exprent exprent : processor.roots) processor.decorate(exprent);
  }

  private void collectRoots(Statement statement) {
    List<Exprent> exprents = statement.getExprents() == null ? statement.getStatExprents() : statement.getExprents();
    roots.addAll(exprents);
    for (Statement child : statement.getStats()) collectRoots(child);
  }

  private void seedParameterDomains(StructMethod structMethod) {
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(structMethod.getDescriptor());
    int slot = structMethod.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    Map<Integer, String> slotDomains = new HashMap<>();
    Map<Integer, ArraySemantics> arraySlotDomains = new HashMap<>();
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      String domain = mappings.parameterDomain(method, parameter);
      if (domain != null) slotDomains.put(slot, domain);
      ArraySemantics arraySemantics = mappings.parameterArraySemantics(method, parameter);
      if (arraySemantics != null) arraySlotDomains.put(slot, arraySemantics);
      slot += descriptor.params[parameter].stackSize;
    }
    Set<VarVersionPair> parameters = new HashSet<>(varProcessor.getParams());
    forEachExprent(exprent -> {
      if (exprent instanceof VarExprent variable && parameters.contains(variable.getVarVersionPair())) {
        Integer original = varProcessor.getVarOriginalIndex(variable.getIndex());
        String domain = slotDomains.get(original == null ? variable.getIndex() : original);
        if (domain != null) mergeVariableDomain(variable.getVarVersionPair(), domain);
        ArraySemantics arraySemantics = arraySlotDomains.get(original == null ? variable.getIndex() : original);
        if (arraySemantics != null) mergeVariableArraySemantics(variable.getVarVersionPair(), arraySemantics);
      }
    });
  }

  private void propagateVariableDomains() {
    boolean changed;
    do {
      boolean[] added = {false};
      forEachExprent(exprent -> {
        if (exprent instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent variable) {
          for (String domain : domainsOf(assignment.getRight())) {
            added[0] |= mergeVariableDomain(variable.getVarVersionPair(), domain);
          }
          for (ArraySemantics arraySemantics : arraySemanticsOf(assignment.getRight())) {
            added[0] |= mergeVariableArraySemantics(variable.getVarVersionPair(), arraySemantics);
          }
        }
      });
      changed = added[0];
    } while (changed);
  }

  private boolean mergeVariableDomain(VarVersionPair variable, String domain) {
    return variableDomains.computeIfAbsent(variable, ignored -> new HashSet<>()).add(domain);
  }

  private boolean mergeVariableArraySemantics(VarVersionPair variable, ArraySemantics semantics) {
    return variableArraySemantics.computeIfAbsent(variable, ignored -> new HashSet<>()).add(semantics);
  }

  private void decorate(Exprent exprent) {
    if (exprent instanceof AssignmentExprent assignment) {
      decorate(assignment.getLeft());
      applyDomain(assignment.getRight(), domainOf(assignment.getLeft()));
      decorate(assignment.getRight());
      return;
    }
    if (exprent instanceof InvocationExprent invocation) {
      if (invocation.getInstance() != null) decorate(invocation.getInstance());
      MemberKey invoked = new MemberKey(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor());
      for (int i = 0; i < invocation.getLstParameters().size(); i++) {
        Exprent parameter = invocation.getLstParameters().get(i);
        applyDomain(parameter, mappings.parameterDomain(invoked, i));
        decorate(parameter);
      }
      return;
    }
    if (exprent instanceof SwitchHeadExprent switchHead) {
      String domain = domainOf(switchHead.getValue());
      decorate(switchHead.getValue());
      for (List<Exprent> cases : switchHead.getCaseValues()) {
        for (Exprent caseValue : cases) {
          if (caseValue != null) {
            applyDomain(caseValue, domain);
            decorate(caseValue);
          }
        }
      }
      return;
    }
    if (exprent instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.RETURN && exit.getValue() != null) {
      applyDomain(exit.getValue(), mappings.returnDomain(method));
      decorate(exit.getValue());
      return;
    }
    if (exprent instanceof ArrayExprent array) {
      ArraySemantics semantics = uniqueArraySemantics(arraySemanticsOf(array.getArray()));
      decorate(array.getArray());
      if (semantics != null) {
        String domain = semantics.indexDomains().get(0);
        if (domain == null) domain = semantics.slotDomains().get(0);
        applyDomain(array.getIndex(), domain);
      }
      decorate(array.getIndex());
      return;
    }
    if (exprent instanceof FunctionExprent function && isComparison(function) && function.getLstOperands().size() == 2) {
      Exprent left = function.getLstOperands().get(0);
      Exprent right = function.getLstOperands().get(1);
      applyDomain(left, domainOf(right));
      applyDomain(right, domainOf(left));
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      String domain = flagDomainOf(function);
      if (domain != null) {
        for (Exprent operand : function.getLstOperands()) applyDomain(operand, domain);
      }
    }
    for (Exprent child : exprent.getAllExprents()) decorate(child);
  }

  private String domainOf(Exprent exprent) {
    if (exprent instanceof FieldExprent field) return mappings.fieldDomain(fieldKey(field));
    if (exprent instanceof InvocationExprent invocation) {
      return mappings.returnDomain(new MemberKey(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor()));
    }
    if (exprent instanceof VarExprent variable) {
      Set<String> domains = variableDomains.get(variable.getVarVersionPair());
      return domains != null && domains.size() == 1 ? domains.iterator().next() : null;
    }
    if (exprent instanceof ArrayExprent array) {
      ArraySemantics semantics = uniqueArraySemantics(arraySemanticsOf(array.getArray()));
      if (semantics == null) return null;
      String slotDomain = semantics.slotDomains().get(0);
      Long slot = literal(array.getIndex());
      if (array.getExprType().arrayDim == 0 && slotDomain != null && slot != null) {
        String domain = slotElementDomain(semantics, slot);
        if (domain != null) return domain;
      }
      return array.getExprType().arrayDim == 0 ? semantics.elementDomain() : null;
    }
    if (exprent instanceof FunctionExprent function && function.getLstOperands().size() == 1 && function.getFuncType().castType != null) {
      return domainOf(function.getLstOperands().get(0));
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      return flagDomainOf(function);
    }
    return null;
  }

  private Set<String> domainsOf(Exprent exprent) {
    if (exprent instanceof VarExprent variable) {
      return variableDomains.getOrDefault(variable.getVarVersionPair(), Set.of());
    }
    if (exprent instanceof FunctionExprent function && function.getLstOperands().size() == 1 && function.getFuncType().castType != null) {
      return domainsOf(function.getLstOperands().get(0));
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      Set<String> result = new HashSet<>(domainsOf(function.getLstOperands().get(1)));
      result.addAll(domainsOf(function.getLstOperands().get(2)));
      return result;
    }
    if (exprent instanceof AssignmentExprent assignment) {
      return domainsOf(assignment.getRight());
    }
    String domain = domainOf(exprent);
    return domain == null ? Set.of() : Set.of(domain);
  }

  private void applyDomain(Exprent exprent, String domain) {
    if (domain == null) return;
    if (exprent instanceof FunctionExprent function && function.getLstOperands().size() == 1 && function.getFuncType().castType != null) {
      applyDomain(function.getLstOperands().get(0), domain);
      return;
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      applyDomain(function.getLstOperands().get(1), domain);
      applyDomain(function.getLstOperands().get(2), domain);
      return;
    }
    if (exprent instanceof FunctionExprent function
        && function.getFuncType() == FunctionExprent.FunctionType.BIT_NOT
        && "flags".equals(mappings.domainKind(domain))) {
      applyDomain(function.getLstOperands().get(0), domain);
      return;
    }
    if (exprent instanceof AssignmentExprent assignment) {
      applyDomain(assignment.getRight(), domain);
      return;
    }
    if (!(exprent instanceof ConstExprent constant)) return;
    Long literal = literal(constant);
    if (literal == null) return;
    List<Value> values = mappings.expressionValues(domain, literal, currentOwner);
    if (values.isEmpty()) return;
    constant.setSymbolicReferences(values.stream()
      .map(value -> new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc()))
      .toList());
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
    for (Exprent operand : function.getLstOperands()) domains.addAll(domainsOf(operand));
    if (domains.size() != 1) return null;
    String domain = domains.iterator().next();
    return "flags".equals(mappings.domainKind(domain)) ? domain : null;
  }

  private static MemberKey fieldKey(FieldExprent field) {
    return new MemberKey(field.getClassname(), field.getName(), field.getDescriptor().descriptorString);
  }

  private Set<ArraySemantics> arraySemanticsOf(Exprent exprent) {
    if (exprent instanceof FieldExprent field) {
      ArraySemantics semantics = mappings.fieldArraySemantics(fieldKey(field));
      return semantics == null ? Set.of() : Set.of(semantics);
    }
    if (exprent instanceof InvocationExprent invocation) {
      ArraySemantics semantics = mappings.returnArraySemantics(
        new MemberKey(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor())
      );
      return semantics == null ? Set.of() : Set.of(semantics);
    }
    if (exprent instanceof VarExprent variable) {
      return variableArraySemantics.getOrDefault(variable.getVarVersionPair(), Set.of());
    }
    if (exprent instanceof ArrayExprent array) {
      Set<ArraySemantics> result = new HashSet<>();
      for (ArraySemantics semantics : arraySemanticsOf(array.getArray())) {
        ArraySemantics element = semantics.element();
        Long slot = literal(array.getIndex());
        if (slot != null) {
          String domain = slotElementDomain(semantics, slot);
          if (domain != null) element = element.withElementDomain(domain);
        }
        if (!element.isEmpty()) result.add(element);
      }
      return result;
    }
    if (exprent instanceof AssignmentExprent assignment) {
      return arraySemanticsOf(assignment.getRight());
    }
    if (exprent instanceof FunctionExprent function && function.getLstOperands().size() == 1 && function.getFuncType().castType != null) {
      return arraySemanticsOf(function.getLstOperands().get(0));
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      Set<ArraySemantics> result = new HashSet<>(arraySemanticsOf(function.getLstOperands().get(1)));
      result.addAll(arraySemanticsOf(function.getLstOperands().get(2)));
      return result;
    }
    return Set.of();
  }

  private static ArraySemantics uniqueArraySemantics(Set<ArraySemantics> candidates) {
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

  private void forEachExprent(Consumer<Exprent> consumer) {
    for (Exprent root : roots) walk(root, consumer);
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
