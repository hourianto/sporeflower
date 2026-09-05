// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.api.SemanticMappingData;
import org.jetbrains.java.decompiler.api.SemanticMappingData.*;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructField;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.FieldDescriptor;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.math.BigDecimal;

public final class SemanticMappings {
  public record MemberKey(String owner, String name, String desc) {}
  public record RecordLayout(String domain, int stride, int offset, boolean planes) {}
  public record CallBinding(int offset, MemberKey callee, String domain) {}
  public record ContainerSemantics(String elements, String keys, String values) {}
  public record Condition(int parameter, Long equalsValue, String domain, Long notEqualsValue, boolean otherwise) {}
  public record SlotSource(int parameter, int slot, int dimension) {}
  public record ArraySemantics(Map<Integer, String> indexDomains, Map<Integer, String> slotDomains, String elementDomain,
                               Map<Integer, RecordLayout> records) {
    public ArraySemantics {
      indexDomains = Map.copyOf(indexDomains);
      slotDomains = Map.copyOf(slotDomains);
      records = Map.copyOf(records);
    }

    public boolean isEmpty() {
      return indexDomains.isEmpty() && slotDomains.isEmpty() && records.isEmpty() && elementDomain == null;
    }

    public ArraySemantics element() {
      return new ArraySemantics(shift(indexDomains), shift(slotDomains), elementDomain, shift(records));
    }

    public ArraySemantics withElementDomain(String domain) {
      return new ArraySemantics(indexDomains, slotDomains, domain, records);
    }

    private static <T> Map<Integer, T> shift(Map<Integer, T> domains) {
      Map<Integer, T> shifted = new LinkedHashMap<>();
      domains.forEach((dimension, domain) -> {
        if (dimension > 0) shifted.put(dimension - 1, domain);
      });
      return shifted;
    }
  }
  public record Value(String domain, long value, String owner, String name, String desc, int access,
                      boolean synthetic, String elementDomain) {}
  public record SymbolicExpression(List<Value> values, Long residual, boolean complemented, boolean longLiteral) {
    public SymbolicExpression {
      values = List.copyOf(values);
    }
  }
  private record BindingTarget(String kind, MemberKey member, int index) {
    private static BindingTarget field(MemberKey member) {
      return new BindingTarget("field", member, -1);
    }

    private static BindingTarget returns(MemberKey member) {
      return new BindingTarget("return", member, -1);
    }

    private static BindingTarget parameter(MemberKey member, int index) {
      return new BindingTarget("parameter", member, index);
    }

    private BindingTarget withMember(MemberKey mapped) {
      return new BindingTarget(kind, mapped, index);
    }

    private boolean isField() {
      return "field".equals(kind);
    }
  }
  private record MaskedValue(Value value, long mask) {}
  private record FlagCover(List<Value> values, long residual) {}
  private final Map<String, String> domainKinds = new LinkedHashMap<>();
  private final Map<String, List<Long>> exclusiveMasks = new LinkedHashMap<>();
  private final Map<String, Map<Long, Value>> values = new LinkedHashMap<>();
  private final Map<BindingTarget, String> scalarBindings = new LinkedHashMap<>();
  private final Map<BindingTarget, ArraySemantics> arrayBindings = new LinkedHashMap<>();
  private final Map<BindingTarget, Integer> returnDomainSources = new LinkedHashMap<>();
  private final Map<MemberKey, List<CallBinding>> callBindings = new LinkedHashMap<>();
  private final Map<String, List<BitFieldEntry>> bitFields = new LinkedHashMap<>();
  private final Map<String, NumberFormatEntry> formats = new LinkedHashMap<>();
  private final Map<String, Map<String, Value>> strings = new LinkedHashMap<>();
  private final Map<BindingTarget, SlotSource> slotSources = new LinkedHashMap<>();
  private final Map<BindingTarget, Optional<SlotSource>> slotSourceCache = new ConcurrentHashMap<>();
  private final Map<BindingTarget, List<Condition>> conditions = new LinkedHashMap<>();
  private final Map<BindingTarget, ContainerSemantics> containers = new LinkedHashMap<>();
  private final Map<BindingTarget, Optional<List<Condition>>> conditionCache = new ConcurrentHashMap<>();
  private final Map<BindingTarget, Optional<ContainerSemantics>> containerCache = new ConcurrentHashMap<>();
  // Most member expressions have no explicit semantic binding. Cache misses as
  // well as hits so repeated uses do not keep walking the same class hierarchy.
  private final Map<BindingTarget, Optional<String>> scalarBindingCache = new ConcurrentHashMap<>();
  private final Map<BindingTarget, Optional<ArraySemantics>> arrayBindingCache = new ConcurrentHashMap<>();
  private final Map<BindingTarget, Optional<Integer>> returnDomainSourceCache = new ConcurrentHashMap<>();

