package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import org.jetbrains.java.decompiler.api.SemanticMappingData.BitFieldEntry;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor.SemanticName;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;

/** Physical field identities for presentation; value-domain inference stays in the value graph. */
final class SemanticPackedFields {
  private record Field(String owner, BitFieldEntry entry) {
    SemanticName name() { return new SemanticName(owner + "." + entry.name(), entry.name()); }
  }
  private final SemanticAnalysis analysis;
  private final Map<Exprent, Field> fields = new IdentityHashMap<>();
  private final Map<Exprent, SemanticName> names = new IdentityHashMap<>();
  private final Map<ConstExprent, Set<SemanticMappings.Value>> literals = new IdentityHashMap<>();

  SemanticPackedFields(SemanticAnalysis analysis) { this.analysis = analysis; }

  void apply(VarProcessor variables) {
    for (Exprent expression : analysis.expressions) {
      SemanticBitAccess.Extraction extraction = SemanticBitAccess.extraction(expression);
      if (extraction == null) continue;
      Field field = field(expression, extraction);
      if (field == null) continue;
      fields.put(expression, field);
      for (SemanticBitAccess.Operand operand : extraction.operands()) addLiteral(field, operand.constant(), operand.part());
    }
    literals.forEach((constant, candidates) -> {
      SemanticMappings.Value value = unique(candidates);
      if (value == null || constant.hasSemanticPresentation()) return;
      constant.setSymbolicExpression(new ConstExprent.SymbolicExpression(
        List.of(new ConstExprent.SymbolicReference(value.owner(), value.name(), value.desc(), value.value())),
        null, false, constant.getValue() instanceof Long, primitiveDescriptor(constant.getExprType())));
    });

    if (fields.isEmpty()) return;

    Map<VarVersionPair, SemanticName> candidates = new LinkedHashMap<>();
    Set<VarVersionPair> ambiguous = new HashSet<>();
    for (Exprent expression : analysis.expressions) {
      if (!(expression instanceof VarExprent variable)) continue;
      VarVersionPair pair = variable.getVarVersionPair();
      SemanticName name = name(variable);
      if (name == null || candidates.containsKey(pair) && !name.equals(candidates.get(pair))) ambiguous.add(pair);
      else candidates.put(pair, name);
    }
    // Authored method parameter names and debug names retain their own precedence.
    for (SemanticContext.Key key : analysis.parameterKeys.values()) {
      SemanticContext.Variable parameter = (SemanticContext.Variable)key.operation();
      ambiguous.add(new VarVersionPair(parameter.index(), parameter.version()));
    }
    candidates.forEach((pair, name) -> {
      if (!ambiguous.contains(pair)) variables.setSemanticName(pair, name);
    });
  }

  private void addLiteral(Field field, ConstExprent constant, SemanticMappings.BitFieldPart part) {
    SemanticMappings.Value value = SemanticMappings.bitFieldConstant(field.owner(), field.entry(), part);
    // JVM shift counts are masked, and masks can be signed int literals.
    // Name only an exactly equal constant after its original storage conversion.
    if (literal(constant) == (constant.getValue() instanceof Long ? value.value() : (int)value.value()))
      literals.computeIfAbsent(constant, ignored -> new HashSet<>()).add(value);
  }

  /** Called by the ordinary consumer traversal at a mapped packing boundary. */
  void packing(Exprent expression, String domain) {
    Field agreed = null;
    List<SemanticBitAccess.Operand> operands = List.of();
    for (BitFieldEntry entry : analysis.mappings.bitFields(domain)) {
      SemanticBitAccess.Packing packing = SemanticBitAccess.packing(expression, entry.shift(), entry.bits());
      if (packing == null || packing.operands().isEmpty()) continue;
      if (entry.name() == null) return;
      Field candidate = new Field(domain, entry);
      if (agreed != null && !agreed.name().equals(candidate.name())) return;
      agreed = candidate;
      operands = packing.operands();
    }
    if (agreed == null) return;
    for (SemanticBitAccess.Operand operand : operands) addLiteral(agreed, operand.constant(), operand.part());
  }

  private Field field(Exprent at, SemanticBitAccess.Extraction extraction) {
    SemanticFacts source = analysis.resolveFacts(at, analysis.graph.layoutFacts(extraction.source()), true);
    if (source.unknown() || source.domains().isEmpty()) return null;
    Field agreed = null;
    for (String domain : source.domains()) {
      boolean found = false;
      for (BitFieldEntry entry : analysis.mappings.bitFields(domain)) {
        if (entry.shift() != extraction.shift() || entry.bits() != extraction.bits() || entry.signed() != extraction.signed()) continue;
        SemanticContext.Key selector = SemanticContext.operation(FunctionExprent.FunctionType.AND,
          analysis.context.key(extraction.source()), SemanticContext.constant(entry.selectorMask()));
        if (entry.selectorMask() != 0 && analysis.context.excludes(at, selector, entry.selectorValue())) continue;
        if (entry.name() == null) return null;
        Field candidate = new Field(domain, entry);
        if (agreed != null && !agreed.name().equals(candidate.name())) return null;
        agreed = candidate;
        found = true;
      }
      if (!found) return null;
    }
    return agreed;
  }

  private SemanticName name(Exprent expression) {
    if (names.containsKey(expression)) return names.get(expression);
    // A cycle alone cannot establish a name. Every reaching definition must agree.
    names.put(expression, null);
    SemanticName result = computeName(expression);
    names.put(expression, result);
    return result;
  }

  private SemanticName computeName(Exprent expression) {
    Field field = fields.get(expression);
    if (field != null) return field.name();
    if (expression instanceof AssignmentExprent assignment)
      return assignment.getCondType() == null ? name(assignment.getRight()) : null;
    if (expression instanceof VarExprent) {
      SemanticName agreed = null;
      for (SemanticLocalFlow.Definition source : analysis.locals.sources(expression)) {
        SemanticName candidate = source.value == null ? null : name(source.value);
        if (candidate == null || agreed != null && !agreed.equals(candidate)) return null;
        agreed = candidate;
      }
      return agreed;
    }
    if (expression instanceof FunctionExprent function) {
      if (isValuePreservingCast(function)) return name(function.getLstOperands().get(0));
      if (function.getFuncType() == FunctionExprent.FunctionType.TERNARY) {
        SemanticName left = name(function.getLstOperands().get(1));
        return Objects.equals(left, name(function.getLstOperands().get(2))) ? left : null;
      }
    }
    if (primitiveDescriptor(expression.getExprType()) == null) return null;
    SemanticFacts facts = analysis.graph.layoutFacts(expression);
    String domain = facts.unknown() ? null : unique(facts.domains());
    if (domain == null || !"packed".equals(analysis.mappings.domainKind(domain))
      || analysis.mappings.bitFields(domain).stream().noneMatch(entry -> entry.name() != null)) return null;
    String simple = domain.substring(domain.lastIndexOf('/') + 1);
    return new SemanticName(domain, "packed" + simple);
  }
}
