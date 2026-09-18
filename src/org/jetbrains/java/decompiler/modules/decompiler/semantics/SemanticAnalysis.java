// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ArrayExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.CallBinding;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Condition;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ContainerSemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.RecordLayout;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.SlotSource;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Value;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticFacts.*;

/** Method-scoped transfer semantics evaluated by the value graph before source locals merge. */
final class SemanticAnalysis {
  final SemanticMappings mappings;
  final MemberKey method;
  final String currentOwner;
  final VarType returnType;
  final SemanticContext context = new SemanticContext();
  final Map<Integer, SemanticContext.Key> parameterKeys = new HashMap<>();
  final List<Exprent> roots;
  final SemanticFlowGraph graph;
  final SemanticLocalFlow locals;
  private final VarProcessor varProcessor;

  SemanticAnalysis(Statement root, StructClass owner, StructMethod method, VarProcessor variables, SemanticMappings mappings) {
    this.mappings = mappings;
    this.method = mappings.namedMember(new MemberKey(owner.qualifiedName, method.getName(), method.getDescriptor()));
    this.currentOwner = mappings.namedOwner(owner.qualifiedName);
    this.returnType = MethodDescriptor.parseDescriptor(this.method.desc()).ret;
    this.varProcessor = variables;
    this.roots = roots(root);
    context.analyze(root);
    Map<Integer, SemanticFacts> parameters = parameterSlotFacts(method);
    Set<VarVersionPair> incoming = new HashSet<>();
    for (int index : parameters.keySet()) incoming.add(new VarVersionPair(index, 0));
    locals = new SemanticLocalFlow((org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement) root, incoming);
    context.localFlow(locals);
    graph = new SemanticFlowGraph(this, roots);
    for (var entry : parameters.entrySet()) graph.parameter(new VarVersionPair(entry.getKey(), 0), entry.getValue());
  }

  SemanticFacts factsOf(Exprent expression) {
    return graph.facts(expression);
  }