  private SemanticMappings(SemanticMappingData root) {
    for (DomainEntry entry : entries(root.domains())) {
      domainKinds.put(entry.id(), entry.kind());
      exclusiveMasks.put(entry.id(), List.copyOf(entries(entry.exclusiveMasks())));
      bitFields.put(entry.id(), List.copyOf(entries(entry.bitFields())));
      if (entry.format() != null) formats.put(entry.id(), entry.format());
    }
    for (StringValueEntry value : entries(root.stringValues())) {
      strings.computeIfAbsent(value.domain(), ignored -> new LinkedHashMap<>()).put(value.value(),
        new Value(value.domain(), 0, value.owner(), value.name(), "Ljava/lang/String;", value.access(), value.synthetic(), null));
    }
    for (ConditionalBindingEntry entry : entries(root.conditionalBindings())) {
      conditions.computeIfAbsent(target(entry.target()), ignored -> new ArrayList<>())
        .add(new Condition(entry.parameter(), entry.equalsValue(), entry.domain(), entry.notEqualsValue(), entry.otherwise()));
    }
    for (SlotDomainSourceEntry entry : entries(root.slotDomainSources())) {
      slotSources.put(target(entry.target()), new SlotSource(entry.sourceParameter(), entry.slot(), entry.dimension()));
    }
    for (ContainerBindingEntry entry : entries(root.containerBindings())) {
      containers.put(target(entry.target()), new ContainerSemantics(entry.elements(), entry.keys(), entry.values()));
    }
    for (ValueEntry entry : entries(root.values())) {
      Value value = new Value(
        entry.domain(), entry.value(), entry.owner(), entry.name(), entry.desc(), entry.access(),
        entry.synthetic(), entry.elementDomain()
      );
      values.computeIfAbsent(entry.domain(), ignored -> new LinkedHashMap<>()).put(value.value(), value);
    }

    for (ScalarBindingEntry entry : entries(root.scalarBindings())) {
      scalarBindings.put(target(entry.target()), entry.domain());
    }

    for (ArrayBindingEntry entry : entries(root.arrayBindings())) {
      Map<Integer, RecordLayout> records = new LinkedHashMap<>();
      for (RecordLayoutEntry layout : entries(entry.records())) {
        if (layout.stride() <= 0 || layout.offset() < 0 || layout.dimension() < 0) {
          throw new IllegalArgumentException("Invalid semantic record layout: " + layout);
        }
        records.put(layout.dimension(), new RecordLayout(layout.domain(), layout.stride(), layout.offset(), layout.planes()));
      }
      arrayBindings.put(target(entry.target()), new ArraySemantics(
        dimensionDomains(entry.indexDomains()),
        dimensionDomains(entry.slotDomains()),
        entry.elementDomain(), records
      ));
    }

    for (ReturnDomainSourceEntry entry : entries(root.returnDomainSources())) {
      returnDomainSources.put(target(entry.target()), entry.sourceParameter());
    }

    for (CallBindingEntry entry : entries(root.callBindings())) {
      callBindings.computeIfAbsent(target(entry.method()).member(), ignored -> new ArrayList<>())
        .add(new CallBinding(entry.offset(), target(entry.callee()).member(), entry.domain()));
    }
  }

