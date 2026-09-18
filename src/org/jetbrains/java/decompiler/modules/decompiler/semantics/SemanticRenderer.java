// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.Collections;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;
import org.jetbrains.java.decompiler.modules.decompiler.exps.AssignmentExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.SymbolicExpression;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Value;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticFacts.*;

/** Plans value-preserving constant presentations after inference has converged. */
final class SemanticRenderer implements SemanticUses.Sink {
  private final SemanticAnalysis analysis;
  private final SemanticMappings mappings;
  private final String currentOwner;
  private final Map<ConstExprent, ConstantContext> constantContexts = new IdentityHashMap<>();
  private final Map<ConstExprent, Set<ConstExprent.SemanticOffset>> offsetContexts = new IdentityHashMap<>();
  private final Set<ConstExprent> intBitwiseOperands = Collections.newSetFromMap(new IdentityHashMap<>());
  private record ConstantContext(Set<String> domains, VarType expectedType) {}
  SemanticRenderer(SemanticAnalysis analysis) {
    this.analysis = analysis;
    mappings = analysis.mappings;
    currentOwner = analysis.currentOwner;
  }
  public void domain(Exprent expression, String domain, VarType type) {
    applyDomain(expression, domain, type);
  }
  public void array(Exprent expression, ArraySemantics shape) {
    applyArrayInitializerSemantics(expression, shape);
  }
  public void bitwise(Exprent expression, VarType type) {
    if (type.equals(VarType.VARTYPE_INT) && expression instanceof ConstExprent constant)
      intBitwiseOperands.add(constant);
  }
  public void offset(ConstExprent expression, ConstExprent.SemanticOffset offset) {
    offsetContexts.computeIfAbsent(expression, ignored -> new HashSet<>()).add(offset);
  }
  void finish() {
    for (Exprent root : analysis.roots)
      walk(root, expression -> {
        if (!(expression instanceof ConstExprent constant))
          return;
        SemanticFacts required = analysis.graph.requirements(expression);
        if (required.domains().isEmpty())
          return;
        ConstantContext context = constantContexts.get(constant);
        if (context == null) {
          for (String domain : required.domains()) applyDomain(constant, domain, constant.getExprType());
        } else
          context.domains().addAll(required.domains());
      });
    renderConstants();
  }

