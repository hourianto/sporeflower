// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.api.SemanticMappingData;
import org.jetbrains.java.decompiler.api.SemanticMappingData.*;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructField;
import org.jetbrains.java.decompiler.struct.StructMethod;

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
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.math.BigDecimal;

public final class SemanticMappings {
  public enum BitFieldPart { MASK, VALUE_MASK, SHIFT }
  public record MemberKey(String owner, String name, String desc) {}
  public record RecordLayout(String domain, int stride, int offset, boolean planes) {}
  public record CallBinding(int offset, MemberKey callee, String domain, Integer parameter) {}
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

    /** Lift a row's shape without assigning a meaning to its containing row index. */
    public ArraySemantics outer() {
      return new ArraySemantics(raise(indexDomains), raise(slotDomains), elementDomain, raise(records));
    }

    private static <T> Map<Integer, T> raise(Map<Integer, T> values) {
      Map<Integer, T> result = new LinkedHashMap<>();
      values.forEach((dimension, value) -> result.put(dimension + 1, value));
      return result;
    }

    /** Conjoin compatible contracts, as distinct from joining alternative producers. */
    public ArraySemantics combine(ArraySemantics other) {
      Map<Integer, String> indexes = combine(indexDomains, other.indexDomains);
      Map<Integer, String> slots = combine(slotDomains, other.slotDomains);
      Map<Integer, RecordLayout> layouts = combine(records, other.records);
      if (indexes == null || slots == null || layouts == null
          || elementDomain != null && other.elementDomain != null && !elementDomain.equals(other.elementDomain)) return null;
      for (Integer dimension : indexes.keySet()) if (slots.containsKey(dimension) || layouts.containsKey(dimension)) return null;
      return new ArraySemantics(indexes, slots, elementDomain == null ? other.elementDomain : elementDomain, layouts);
    }