  public static SemanticMappings load(Path path) throws IOException {
    return fromData(SemanticMappingData.read(path));
  }

  public static SemanticMappings fromData(SemanticMappingData root) {
    return new SemanticMappings(root);
  }

  public List<BitFieldEntry> bitFields(String domain) {
    return bitFields.getOrDefault(domain, List.of());
  }

  public List<Condition> conditions(MemberKey method, int parameter) {
    List<Condition> result = inheritedBinding(conditions, conditionCache,
      parameter < 0 ? BindingTarget.returns(method) : BindingTarget.parameter(method, parameter));
    return result == null ? List.of() : result;
  }

  public SlotSource slotSource(MemberKey method, int parameter) {
    return inheritedBinding(slotSources, slotSourceCache,
      parameter < 0 ? BindingTarget.returns(method) : BindingTarget.parameter(method, parameter));
  }

  public ContainerSemantics container(MemberKey member, String kind, int parameter) {
    return inheritedBinding(containers, containerCache, new BindingTarget(kind, member, parameter));
  }

  public Value stringValue(String domain, String text, String currentOwner) {
    Value value = strings.getOrDefault(domain, Map.of()).get(text);
    return value != null && isAccessible(value, currentOwner) ? value : null;
  }

  public String formattedLiteral(String domain, long value, boolean wide) {
    NumberFormatEntry format = formats.get(domain);
    if (format == null) return null;
    // Zero and standard integer extrema are clearer in their ordinary form,
    // especially in sign tests and min/max searches. Keep RGB/ARGB masks intact.
    if ("fixed".equals(format.kind()) && (value == 0 || (wide
        ? value == Long.MIN_VALUE || value == Long.MAX_VALUE
        : value == Integer.MIN_VALUE || value == Integer.MAX_VALUE))) return null;
    int digits = "rgb".equals(format.kind()) ? 6 : "argb".equals(format.kind()) ? 8 : 1;
    String hex = Long.toHexString(wide ? value : value & 0xffffffffL).toUpperCase(java.util.Locale.ROOT);
    String result = "0x" + "0".repeat(Math.max(0, digits - hex.length())) + hex + (wide ? "L" : "");
    if ("fixed".equals(format.kind())) {
      // Render the original integer exactly; the decoded quantity is only a
      // comment. No floating arithmetic, rounding, or overflow is introduced.
      String decoded = BigDecimal.valueOf(value).divide(BigDecimal.valueOf(2).pow(format.fractionBits()))
        .stripTrailingZeros().toPlainString();
      result += " /* Q" + format.fractionBits() + ": " + decoded + " */";
    }
    return result;
  }

  public List<CallBinding> callBindings(MemberKey method) {
    // A bytecode offset belongs only to its exact containing method. Overrides
    // and inherited methods must never borrow a call site's contract.
    return callBindings.getOrDefault(namedMember(method), List.of());
  }

  public String fieldDomain(MemberKey field) {
    return inheritedBinding(scalarBindings, scalarBindingCache, BindingTarget.field(field));
  }

  public String returnDomain(MemberKey method) {
    return inheritedBinding(scalarBindings, scalarBindingCache, BindingTarget.returns(method));
  }

  public String parameterDomain(MemberKey method, int index) {
    return inheritedBinding(scalarBindings, scalarBindingCache, BindingTarget.parameter(method, index));
  }

  public Integer returnDomainSource(MemberKey method) {
    return inheritedBinding(returnDomainSources, returnDomainSourceCache, BindingTarget.returns(method));
  }

  public ArraySemantics fieldArraySemantics(MemberKey field) {
    return inheritedBinding(arrayBindings, arrayBindingCache, BindingTarget.field(field));
  }