  private void applyDomain(Exprent exprent, String domain, VarType expectedType) {
    if (domain == null)
      return;
    if (exprent instanceof NewExprent creation && creation.getConstructor() != null) {
      Exprent boxed = boxedArgument(creation.getConstructor());
      if (boxed != null) {
        applyDomain(boxed, domain, boxed.getExprType());
        return;
      }
    }
    if (exprent instanceof InvocationExprent invocation) {
      Exprent boxed = boxedArgument(invocation);
      if (boxed != null) {
        applyDomain(boxed, domain, boxed.getExprType());
        return;
      }
    }
    if ("packed".equals(mappings.domainKind(domain)))
      decoratePacking(exprent, domain);
    if (exprent instanceof FunctionExprent function
      && (isValuePreservingCast(function)
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
    if (exprent instanceof FunctionExprent function && isBitwise(function) && "flags".equals(mappings.domainKind(domain))) {
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
    if (!(exprent instanceof ConstExprent constant))
      return;
    Long literal = literal(constant);
    if (literal == null
      && !(constant.getExprType().equals(VarType.VARTYPE_STRING) && constant.getValue() instanceof String
        && "string".equals(mappings.domainKind(domain))))
      return;
    // Comparisons promote narrow values to int (or long). The other operand
    // may lie outside the annotated value's storage range; narrowing that
    // literal while rendering would change the comparison's result.
    if (literal != null && !fitsType(literal, expectedType))
      expectedType = constant.getExprType();
    ConstantContext context = constantContexts.computeIfAbsent(constant, ignored -> new ConstantContext(new HashSet<>(), null));
    context.domains().add(domain);
    constantContexts.put(constant, new ConstantContext(context.domains(), expectedType));
  }

  private void decoratePacking(Exprent expression, String domain) {
    if (expression instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.OR) {
      for (Exprent operand : function.getLstOperands()) decoratePacking(operand, domain);
      return;
    }
    for (var field : mappings.bitFields(domain)) {
      if (field.selectorMask() != 0)
        continue;
      Exprent value = SemanticBitAccess.packingValue(expression, field.shift(), field.bits());
      if (value != null)
        applyDomain(value, field.domain(), value.getExprType());
    }
  }

  private void renderConstants() {
    // Consumer and producer contexts can disagree. Collect both before rendering
    // so traversal order never decides which conflicting name wins.
    constantContexts.forEach((constant, context) -> {
      String domain = unique(context.domains());
      if (domain == null) {
        // Numeric presentation preferences need not imply identical meanings.
        // RGB and ARGB can both request the same value-preserving hex literal;
        // conflicting symbolic names still remain unresolved.
        String common = commonFormat(constant, context.domains());
        if (common != null)
          constant.setSemanticLiteral(common);
        return;
      }
      if (constant.getValue() instanceof String text) {
        Value value = mappings.stringValue(domain, text, currentOwner);
        if (value != null)
          constant.setSymbolicExpression(
            new ConstExprent.SymbolicExpression(List.of(new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), 0)),
              null, false, false, "Ljava/lang/String;"));
        return;
      }
      String formatted = mappings.formattedLiteral(domain, literal(constant), constant.getValue() instanceof Long);
      if (formatted != null) {
        constant.setSemanticLiteral(formatted);
        return;
      }
      SymbolicExpression expression =
        mappings.symbolicExpression(domain, literal(constant), currentOwner, bitWidth(context.expectedType()));
      if (expression == null)
        return;
      String target = primitiveDescriptor(context.expectedType());
      if (intBitwiseOperands.contains(constant) && literal(constant) < 0 && expression.residual() != null) {
        // A byte/short cast can express sign extension without a numeric high-bit
        // residual. Only do this inside int bitwise operations: Java promotes the
        // cast back to int there, so overload selection and long widths cannot change.
        for (VarType narrow : List.of(VarType.VARTYPE_BYTE, VarType.VARTYPE_SHORT)) {
          if (!fitsType(literal(constant), narrow))
            continue;
          SymbolicExpression candidate = mappings.symbolicExpression(domain, literal(constant), currentOwner, bitWidth(narrow));
          if (candidate != null && candidate.residual() == null && !candidate.complemented()
            && candidate.values().size() <= expression.values().size()) {
            expression = candidate;
            target = primitiveDescriptor(narrow);
            break;
          }
        }
      }
      constant.setSymbolicExpression(new ConstExprent.SymbolicExpression(
        expression.values()
          .stream()
          .map(value -> new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), value.value()))
          .toList(),
        expression.residual(), expression.complemented(), expression.longLiteral(), target));
    });
    offsetContexts.forEach((constant, offsets) -> {
      ConstExprent.SemanticOffset offset = unique(offsets);
      if (offset != null && !constantContexts.containsKey(constant))
        constant.setSemanticOffset(offset);
    });
  }

  private String commonFormat(ConstExprent constant, Set<String> domains) {
    Long value = literal(constant);
    if (value == null)
      return null;
    String result = null, normalized = null;
    for (String domain : domains) {
      String format = mappings.formattedLiteral(domain, value, constant.getValue() instanceof Long);
      if (format == null)
        return null;
      String comparable = format.replaceFirst("(?i)^0x0+(?=[0-9a-f])", "0x");
      if (normalized != null && !normalized.equals(comparable))
        return null;
      normalized = comparable;
      // Hex widths specify padding. The wider agreed representation satisfies
      // both preferences without asserting either domain's symbolic identity.
      if (result == null || format.length() > result.length())
        result = format;
    }
    return result;
  }

  private void applyArrayInitializerSemantics(Exprent exprent, ArraySemantics semantics) {
    if (semantics == null)
      return;
    if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
      applyArrayInitializerSemantics(function.getLstOperands().get(1), semantics);
      applyArrayInitializerSemantics(function.getLstOperands().get(2), semantics);
      return;
    }
    if (exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null) {
      applyArrayInitializerSemantics(assignment.getRight(), semantics);
      return;
    }
    if (!(exprent instanceof NewExprent array) || array.getLstArrayElements().isEmpty())
      return;

    VarType elementType = array.getNewType().decreaseArrayDim();
    ArraySemantics nestedSemantics = semantics.element();
    for (int index = 0; index < array.getLstArrayElements().size(); index++) {
      Exprent element = array.getLstArrayElements().get(index);
      String slotDomain = analysis.slotElementDomain(semantics, index);
      if (elementType.arrayDim > 0) {
        applyArrayInitializerSemantics(element, slotDomain == null ? nestedSemantics : nestedSemantics.withElementDomain(slotDomain));
      } else {
        String domain = semantics.elementDomain();
        if (slotDomain != null)
          domain = slotDomain;
        applyDomain(element, domain, elementType);
      }
    }
  }
}