  Map<Integer, SemanticFacts> parameterSlotFacts(StructMethod structMethod) {
    MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(structMethod.getDescriptor());
    int parameterSlot = structMethod.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    Map<Integer, SemanticFacts> slotFacts = new HashMap<>();
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      int index = parameterSlot;
      for (VarVersionPair pair : varProcessor.getParams()) {
        Integer original = varProcessor.getVarOriginalIndex(pair.var);
        if (original != null && original == parameterSlot) {
          index = pair.var;
          break;
        }
      }
      parameterKeys.put(parameter, SemanticContext.variable(index, 0));
      parameterSlot += descriptor.params[parameter].stackSize;
    }
    for (int parameter = 0; parameter < descriptor.params.length; parameter++) {
      SemanticFacts facts = SemanticFacts.declaration(mappings.parameterDomain(method, parameter),
        mappings.parameterArraySemantics(method, parameter), mappings.container(method, "parameter", parameter));
      List<SemanticMappings.Condition> conditions = mappings.conditions(method, parameter);
      if (!conditions.isEmpty())
        facts = SemanticFacts.conditional(parameterKeys.get(conditions.get(0).parameter()), conditions);
      slotFacts.put(((SemanticContext.Variable) parameterKeys.get(parameter).operation()).index(), facts);
    }
    return slotFacts;
  }

  SemanticFacts resolveFacts(Exprent at, SemanticFacts facts, boolean requireKnown) {
    if (facts.dependent().isEmpty())
      return facts;
    Set<String> domains = new HashSet<>(facts.domains());
    Set<DependentDomain> pending = new HashSet<>();
    boolean unknown = facts.unknown();
    for (DependentDomain dependent : facts.dependent()) {
      if (dependent instanceof ConditionalDomain conditional) {
        Long value = context.known(at, conditional.selector());
        String domain = selectDomain(at, conditional.selector(), conditional.conditions());
        if (domain != null)
          domains.add(domain);
        else if (value == null && !requireKnown)
          pending.add(dependent);
        else
          unknown = true;
      } else if (dependent instanceof PackedDomain packed) {
        Set<String> matching = new HashSet<>();
        boolean unresolved = false;
        for (PackedCase field : packed.cases()) {
          Long value = context.known(at, field.selector());
          if (value != null && value == field.value())
            matching.add(field.domain());
          else if (value == null && !context.excludes(at, field.selector(), field.value()))
            unresolved = true;
        }
        // Validated overlapping fields have mutually exclusive selectors. A
        // proven match therefore rules out all other alternatives for the slice.
        if (!matching.isEmpty())
          domains.addAll(matching);
        else if (unresolved && !requireKnown)
          pending.add(dependent);
        else
          unknown = true;
      }
    }
    return new SemanticFacts(domains, facts.arrays(), facts.containers(), pending, unknown);
  }

  static String selectDomain(Long value, List<SemanticMappings.Condition> conditions) {
    if (value == null)
      return null;
    Set<String> domains = new HashSet<>();
    String fallback = null;
    for (SemanticMappings.Condition condition : conditions) {
      if (condition.otherwise())
        fallback = condition.domain();
      else if (value.equals(condition.equalsValue()) || condition.notEqualsValue() != null && !value.equals(condition.notEqualsValue())) {
        domains.add(condition.domain());
      }
    }
    return domains.isEmpty() ? fallback : unique(domains);
  }

  String selectDomain(Exprent at, SemanticContext.Key selector, List<SemanticMappings.Condition> conditions) {
    Long value = context.known(at, selector);
    if (value != null)
      return selectDomain(value, conditions);
    for (SemanticMappings.Condition condition : conditions) {
      if (condition.notEqualsValue() != null && context.excludes(at, selector, condition.notEqualsValue()))
        return condition.domain();
      if (condition.otherwise()
        && conditions.stream().filter(c -> c.equalsValue() != null).allMatch(c -> context.excludes(at, selector, c.equalsValue())))
        return condition.domain();
    }
    return null;
  }

  String domainOf(Exprent exprent) {
    SemanticFacts facts = resolveFacts(exprent, factsOf(exprent), true);
    return facts.unknown() ? null : unique(facts.domains());
  }

  Set<String> scopedCallDomains(InvocationExprent invocation, Integer parameter) {
    List<CallBinding> bindings = mappings.callBindings(method);
    if (bindings.isEmpty())
      return Set.of();
    Set<String> scoped = new HashSet<>();
    MemberKey invoked = mappings.namedMember(invocationKey(invocation));
    for (CallBinding binding : bindings) {
      if (java.util.Objects.equals(parameter, binding.parameter()) && invocation.bytecode != null && binding.offset() >= 0
        && invocation.bytecode.get(binding.offset()) && binding.callee().equals(invoked)
        && !childOwnsCallOffset(invocation, binding.offset())) {
        scoped.add(binding.domain());
      }
    }
    return scoped;
  }

  SemanticFacts invocationFacts(InvocationExprent invocation) {
    MemberKey invoked = invocationKey(invocation);
    if (invocation.getExprType().arrayDim == 0 && !invocation.getExprType().equals(VarType.VARTYPE_VOID)) {
      Set<String> scoped = scopedCallDomains(invocation, null);
      if (!scoped.isEmpty())
        return new SemanticFacts(scoped, Set.of(), Set.of(), Set.of(), false);
    }
    String declaredDomain = mappings.returnDomain(invoked);
    ArraySemantics array = mappings.returnArraySemantics(invoked);
    SemanticMappings.ContainerSemantics container = mappings.container(invoked, "return", -1);
    if (declaredDomain != null || array != null || container != null)
      return SemanticFacts.declaration(declaredDomain, array, container);
    List<SemanticMappings.Condition> conditions = mappings.conditions(invoked, -1);
    if (!conditions.isEmpty() && conditions.get(0).parameter() < invocation.getLstParameters().size()) {
      Exprent selector = invocation.getLstParameters().get(conditions.get(0).parameter());
      Long value = context.value(selector);
      if (value != null)
        return SemanticFacts.of(selectDomain(value, conditions), null);
      return SemanticFacts.conditional(context.key(selector), conditions);
    }
    SemanticMappings.SlotSource tableSource = mappings.slotSource(invoked, -1);
    if (tableSource != null)
      return slotSourceFacts(invocation, tableSource);
    Exprent boxed = boxedArgument(invocation);
    if (boxed != null)
      return factsOf(boxed);
    if (isUnboxing(invocation)) {
      if (unboxingPreservesStorage(invocation))
        return factsOf(invocation.getInstance());
      SemanticFacts source = resolveFacts(invocation.getInstance(), factsOf(invocation.getInstance()), true);
      String target = primitiveDescriptor(invocation.getExprType());
      if (source.domains().stream().allMatch(domain -> mappings.fitsIntegralType(domain, target)))
        return source;
      return SemanticFacts.UNKNOWN;
    }
    SemanticFacts contents = containerCallFacts(invocation);
    if (contents != null)
      return contents;

    Integer sourceParameter = mappings.returnDomainSource(invoked);
    if (sourceParameter == null || sourceParameter < 0 || sourceParameter >= invocation.getLstParameters().size())
      return SemanticFacts.UNKNOWN;
    // The mapping explicitly promises that the result keeps the argument's
    // semantic meaning, including ambiguity between multiple possible domains.
    return factsOf(invocation.getLstParameters().get(sourceParameter));
  }

  SemanticFacts slotSourceFacts(InvocationExprent invocation, SemanticMappings.SlotSource source) {
    if (source.parameter() < 0 || source.parameter() >= invocation.getLstParameters().size())
      return SemanticFacts.UNKNOWN;
    SemanticFacts table = factsOf(invocation.getLstParameters().get(source.parameter()));
    if (table.isEmpty())
      return SemanticFacts.BOTTOM;
    if (table.unknown() || table.arrays().isEmpty())
      return SemanticFacts.UNKNOWN;
    SemanticFacts result = SemanticFacts.BOTTOM;
    for (ArraySemantics array : table.arrays()) {
      RecordLayout layout = array.records().get(source.dimension());
      // The source names a column, not an absolute array index after a header.
      if (layout != null && !layout.planes() && source.slot() >= layout.stride())
        return SemanticFacts.UNKNOWN;
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
          if (outer != null)
            for (String rowDomain : mappings.slotElementDomains(outer.domain())) {
              result = result.merge(SemanticFacts.of(rowDomain, null));
            }
        }
      }
    }
    return result;
  }

  static boolean childOwnsCallOffset(Exprent expression, int offset) {
    for (Exprent child : expression.getAllExprents()) {
      if (child instanceof InvocationExprent && child.bytecode != null && child.bytecode.get(offset) || childOwnsCallOffset(child, offset))
        return true;
    }
    return false;
  }

  SemanticFacts computeFacts(Exprent exprent) {
    if (exprent instanceof ConstExprent)
      return SemanticFacts.BOTTOM;
    if (exprent instanceof FieldExprent field) {
      MemberKey fieldMember = fieldKey(field);
      return SemanticFacts.declaration(
        mappings.fieldDomain(fieldMember), mappings.fieldArraySemantics(fieldMember), mappings.container(fieldMember, "field", -1));
    }
    if (exprent instanceof InvocationExprent invocation) {
      return invocationFacts(invocation);
    }
    if (exprent instanceof NewExprent creation && creation.getConstructor() != null) {
      Exprent boxed = boxedArgument(creation.getConstructor());
      if (boxed != null)
        return factsOf(boxed);
      if (SemanticHeap.isContainer(creation))
        return SemanticFacts.BOTTOM;
    }
    if (exprent instanceof FunctionExprent) {
      SemanticFacts extracted = extractionFacts(exprent);
      if (extracted != null)
        return extracted;
    }
    if (exprent instanceof FunctionExprent function && (isValuePreservingCast(function) || isIntegralCast(function))) {
      SemanticFacts source = factsOf(function.getLstOperands().get(0));
      if (isValuePreservingCast(function) || source.equals(SemanticFacts.BOTTOM))
        return source;
      // Preserve the declared numeric values (or flag bits) across an integral
      // narrowing cast. Never turn a not-yet-evaluated definition into UNKNOWN:
      // that would make the fixed point depend on assignment traversal order.
      return !source.unknown() && !source.domains().isEmpty()
          && source.domains().stream().allMatch(domain -> mappings.fitsIntegralType(domain, primitiveDescriptor(function.getExprType())))
        ? source
        : SemanticFacts.UNKNOWN;
    }
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      return factsOf(function.getLstOperands().get(1)).merge(factsOf(function.getLstOperands().get(2)));
    }
    if (exprent instanceof AssignmentExprent assignment) {
      if (assignment.getCondType() == null)
        return factsOf(assignment.getRight());
      // A compound assignment evaluates to the updated LHS, not to its delta.
      return isBitwise(assignment.getCondType()) ? bitwiseFacts(List.of(assignment.getLeft(), assignment.getRight()))
                                                 : SemanticFacts.UNKNOWN;
    }
    if (exprent instanceof ArrayExprent array) {
      return arrayElementFacts(array);
    }
    if (exprent instanceof FunctionExprent function && isBitwise(function)) {
      return bitwiseFacts(function.getLstOperands());
    }
    return SemanticFacts.UNKNOWN;
  }

  SemanticFacts containerCallFacts(InvocationExprent invocation) {
    if (invocation.getInstance() == null
      || !Set.of("java/util/Vector", "java/util/Hashtable", "java/util/Enumeration").contains(invocation.getClassname()))
      return null;
    SemanticFacts source = factsOf(invocation.getInstance());
    if (source.containers().isEmpty())
      return null;
    SemanticFacts result = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), source.unknown());
    String owner = invocation.getClassname(), name = invocation.getName();
    boolean enumeration = name.equals("elements") || name.equals("keys");
    boolean read = owner.equals("java/util/Vector") && Set.of("elementAt", "firstElement", "lastElement").contains(name)
      || owner.equals("java/util/Hashtable") && Set.of("get", "put", "remove").contains(name)
      || owner.equals("java/util/Enumeration") && name.equals("nextElement");
    if (!enumeration && !read)
      return null;
    for (SemanticMappings.ContainerSemantics container : source.containers()) {
      String domain =
        owner.equals("java/util/Hashtable") ? name.equals("keys") ? container.keys() : container.values() : container.elements();
      result = result.merge(enumeration && domain != null
          ? SemanticFacts.declaration(null, null, new SemanticMappings.ContainerSemantics(domain, null, null))
          : SemanticFacts.of(domain, null));
    }
    return result;
  }

  SemanticFacts extractionFacts(Exprent expression) {
    SemanticBitAccess.Extraction extraction = SemanticBitAccess.extraction(expression);
    if (extraction == null)
      return null;
    SemanticFacts source = resolveFacts(extraction.source(), factsOf(extraction.source()), true);
    if (source.domains().stream().allMatch(domain -> mappings.bitFields(domain).isEmpty()))
      return null;
    SemanticFacts result = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), source.unknown());
    boolean matchesShape = false;
    for (String domain : source.domains()) {
      List<PackedCase> cases = new ArrayList<>();
      for (var field : mappings.bitFields(domain)) {
        if (field.shift() != extraction.shift() || field.bits() != extraction.bits() || field.signed() != extraction.signed())
          continue;
        matchesShape = true;
        SemanticContext.Key selector = SemanticContext.constant(0);
        if (field.selectorMask() != 0) {
          selector = SemanticContext.operation(
            FunctionExprent.FunctionType.AND, context.key(extraction.source()), SemanticContext.constant(field.selectorMask()));
        }
        cases.add(new PackedCase(selector, field.selectorValue(), field.domain()));
      }
      result = result.merge(cases.isEmpty()
          ? SemanticFacts.UNKNOWN
          : new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(new PackedDomain(List.copyOf(cases))), false));
    }
    // Masks that do not select a declared field may simply preserve or clear
    // bits in the original word (e.g. unsigned-byte normalization with &255).
    // Keep the packed source's immutable definition key through copies. A
    // later guard can select the field, but a guard on a rewritten source cannot.
    return matchesShape ? resolveFacts(expression, result, false) : null;
  }

  SemanticFacts arrayElementFacts(ArrayExprent array) {
    SemanticFacts source = factsOf(array.getArray());
    SemanticFacts result = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), source.unknown());
    for (ArraySemantics shape : source.arrays()) result = result.merge(project(shape, array.getIndex(), array.getExprType()));
    return result;
  }

  SemanticFacts project(ArraySemantics shape, Exprent index, VarType type) {
    Long slot = literal(index);
    RecordLayout layout = shape.records().get(0);
    Integer column = layout == null ? null : SemanticRecordAccess.slot(index, layout, context);
    String override = column != null ? slotElementDomain(layout.domain(), column) : slot == null ? null : slotElementDomain(shape, slot);
    SemanticFacts result = elementFacts(shape, override, type);
    if (slot == null && column == null) {
      for (String domain : mappings.slotElementDomains(shape.slotDomains().get(0)))
        result = result.merge(elementFacts(shape, domain, type));
      if (layout != null)
        for (String domain : mappings.slotElementDomains(layout.domain())) result = result.merge(elementFacts(shape, domain, type));
    }
    return result;
  }

  static SemanticFacts elementFacts(ArraySemantics semantics, String override, VarType type) {
    if (type.arrayDim == 0)
      return SemanticFacts.of(override == null ? semantics.elementDomain() : override, null);
    ArraySemantics element = semantics.element();
    if (override != null)
      element = element.withElementDomain(override);
    return SemanticFacts.of(null, element.isEmpty() ? null : element);
  }

  String arrayIndexDomain(Exprent array) {
    SemanticFacts facts = factsOf(array);
    if (facts.unknown())
      return null;
    Set<String> domains = new HashSet<>();
    for (ArraySemantics semantics : facts.arrays()) {
      String domain = semantics.indexDomains().get(0);
      if (domain == null)
        domain = semantics.slotDomains().get(0);
      if (domain == null)
        return null;
      domains.add(domain);
    }
    // Index meaning can agree even when possible rows have different leaf domains.
    return unique(domains);
  }

  String flagDomainOf(FunctionExprent function) {
    return flagDomainOf(function.getLstOperands());
  }

  String flagDomainOf(List<Exprent> operands) {
    SemanticFacts facts = bitwiseFacts(operands);
    String domain = facts.unknown() ? null : unique(facts.domains());
    return "flags".equals(mappings.domainKind(domain)) ? domain : null;
  }

  SemanticFacts bitwiseFacts(List<Exprent> operands) {
    SemanticFacts facts = SemanticFacts.BOTTOM;
    for (Exprent operand : operands) {
      // Numeric masks are neutral terms; unknown variables and conflicting
      // domains are not. Keep conflicts even inside a larger bitwise tree.
      if (literal(operand) == null)
        facts = facts.merge(resolveFacts(operand, factsOf(operand), true));
    }
    boolean unknown =
      facts.unknown() || facts.domains().stream().anyMatch(domain -> !Set.of("flags", "packed").contains(mappings.domainKind(domain)));
    return new SemanticFacts(facts.domains(), Set.of(), Set.of(), Set.of(), unknown);
  }

  Set<ArraySemantics> arraySemanticsOf(Exprent exprent) {
    SemanticFacts facts = factsOf(exprent);
    return facts.unknown() ? Set.of() : facts.arrays();
  }

  String slotElementDomain(ArraySemantics semantics, long slot) {
    RecordLayout layout = semantics.records().get(0);
    if (layout != null) {
      Integer recordSlot = SemanticRecordAccess.slot(slot, layout);
      if (recordSlot != null)
        return slotElementDomain(layout.domain(), recordSlot);
    }
    String slotDomain = semantics.slotDomains().get(0);
    return slotElementDomain(slotDomain, slot);
  }

  String slotElementDomain(String slotDomain, long slot) {
    if (slotDomain == null)
      return null;
    Value value = mappings.value(slotDomain, slot, currentOwner);
    return value == null ? null : value.elementDomain();
  }
}