    private static <T> Map<Integer, T> combine(Map<Integer, T> left, Map<Integer, T> right) {
      Map<Integer, T> result = new LinkedHashMap<>(left);
      for (var entry : right.entrySet()) {
        T previous = result.putIfAbsent(entry.getKey(), entry.getValue());
        if (previous != null && !previous.equals(entry.getValue())) return null;
      }
      return result;
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
  private final Map<BindingTarget, SemanticContract> contracts = new LinkedHashMap<>();
  private final Map<BindingTarget, SemanticContract> contractCache = new ConcurrentHashMap<>();
  private final Map<MemberKey, List<CallBinding>> callBindings = new LinkedHashMap<>();
  private final Map<MemberKey, Map<Integer, ClassNameLiteralEntry>> classNameLiterals = new LinkedHashMap<>();
  private final boolean resolvedClassNames;
  private final Map<String, List<BitFieldEntry>> bitFields = new LinkedHashMap<>();
  private final boolean namedBitFields;
  private final Map<String, NumberFormatEntry> formats = new LinkedHashMap<>();
  private final Map<String, Map<String, Value>> strings = new LinkedHashMap<>();
  // Queries run after renaming, and this object belongs to one decompilation.
  // Resolve declarations once, independently of parameter positions and binding
  // kinds; otherwise every cache miss scans and renames the whole class again.
  private final Map<StructClass, ClassMembers> classMembers = new ConcurrentHashMap<>();
  private final Map<StructMethod, List<MemberKey>> methodCandidates = new ConcurrentHashMap<>();

  private record MemberSignature(String name, String desc) {
    private MemberSignature(MemberKey member) {
      this(member.name(), member.desc());
    }
  }

  private record ClassMembers(Map<MemberSignature, List<StructField>> fields,
                              Map<MemberSignature, List<StructMethod>> methods) {}

  private SemanticMappings(SemanticMappingData root) {
    resolvedClassNames = root.classNameLiterals() != null;
    for (ClassNameLiteralEntry entry : entries(root.classNameLiterals())) {
      classNameLiterals.computeIfAbsent(target(entry.target()).member(), ignored -> new LinkedHashMap<>()).put(entry.offset(), entry);
    }
    for (DomainEntry entry : entries(root.domains())) {
      domainKinds.put(entry.id(), entry.kind());
      exclusiveMasks.put(entry.id(), List.copyOf(entries(entry.exclusiveMasks())));
      bitFields.put(entry.id(), List.copyOf(entries(entry.bitFields())));
      if (entry.format() != null) formats.put(entry.id(), entry.format());
    }
    namedBitFields = bitFields.values().stream().flatMap(List::stream).anyMatch(field -> field.name() != null);
    for (StringValueEntry value : entries(root.stringValues())) {
      strings.computeIfAbsent(value.domain(), ignored -> new LinkedHashMap<>()).put(value.value(),
        new Value(value.domain(), 0, value.owner(), value.name(), "Ljava/lang/String;", value.access(), value.synthetic(), null));
    }
    for (ConditionalBindingEntry entry : entries(root.conditionalBindings())) {
      bind(entry.target(), contract -> contract.withCondition(
        new Condition(entry.parameter(), entry.equalsValue(), entry.domain(), entry.notEqualsValue(), entry.otherwise())));
    }
    for (SlotDomainSourceEntry entry : entries(root.slotDomainSources())) {
      bind(entry.target(), contract -> contract.withMeaning(new SemanticContract.Column(new SlotSource(entry.sourceParameter(), entry.slot(), entry.dimension()))));
    }
    for (ContainerBindingEntry entry : entries(root.containerBindings())) {
      bind(entry.target(), contract -> contract.withContainer(new ContainerSemantics(entry.elements(), entry.keys(), entry.values())));
    }
    for (ValueEntry entry : entries(root.values())) {
      Value value = new Value(
        entry.domain(), entry.value(), entry.owner(), entry.name(), entry.desc(), entry.access(),
        entry.synthetic(), entry.elementDomain()
      );
      values.computeIfAbsent(entry.domain(), ignored -> new LinkedHashMap<>()).put(value.value(), value);
    }

    for (ScalarBindingEntry entry : entries(root.scalarBindings())) {
      bind(entry.target(), contract -> contract.withMeaning(new SemanticContract.Fixed(entry.domain())));
    }

    for (ArrayBindingEntry entry : entries(root.arrayBindings())) {
      Map<Integer, RecordLayout> records = new LinkedHashMap<>();
      for (RecordLayoutEntry layout : entries(entry.records())) {
        if (layout.stride() <= 0 || layout.offset() < 0 || layout.dimension() < 0) {
          throw new IllegalArgumentException("Invalid semantic record layout: " + layout);
        }
        records.put(layout.dimension(), new RecordLayout(layout.domain(), layout.stride(), layout.offset(), layout.planes()));
      }
      bind(entry.target(), contract -> contract.withArray(new ArraySemantics(
        dimensionDomains(entry.indexDomains()),
        dimensionDomains(entry.slotDomains()),
        entry.elementDomain(), records
      )));
    }

    for (ReturnDomainSourceEntry entry : entries(root.returnDomainSources())) {
      bind(entry.target(), contract -> contract.withMeaning(new SemanticContract.Argument(entry.sourceParameter())));
    }

    for (CallBindingEntry entry : entries(root.callBindings())) {
      callBindings.computeIfAbsent(target(entry.method()).member(), ignored -> new ArrayList<>())
        .add(new CallBinding(entry.offset(), target(entry.callee()).member(), entry.domain(), entry.parameter()));
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

  boolean hasNamedBitFields() { return namedBitFields; }

  public static String bitFieldConstantName(String name, BitFieldPart part) {
    if (!javax.lang.model.SourceVersion.isIdentifier(name) || javax.lang.model.SourceVersion.isKeyword(name) || name.equals("_"))
      throw new IllegalArgumentException("Invalid packed field name: " + name);
    return name.replaceAll("([A-Z]+)([A-Z][a-z])", "$1_$2").replaceAll("([a-z0-9])([A-Z])", "$1_$2")
      .toUpperCase(java.util.Locale.ROOT) + "_" + part;
  }

  static Value bitFieldConstant(String owner, BitFieldEntry field, BitFieldPart part) {
    long mask = SemanticBitAccess.lowMask(field.bits());
    long value = switch (part) {
      case MASK -> mask << field.shift();
      case VALUE_MASK -> mask;
      case SHIFT -> field.shift();
    };
    // Keep the unsigned high bit positive where possible. A use in an int
    // expression gets an explicit narrowing cast; a long use must not sign-extend it.
    boolean wide = part != BitFieldPart.SHIFT && (part == BitFieldPart.MASK ? field.shift() + field.bits() : field.bits()) >= 32;
    return new Value(owner, value, owner, bitFieldConstantName(field.name(), part), wide ? "J" : "I",
      CodeConstants.ACC_PUBLIC | CodeConstants.ACC_STATIC | CodeConstants.ACC_FINAL, true, null);
  }

  SemanticContract contract(MemberKey member, String kind, int parameter) {
    BindingTarget target = new BindingTarget(kind, member, parameter);
    return contractCache.computeIfAbsent(target, this::resolveContract);
  }

  private void bind(TargetEntry target, java.util.function.UnaryOperator<SemanticContract> update) {
    contracts.compute(target(target), (key, previous) -> update.apply(previous == null ? SemanticContract.NONE : previous));
  }

  public List<Condition> conditions(MemberKey method, int parameter) {
    return contract(method, parameter < 0 ? "return" : "parameter", parameter).conditions();
  }

  public SlotSource slotSource(MemberKey method, int parameter) {
    return contract(method, parameter < 0 ? "return" : "parameter", parameter).column();
  }

  public ContainerSemantics container(MemberKey member, String kind, int parameter) {
    return contract(member, kind, parameter).container();
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
    if (("fixed".equals(format.kind()) || "scaled".equals(format.kind())) && (value == 0 || (wide
        ? value == Long.MIN_VALUE || value == Long.MAX_VALUE
        : value == Integer.MIN_VALUE || value == Integer.MAX_VALUE))) return null;
    if ("scaled".equals(format.kind())) {
      java.math.BigInteger numerator = java.math.BigInteger.valueOf(value);
      java.math.BigInteger denominator = java.math.BigInteger.valueOf(format.divisor());
      String decoded;
      try {
        decoded = new BigDecimal(numerator).divide(new BigDecimal(denominator)).stripTrailingZeros().toPlainString();
      } catch (ArithmeticException repeating) {
        // Non-terminating decimals stay exact rather than silently rounding.
        java.math.BigInteger gcd = numerator.gcd(denominator);
        decoded = numerator.divide(gcd) + "/" + denominator.divide(gcd);
      }
      return value + (wide ? "L" : "") + " /* /" + format.divisor() + ": " + decoded
        + (format.unit() == null ? "" : " " + format.unit()) + " */";
    }
    int digits = "rgb".equals(format.kind()) ? 6 : "argb".equals(format.kind()) ? 8 : 1;
    String hex = Long.toHexString(wide ? value : value & 0xffffffffL).toUpperCase(java.util.Locale.ROOT);
    String result = "0x" + "0".repeat(Math.max(0, digits - hex.length())) + hex + (wide ? "L" : "");
    if ("fixed".equals(format.kind())) {
      // Render the original integer exactly; the decoded quantity is only a
      // comment. No floating arithmetic, rounding, or overflow is introduced.
      String decoded = BigDecimal.valueOf(value).divide(BigDecimal.valueOf(2).pow(format.fractionBits()))
        .stripTrailingZeros().toPlainString();
      result += " /* Q" + format.fractionBits() + ": " + decoded + (format.unit() == null ? "" : " " + format.unit()) + " */";
    }
    return result;
  }

  public List<CallBinding> callBindings(MemberKey method) {
    // A bytecode offset belongs only to its exact containing method. Overrides
    // and inherited methods must never borrow a call site's contract.
    return callBindings.getOrDefault(method, List.of());
  }

  public boolean hasResolvedClassNames() { return resolvedClassNames; }

  public String classNameLiteral(MemberKey member, int offset, String original) {
    ClassNameLiteralEntry entry = classNameLiterals.getOrDefault(member, Map.of()).get(offset);
    return entry != null && entry.original().equals(original) ? entry.replacement() : original;
  }

  public String fieldDomain(MemberKey field) { return contract(field, "field", -1).domain(); }
  public String returnDomain(MemberKey method) { return contract(method, "return", -1).domain(); }
  public String parameterDomain(MemberKey method, int index) { return contract(method, "parameter", index).domain(); }
  public Integer returnDomainSource(MemberKey method) { return contract(method, "return", -1).argument(); }
  public ArraySemantics fieldArraySemantics(MemberKey field) { return contract(field, "field", -1).array(); }
  public ArraySemantics returnArraySemantics(MemberKey method) { return contract(method, "return", -1).array(); }
  public ArraySemantics parameterArraySemantics(MemberKey method, int parameter) { return contract(method, "parameter", parameter).array(); }
  public boolean hasParameterSemantics(MemberKey method, int parameter) { return !contract(method, "parameter", parameter).equals(SemanticContract.NONE); }

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
    Map<String, Map<String, Value>> numeric = new LinkedHashMap<>();
    for (Map<Long, Value> domain : values.values()) {
      for (Value value : domain.values()) {
        if (value.synthetic()) numeric.computeIfAbsent(value.owner(), ignored -> new LinkedHashMap<>()).put(value.name(), value);
      }
    }
    for (var domain : bitFields.entrySet()) {
      for (BitFieldEntry field : domain.getValue()) {
        if (field.name() == null) continue;
        for (BitFieldPart part : BitFieldPart.values()) {
          Value value = bitFieldConstant(domain.getKey(), field, part);
          Value previous = numeric.computeIfAbsent(value.owner(), ignored -> new LinkedHashMap<>()).putIfAbsent(value.name(), value);
          if (previous != null && !previous.equals(value))
            throw new IllegalArgumentException("Conflicting generated packed constant: " + value.owner() + "." + value.name());
        }
      }
    }
    List<IContextSource.OutputClass> result = new ArrayList<>();
    for (Map<String, Value> domainValues : numeric.values()) {
      List<Value> synthetic = new ArrayList<>(domainValues.values());

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

  private SemanticContract resolveContract(BindingTarget normalized) {
    SemanticContract direct = contracts.get(normalized);
    if (direct != null) return direct;
    // Resolve the declaration once for all facets. A nearer explicit contract
    // replaces the ancestor as a whole, including its expression for the meaning.
    Set<SemanticContract> inherited = new HashSet<>();
    for (BindingTarget target : findInheritedTargets(normalized)) {
      inherited.add(contracts.getOrDefault(target, SemanticContract.NONE));
    }
    return inherited.size() == 1 ? inherited.iterator().next() : SemanticContract.NONE;
  }

  private List<BindingTarget> findInheritedTargets(BindingTarget normalized) {
    Set<BindingTarget> inherited = new LinkedHashSet<>();
    String owner = normalized.member().owner();
    if (normalized.isField()) {
      collectFieldTargets(normalized, owner, new HashSet<>(), inherited);
    }
    else {
      collectMethodTargets(normalized, owner, new HashSet<>(), inherited);
    }
    return List.copyOf(inherited);
  }

  private void collectFieldTargets(BindingTarget requested, String owner, Set<String> seen, Set<BindingTarget> found) {
    StructClass cl = resolveClass(owner);
    if (cl == null || !seen.add(cl.qualifiedName)) return;

    List<StructField> declared = members(cl).fields().getOrDefault(new MemberSignature(requested.member()), List.of());
    for (StructField field : declared) {
      MemberKey declaration = new MemberKey(cl.qualifiedName, field.getName(), field.getDescriptor());
      found.add(requested.withMember(declaration));
    }
    // Fields are hidden, not overridden. Once a declaration is found, an
    // unannotated field must not inherit a same-named ancestor's meaning.
    if (!declared.isEmpty()) return;

    if (cl.superClass != null) {
      collectFieldTargets(requested, cl.superClass.getString(), seen, found);
    }
    for (String iface : cl.getInterfaceNames()) {
      collectFieldTargets(requested, iface, seen, found);
    }
  }

  private void collectMethodTargets(BindingTarget requested, String owner, Set<String> seen, Set<BindingTarget> found) {
    StructClass cl = resolveClass(owner);
    if (cl == null || !seen.add(cl.qualifiedName)) return;

    List<StructMethod> declared = members(cl).methods().getOrDefault(new MemberSignature(requested.member()), List.of());
    for (StructMethod method : declared) {
      List<MemberKey> candidates = methodCandidates.computeIfAbsent(method, key -> overrideCandidates(cl, key));
      // A nearer explicit contract replaces an older one, including a change
      // from a fixed return domain to a parameter-derived return. Unrelated
      // interfaces still contribute competing candidates and remain ambiguous.
      List<MemberKey> bound = candidates.stream()
        .filter(candidate -> contracts.containsKey(requested.withMember(candidate))).toList();
      for (MemberKey candidate : bound) {
        boolean shadowed = bound.stream().anyMatch(other -> !other.owner().equals(candidate.owner())
          && SourceMethodSemantics.isSubtype(DecompilerContext.getStructContext(), other.owner(), candidate.owner()));
        if (!shadowed) {
          found.add(requested.withMember(candidate));
        }
      }
    }
    if (!declared.isEmpty()) return;

    if (cl.superClass != null) {
      collectMethodTargets(requested, cl.superClass.getString(), seen, found);
    }
    for (String iface : cl.getInterfaceNames()) {
      collectMethodTargets(requested, iface, seen, found);
    }
  }

  private ClassMembers members(StructClass cl) {
    return classMembers.computeIfAbsent(cl, owner -> {
      Map<MemberSignature, List<StructField>> fields = new HashMap<>();
      for (StructField field : owner.getFields()) {
        MemberKey named = new MemberKey(owner.qualifiedName, field.getName(), field.getDescriptor());
        fields.computeIfAbsent(new MemberSignature(named), ignored -> new ArrayList<>()).add(field);
      }
      Map<MemberSignature, List<StructMethod>> methods = new HashMap<>();
      for (StructMethod method : owner.getMethods()) {
        MemberKey named = new MemberKey(owner.qualifiedName, method.getName(), method.getDescriptor());
        methods.computeIfAbsent(new MemberSignature(named), ignored -> new ArrayList<>()).add(method);
      }
      return new ClassMembers(fields, methods);
    });
  }

  private static List<MemberKey> overrideCandidates(StructClass cl, StructMethod method) {
    List<MemberKey> candidates = new ArrayList<>();
    candidates.add(new MemberKey(cl.qualifiedName, method.getName(), method.getDescriptor()));
    for (SourceMethodSemantics.InheritedMethod inherited : SourceMethodSemantics.findOverriddenMethods(
      DecompilerContext.getStructContext(), cl, method
    )) {
      StructMethod inheritedMethod = inherited.method();
      candidates.add(new MemberKey(inherited.ownerClass().qualifiedName, inheritedMethod.getName(), inheritedMethod.getDescriptor()));
    }
    return List.copyOf(candidates);
  }

  private StructClass resolveClass(String owner) {
    return DecompilerContext.getStructContext().getClass(owner);
  }

  private boolean isAccessible(Value value, String currentOwner) {
    if (value.synthetic() || (value.access() & CodeConstants.ACC_PUBLIC) != 0) return true;
    if ((value.access() & CodeConstants.ACC_PRIVATE) != 0) return value.owner().equals(currentOwner);
    if (packageName(value.owner()).equals(packageName(currentOwner))) return true;
    return (value.access() & CodeConstants.ACC_PROTECTED) != 0 &&
      DecompilerContext.getStructContext().instanceOf(currentOwner, value.owner());
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