  public ArraySemantics returnArraySemantics(MemberKey method) {
    return inheritedBinding(arrayBindings, arrayBindingCache, BindingTarget.returns(method));
  }

  public ArraySemantics parameterArraySemantics(MemberKey method, int parameter) {
    return inheritedBinding(arrayBindings, arrayBindingCache, BindingTarget.parameter(method, parameter));
  }

  public boolean hasParameterSemantics(MemberKey method, int parameter) {
    return parameterDomain(method, parameter) != null || parameterArraySemantics(method, parameter) != null
      || !conditions(method, parameter).isEmpty() || slotSource(method, parameter) != null || container(method, "parameter", parameter) != null;
  }

  public String namedOwner(String owner) {
    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor == null) return owner;
    String mapped = interceptor.getName(owner);
    return mapped == null ? owner : mapped;
  }

  public MemberKey namedMember(MemberKey member) {
    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor == null) return member;

    String mappedMember = interceptor.getName(member.owner() + ' ' + member.name() + ' ' + member.desc());
    if (mappedMember != null) {
      String[] parts = mappedMember.split(" ", 3);
      if (parts.length == 3) return new MemberKey(parts[0], parts[1], parts[2]);
    }

    String descriptor = remapDescriptor(member.desc(), interceptor);
    return new MemberKey(namedOwner(member.owner()), member.name(), descriptor);
  }

  public String domainKind(String domain) {
    return domainKinds.get(domain);
  }

  public Value value(String domain, long literal, String currentOwner) {
    Value value = values.getOrDefault(domain, Map.of()).get(literal);
    return value != null && isAccessible(value, currentOwner) ? value : null;
  }

  Set<String> slotElementDomains(String domain) {
    Set<String> domains = new HashSet<>();
    for (Value value : values.getOrDefault(domain, Map.of()).values()) {
      if (value.elementDomain() != null) domains.add(value.elementDomain());
    }
    return domains;
  }

  public boolean isRangeBoundary(String domain, long literal, boolean lowerBound) {
    if (!"value".equals(domainKind(domain))) return false;
    Set<Long> known = values.getOrDefault(domain, Map.of()).keySet();
    return !known.isEmpty() && known.stream().allMatch(value -> lowerBound ? value >= literal : value <= literal);
  }

  public boolean fitsIntegralType(String domain, String descriptor) {
    Map<Long, Value> known = values.getOrDefault(domain, Map.of());
    if (known.isEmpty()) return false;
    boolean flags = "flags".equals(domainKind(domain));
    return known.keySet().stream().allMatch(value -> switch (descriptor) {
      case "B" -> value >= Byte.MIN_VALUE && value <= (flags ? 255 : Byte.MAX_VALUE);
      case "S" -> value >= Short.MIN_VALUE && value <= (flags ? 65535 : Short.MAX_VALUE);
      case "C" -> value >= (flags ? Short.MIN_VALUE : Character.MIN_VALUE) && value <= Character.MAX_VALUE;
      case "I" -> value >= Integer.MIN_VALUE && value <= (flags ? 0xffffffffL : Integer.MAX_VALUE);
      case "J" -> true;
      default -> false;
    });
  }

  public SymbolicExpression symbolicExpression(String domain, long literal, String currentOwner, int requestedWidth) {
    Value exact = value(domain, literal, currentOwner);
    if (exact != null) return new SymbolicExpression(List.of(exact), null, false, "J".equals(exact.desc()));
    if (!"flags".equals(domainKind(domain))) return null;

    List<Value> domainValues = values.getOrDefault(domain, Map.of()).values().stream()
      .filter(value -> isAccessible(value, currentOwner))
      .toList();
    int width = switch (requestedWidth) {
      case 8, 16, 32, 64 -> requestedWidth;
      default -> flagWidth(domainValues);
    };
    long widthMask = widthMask(width);
    long target = literal & widthMask;
    if (target == 0 || target == widthMask) return null;

    List<Long> exclusive = exclusiveMasks.getOrDefault(domain, List.of());
    // A selector such as TextField's low constraint bits is one enum value,
    // not independently combinable bits. Unknown selectors remain residuals.
    List<Value> positiveValues = domainValues.stream().filter(value -> exclusive.stream().allMatch(mask ->
      (value.value() & mask & widthMask) == 0 || (value.value() & mask & widthMask) == (target & mask))).toList();
    FlagCover positive = coverFlags(positiveValues, target, widthMask);
    // Complements of independent flags remain useful (e.g. ~PASSWORD).
    // Complementing an enum selector would name a different set of modes.
    List<Value> negativeValues = domainValues.stream().filter(value -> exclusive.stream().allMatch(mask ->
      (value.value() & mask & widthMask) == 0)).toList();
    FlagCover negative = coverFlags(negativeValues, (~target) & widthMask, widthMask);
    int positiveTerms = positive.values().size() + (positive.residual() == 0 ? 0 : 1);
    boolean useNegative = !negative.values().isEmpty()
      && negative.residual() == 0
      && (positive.values().isEmpty() || negative.values().size() < positiveTerms);

    if (useNegative) {
      return new SymbolicExpression(negative.values(), null, true, width == 64);
    }
    if (positive.values().isEmpty()) return null;
    Long residual = positive.residual() == 0 ? null : signedValue(positive.residual(), width, widthMask);
    return new SymbolicExpression(positive.values(), residual, false, width == 64);
  }

  private static int flagWidth(List<Value> values) {
    int width = 0;
    for (Value value : values) {
      width = Math.max(width, switch (value.desc()) {
        case "B" -> 8;
        case "S", "C" -> 16;
        case "I" -> 32;
        case "J" -> 64;
        default -> 0;
      });
    }
    return width == 0 ? 32 : width;
  }

  private static long widthMask(int width) {
    return width == 64 ? -1L : (1L << width) - 1;
  }

  private static long signedValue(long value, int width, long mask) {
    if (width == 64) return value;
    long signBit = 1L << (width - 1);
    return (value & signBit) == 0 ? value : value | ~mask;
  }

  private static FlagCover coverFlags(List<Value> values, long target, long widthMask) {
    Comparator<Value> stableOrder = Comparator.comparing(Value::owner)
      .thenComparing(Value::name)
      .thenComparing(Value::desc);
    Map<Long, Value> byMask = new LinkedHashMap<>();
    values.stream().sorted(stableOrder).forEach(value -> {
      long mask = value.value() & widthMask;
      if (mask != 0 && (mask & ~target) == 0) byMask.putIfAbsent(mask, value);
    });

    List<MaskedValue> candidates = byMask.entrySet().stream()
      .filter(entry -> byMask.keySet().stream().noneMatch(other -> !other.equals(entry.getKey()) && (entry.getKey() & other) == entry.getKey()))
      .map(entry -> new MaskedValue(entry.getValue(), entry.getKey()))
      .sorted(Comparator.<MaskedValue>comparingInt(value -> Long.bitCount(value.mask())).reversed()
        .thenComparing(value -> value.value().owner())
        .thenComparing(value -> value.value().name())
        .thenComparing(value -> value.value().desc()))
      .toList();
    long coverable = 0;
    for (MaskedValue candidate : candidates) coverable |= candidate.mask();
    if (coverable == 0) return new FlagCover(List.of(), target);

    CoverSearch search = new CoverSearch(candidates, coverable);
    search.run(0, new ArrayList<>());
    return new FlagCover(search.best == null ? List.of() : search.best, target & ~coverable);
  }

  private static final class CoverSearch {
    private final List<MaskedValue> candidates;
    private final long target;
    private final Map<Long, Integer> depths = new HashMap<>();
    private List<Value> best;

    private CoverSearch(List<MaskedValue> candidates, long target) {
      this.candidates = candidates;
      this.target = target;
    }

    private void run(long covered, List<Value> chosen) {
      if (covered == target) {
        if (best == null || chosen.size() < best.size()) best = List.copyOf(chosen);
        return;
      }
      if (best != null && chosen.size() >= best.size()) return;
      Integer previousDepth = depths.putIfAbsent(covered, chosen.size());
      if (previousDepth != null && previousDepth <= chosen.size()) return;
      depths.put(covered, chosen.size());

      long missingBit = Long.lowestOneBit(target & ~covered);
      for (MaskedValue candidate : candidates) {
        if ((candidate.mask() & missingBit) == 0) continue;
        long next = covered | candidate.mask();
        if (next == covered) continue;
        chosen.add(candidate.value());
        run(next, chosen);
        chosen.remove(chosen.size() - 1);
      }
    }
  }

  public List<IContextSource.OutputClass> syntheticSources() {
    List<IContextSource.OutputClass> result = new ArrayList<>();
    for (Map<Long, Value> domainValues : values.values()) {
      List<Value> synthetic = domainValues.values().stream().filter(Value::synthetic).toList();
      if (synthetic.isEmpty()) continue;

      String owner = synthetic.get(0).owner();
      int slash = owner.lastIndexOf('/');
      String packageName = slash < 0 ? "" : owner.substring(0, slash).replace('/', '.');
      String simpleName = slash < 0 ? owner : owner.substring(slash + 1);
      StringBuilder source = new StringBuilder();
      if (!packageName.isEmpty()) source.append("package ").append(packageName).append(";\n\n");
      source.append("// Generated from semantic mappings; not present in the input JAR.\n");
      source.append("public interface ").append(simpleName).append(" {\n");
      synthetic.stream().sorted(Comparator.comparingLong(Value::value).thenComparing(Value::name)).forEach(value ->
        source.append("   ").append(javaType(value.desc())).append(' ').append(value.name())
          .append(" = ").append(javaLiteral(value)).append(";\n")
      );
      source.append("}\n");
      result.add(new IContextSource.OutputClass(owner, owner + ".java", source.toString()));
    }
    for (Map<String, Value> domainValues : strings.values()) {
      List<Map.Entry<String, Value>> synthetic = domainValues.entrySet().stream().filter(entry -> entry.getValue().synthetic()).toList();
      if (synthetic.isEmpty()) continue;
      String owner = synthetic.get(0).getValue().owner();
      int slash = owner.lastIndexOf('/');
      StringBuilder source = new StringBuilder();
      if (slash >= 0) source.append("package ").append(owner.substring(0, slash).replace('/', '.')).append(";\n\n");
      source.append("// Generated from semantic mappings; not present in the input JAR.\npublic interface ")
        .append(owner.substring(slash + 1)).append(" {\n");
      synthetic.stream().sorted(Comparator.comparing(entry -> entry.getValue().name())).forEach(entry ->
        source.append("   String ").append(entry.getValue().name()).append(" = ")
          .append(new com.google.gson.Gson().toJson(entry.getKey())).append(";\n"));
      source.append("}\n");
      result.add(new IContextSource.OutputClass(owner, owner + ".java", source.toString()));
    }
    return List.copyOf(result);
  }

  private <T> T inheritedBinding(Map<BindingTarget, T> bindings, Map<BindingTarget, Optional<T>> cache,
                                 BindingTarget requested) {
    return cache.computeIfAbsent(requested, key -> Optional.ofNullable(resolveBinding(bindings, key))).orElse(null);
  }

  private <T> T resolveBinding(Map<BindingTarget, T> bindings, BindingTarget requested) {
    BindingTarget normalized = requested.withMember(namedMember(requested.member()));
    T direct = bindings.get(normalized);
    if (direct != null) return direct;
    // An explicit binding of another shape on the same declaration replaces
    // inherited semantics rather than accidentally combining with them.
    if (hasDirectBinding(normalized)) return null;

    Set<T> inherited = new LinkedHashSet<>();
    String owner = originalOwner(normalized.member().owner());
    if (normalized.isField()) {
      collectFieldBindings(bindings, normalized, owner, new HashSet<>(), inherited);
    }
    else {
      collectMethodBindings(bindings, normalized, owner, new HashSet<>(), inherited);
    }
    return inherited.size() == 1 ? inherited.iterator().next() : null;
  }

  private boolean hasDirectBinding(BindingTarget target) {
    return scalarBindings.containsKey(target) || arrayBindings.containsKey(target) || returnDomainSources.containsKey(target)
      || conditions.containsKey(target) || containers.containsKey(target) || slotSources.containsKey(target);
  }

  private <T> void collectFieldBindings(Map<BindingTarget, T> bindings, BindingTarget requested,
                                        String owner, Set<String> seen, Set<T> found) {
    StructClass cl = resolveClass(owner);
    if (cl == null || !seen.add(cl.qualifiedName)) return;

    boolean declared = false;
    for (StructField field : cl.getFields()) {
      MemberKey declaration = new MemberKey(cl.qualifiedName, field.getName(), field.getDescriptor());
      if (matches(requested.member(), declaration)) {
        declared = true;
        addDeclaredBinding(bindings, requested, declaration, found);
      }
    }
    // Fields are hidden, not overridden. Once a declaration is found, an
    // unannotated field must not inherit a same-named ancestor's meaning.
    if (declared) return;

    if (cl.superClass != null) {
      collectFieldBindings(bindings, requested, cl.superClass.getString(), seen, found);
    }
    for (String iface : cl.getInterfaceNames()) {
      collectFieldBindings(bindings, requested, iface, seen, found);
    }
  }

  private <T> void collectMethodBindings(Map<BindingTarget, T> bindings, BindingTarget requested,
                                         String owner, Set<String> seen, Set<T> found) {
    StructClass cl = resolveClass(owner);
    if (cl == null || !seen.add(cl.qualifiedName)) return;

    boolean declared = false;
    for (StructMethod method : cl.getMethods()) {
      MemberKey declaration = new MemberKey(cl.qualifiedName, method.getName(), method.getDescriptor());
      if (!matches(requested.member(), declaration)) continue;
      declared = true;
      List<MemberKey> candidates = new ArrayList<>();
      candidates.add(declaration);
      if (SourceMethodSemantics.canParticipateInOverride(method)) {
        for (SourceMethodSemantics.InheritedMethod inherited : SourceMethodSemantics.findOverriddenMethods(
          DecompilerContext.getStructContext(), cl, method
        )) {
          StructMethod inheritedMethod = inherited.method();
          candidates.add(new MemberKey(
            inherited.ownerClass().qualifiedName,
            inheritedMethod.getName(),
            inheritedMethod.getDescriptor()
          ));
        }
      }
      // A nearer explicit contract replaces an older one, including a change
      // from a fixed return domain to a parameter-derived return. Unrelated
      // interfaces still contribute competing candidates and remain ambiguous.
      List<MemberKey> bound = candidates.stream()
        .filter(candidate -> hasDirectBinding(requested.withMember(namedMember(candidate)))).toList();
      for (MemberKey candidate : bound) {
        boolean shadowed = bound.stream().anyMatch(other -> !other.owner().equals(candidate.owner())
          && SourceMethodSemantics.isSubtype(DecompilerContext.getStructContext(), other.owner(), candidate.owner()));
        if (!shadowed) {
          found.add(bindings.get(requested.withMember(namedMember(candidate))));
        }
      }
    }
    if (declared) return;

    if (cl.superClass != null) {
      collectMethodBindings(bindings, requested, cl.superClass.getString(), seen, found);
    }
    for (String iface : cl.getInterfaceNames()) {
      collectMethodBindings(bindings, requested, iface, seen, found);
    }
  }

  private boolean matches(MemberKey requested, MemberKey declaration) {
    MemberKey namedDeclaration = namedMember(declaration);
    return namedDeclaration.name().equals(requested.name()) && namedDeclaration.desc().equals(requested.desc());
  }

  private <T> void addDeclaredBinding(Map<BindingTarget, T> bindings, BindingTarget requested,
                                      MemberKey declaration, Set<T> found) {
    MemberKey namedDeclaration = namedMember(declaration);
    T value = bindings.get(requested.withMember(namedDeclaration));
    if (value != null) found.add(value);
  }

  private StructClass resolveClass(String owner) {
    StructClass cl = DecompilerContext.getStructContext().getClass(owner);
    if (cl != null) return cl;
    String original = originalOwner(owner);
    if (!original.equals(owner) && (cl = DecompilerContext.getStructContext().getClass(original)) != null) return cl;
    String named = namedOwner(owner);
    return named.equals(owner) ? null : DecompilerContext.getStructContext().getClass(named);
  }

  private String originalOwner(String owner) {
    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor == null) return owner;
    String original = interceptor.getOldName(owner);
    return original == null ? owner : original;
  }

  private static String remapDescriptor(String descriptor, PoolInterceptor interceptor) {
    String mapped = descriptor.startsWith("(")
      ? MethodDescriptor.parseDescriptor(descriptor).buildNewDescriptor(interceptor::getName)
      : FieldDescriptor.parseDescriptor(descriptor).buildNewDescriptor(interceptor::getName);
    return mapped == null ? descriptor : mapped;
  }

  private boolean isAccessible(Value value, String currentOwner) {
    if (value.synthetic() || (value.access() & CodeConstants.ACC_PUBLIC) != 0) return true;
    if ((value.access() & CodeConstants.ACC_PRIVATE) != 0) return value.owner().equals(currentOwner);
    if (packageName(value.owner()).equals(packageName(currentOwner))) return true;
    return (value.access() & CodeConstants.ACC_PROTECTED) != 0 &&
      DecompilerContext.getStructContext().instanceOf(originalOwner(currentOwner), originalOwner(value.owner()));
  }

  private static String packageName(String owner) {
    int slash = owner.lastIndexOf('/');
    return slash < 0 ? "" : owner.substring(0, slash);
  }

  private static String javaType(String desc) {
    return switch (desc) {
      case "B" -> "byte";
      case "S" -> "short";
      case "C" -> "char";
      case "I" -> "int";
      case "J" -> "long";
      default -> throw new IllegalArgumentException("Unsupported semantic constant descriptor: " + desc);
    };
  }

  private static String javaLiteral(Value value) {
    return switch (value.desc()) {
      case "B" -> "(byte)" + value.value();
      case "S" -> "(short)" + value.value();
      case "C" -> "(char)" + value.value();
      case "J" -> value.value() + "L";
      default -> Long.toString(value.value());
    };
  }

  private static BindingTarget target(TargetEntry entry) {
    MemberKey member = new MemberKey(entry.owner(), entry.name(), entry.desc());
    return switch (entry.kind()) {
      case "field" -> BindingTarget.field(member);
      case "return" -> BindingTarget.returns(member);
      case "parameter" -> BindingTarget.parameter(member, entry.index() == null ? -1 : entry.index());
      default -> throw new IllegalArgumentException("Unsupported semantic target kind: " + entry.kind());
    };
  }

  private static Map<Integer, String> dimensionDomains(List<DimensionEntry> entries) {
    Map<Integer, String> result = new LinkedHashMap<>();
    for (DimensionEntry entry : entries(entries)) result.put(entry.dimension(), entry.domain());
    return result;
  }

  private static <T> List<T> entries(List<T> values) {
    return values == null ? List.of() : values;
  }
}
