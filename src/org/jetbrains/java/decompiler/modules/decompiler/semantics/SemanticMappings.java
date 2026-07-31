// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonParseException;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IResultSaver;
import org.jetbrains.java.decompiler.struct.StructClass;

import java.io.IOException;
import java.io.Reader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class SemanticMappings {
  private static final Gson GSON = new GsonBuilder()
    .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
    .create();

  public record MemberKey(String owner, String name, String desc) {}
  public record ArraySemantics(Map<Integer, String> indexDomains, Map<Integer, String> slotDomains, String elementDomain) {
    public ArraySemantics {
      indexDomains = Map.copyOf(indexDomains);
      slotDomains = Map.copyOf(slotDomains);
    }

    public boolean isEmpty() {
      return indexDomains.isEmpty() && slotDomains.isEmpty() && elementDomain == null;
    }

    public ArraySemantics element() {
      return new ArraySemantics(shift(indexDomains), shift(slotDomains), elementDomain);
    }

    public ArraySemantics withElementDomain(String domain) {
      return new ArraySemantics(indexDomains, slotDomains, domain);
    }

    private static Map<Integer, String> shift(Map<Integer, String> domains) {
      Map<Integer, String> shifted = new LinkedHashMap<>();
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
  private record MaskedValue(Value value, long mask) {}
  private record FlagCover(List<Value> values, long residual) {}
  private record Sidecar(
    int version,
    String namespace,
    List<DomainEntry> domains,
    List<ValueEntry> values,
    List<BindingEntry> fieldBindings,
    List<BindingEntry> returnBindings,
    List<BindingEntry> parameterBindings,
    List<ArrayEntry> fieldArrays,
    List<ArrayEntry> returnArrays,
    List<ArrayEntry> parameterArrays
  ) {}
  private record DomainEntry(String id, String kind) {}
  private record ValueEntry(String domain, long value, String owner, String name, String desc, int access,
                            boolean synthetic, String elementDomain) {}
  private record BindingEntry(String owner, String name, String desc, int index, String domain) {}
  private record DimensionEntry(int dimension, String domain) {}
  private record ArrayEntry(String owner, String name, String desc, int index,
                            List<DimensionEntry> indexDomains, List<DimensionEntry> slotDomains,
                            String elementDomain) {}

  private final Map<String, String> domainKinds;
  private final Map<String, Map<Long, Value>> values;
  private final Map<MemberKey, String> fieldDomains;
  private final Map<MemberKey, String> returnDomains;
  private final Map<MemberKey, Map<Integer, String>> parameterDomains;
  private final Map<MemberKey, ArraySemantics> fieldArrays;
  private final Map<MemberKey, ArraySemantics> returnArrays;
  private final Map<MemberKey, Map<Integer, ArraySemantics>> parameterArrays;

  private SemanticMappings(
    Map<String, String> domainKinds,
    Map<String, Map<Long, Value>> values,
    Map<MemberKey, String> fieldDomains,
    Map<MemberKey, String> returnDomains,
    Map<MemberKey, Map<Integer, String>> parameterDomains,
    Map<MemberKey, ArraySemantics> fieldArrays,
    Map<MemberKey, ArraySemantics> returnArrays,
    Map<MemberKey, Map<Integer, ArraySemantics>> parameterArrays
  ) {
    this.domainKinds = domainKinds;
    this.values = values;
    this.fieldDomains = fieldDomains;
    this.returnDomains = returnDomains;
    this.parameterDomains = parameterDomains;
    this.fieldArrays = fieldArrays;
    this.returnArrays = returnArrays;
    this.parameterArrays = parameterArrays;
  }

  public static SemanticMappings load(Path path) throws IOException {
    Sidecar root;
    try (Reader reader = Files.newBufferedReader(path)) {
      root = GSON.fromJson(reader, Sidecar.class);
    } catch (JsonParseException ex) {
      throw new IOException("Invalid semantic map: " + path, ex);
    }
    if (root == null || root.version() != 2) {
      throw new IOException("Unsupported semantic map version in " + path);
    }
    if (!"named".equals(root.namespace())) {
      throw new IOException("Semantic map must use the named namespace: " + path);
    }

    Map<String, String> domainKinds = new LinkedHashMap<>();
    for (DomainEntry entry : entries(root.domains())) {
      domainKinds.put(entry.id(), entry.kind());
    }

    Map<String, Map<Long, Value>> values = new LinkedHashMap<>();
    for (ValueEntry entry : entries(root.values())) {
      Value value = new Value(
        entry.domain(), entry.value(), entry.owner(), entry.name(), entry.desc(), entry.access(),
        entry.synthetic(), entry.elementDomain()
      );
      values.computeIfAbsent(entry.domain(), ignored -> new LinkedHashMap<>()).put(value.value(), value);
    }

    Map<MemberKey, String> fields = readFieldBindings(root.fieldBindings());
    Map<MemberKey, String> returns = readFieldBindings(root.returnBindings());

    Map<MemberKey, Map<Integer, String>> parameters = new LinkedHashMap<>();
    for (BindingEntry entry : entries(root.parameterBindings())) {
      MemberKey method = member(entry);
      parameters.computeIfAbsent(method, ignored -> new LinkedHashMap<>()).put(entry.index(), entry.domain());
    }

    Map<MemberKey, ArraySemantics> fieldArrays = readArrays(root.fieldArrays());
    Map<MemberKey, ArraySemantics> returnArrays = readArrays(root.returnArrays());
    Map<MemberKey, Map<Integer, ArraySemantics>> parameterArrays = new LinkedHashMap<>();
    for (ArrayEntry entry : entries(root.parameterArrays())) {
      parameterArrays.computeIfAbsent(member(entry), ignored -> new LinkedHashMap<>())
        .put(entry.index(), arraySemantics(entry));
    }

    return new SemanticMappings(domainKinds, values, fields, returns, parameters, fieldArrays, returnArrays, parameterArrays);
  }

  public String fieldDomain(MemberKey field) {
    return inheritedBinding(fieldDomains, field);
  }

  public String returnDomain(MemberKey method) {
    return inheritedBinding(returnDomains, method);
  }

  public String parameterDomain(MemberKey method, int index) {
    MemberKey declaration = inheritedKey(parameterDomains, method);
    return declaration == null ? null : parameterDomains.get(declaration).get(index);
  }

  public ArraySemantics fieldArraySemantics(MemberKey field) {
    return inheritedBinding(fieldArrays, field);
  }

  public ArraySemantics returnArraySemantics(MemberKey method) {
    return inheritedBinding(returnArrays, method);
  }

  public ArraySemantics parameterArraySemantics(MemberKey method, int parameter) {
    MemberKey declaration = inheritedKey(parameterArrays, method);
    if (declaration == null) return null;
    return parameterArrays.get(declaration).get(parameter);
  }

  public String domainKind(String domain) {
    return domainKinds.get(domain);
  }

  public Value value(String domain, long literal, String currentOwner) {
    Value value = values.getOrDefault(domain, Map.of()).get(literal);
    return value != null && isAccessible(value, currentOwner) ? value : null;
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

    FlagCover positive = coverFlags(domainValues, target, widthMask);
    FlagCover negative = coverFlags(domainValues, (~target) & widthMask, widthMask);
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

  public void writeSyntheticSources(IResultSaver saver) {
    for (Map.Entry<String, Map<Long, Value>> entry : values.entrySet()) {
      List<Value> synthetic = entry.getValue().values().stream().filter(Value::synthetic).toList();
      if (synthetic.isEmpty()) {
        continue;
      }
      String owner = synthetic.get(0).owner();
      int slash = owner.lastIndexOf('/');
      String packageName = slash < 0 ? "" : owner.substring(0, slash).replace('/', '.');
      String simpleName = slash < 0 ? owner : owner.substring(slash + 1);
      StringBuilder source = new StringBuilder();
      if (!packageName.isEmpty()) {
        source.append("package ").append(packageName).append(";\n\n");
      }
      source.append("// Generated from semantic mappings; not present in the input JAR.\n");
      source.append("public interface ").append(simpleName).append(" {\n");
      synthetic.stream().sorted((a, b) -> a.name().compareTo(b.name())).forEach(value ->
        source.append("   ").append(javaType(value.desc())).append(' ').append(value.name())
          .append(" = ").append(javaLiteral(value)).append(";\n")
      );
      source.append("}\n");
      String outputPath = slash < 0 ? "" : owner.substring(0, slash);
      saver.saveFolder(outputPath);
      saver.saveClassFile("", owner, owner + ".java", source.toString(), null);
    }
  }

  private <T> T inheritedBinding(Map<MemberKey, T> bindings, MemberKey requested) {
    MemberKey key = inheritedKey(bindings, requested);
    return key == null ? null : bindings.get(key);
  }

  private <T> MemberKey inheritedKey(Map<MemberKey, T> bindings, MemberKey requested) {
    if (bindings.containsKey(requested)) {
      return requested;
    }
    return findInHierarchy(bindings, requested, requested.owner(), new HashSet<>());
  }

  private <T> MemberKey findInHierarchy(Map<MemberKey, T> bindings, MemberKey requested, String owner, Set<String> seen) {
    if (!seen.add(owner)) return null;
    MemberKey candidate = new MemberKey(owner, requested.name(), requested.desc());
    if (bindings.containsKey(candidate)) return candidate;
    StructClass cl = DecompilerContext.getStructContext().getClass(owner);
    if (cl == null) return null;
    if (cl.superClass != null) {
      MemberKey found = findInHierarchy(bindings, requested, cl.superClass.getString(), seen);
      if (found != null) return found;
    }
    for (String iface : cl.getInterfaceNames()) {
      MemberKey found = findInHierarchy(bindings, requested, iface, seen);
      if (found != null) return found;
    }
    return null;
  }

  private static boolean isAccessible(Value value, String currentOwner) {
    if (value.synthetic() || (value.access() & CodeConstants.ACC_PUBLIC) != 0) return true;
    if ((value.access() & CodeConstants.ACC_PRIVATE) != 0) return value.owner().equals(currentOwner);
    return packageName(value.owner()).equals(packageName(currentOwner));
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

  private static Map<MemberKey, String> readFieldBindings(List<BindingEntry> bindings) {
    Map<MemberKey, String> result = new LinkedHashMap<>();
    for (BindingEntry entry : entries(bindings)) {
      result.put(member(entry), entry.domain());
    }
    return result;
  }

  private static MemberKey member(BindingEntry entry) {
    return new MemberKey(entry.owner(), entry.name(), entry.desc());
  }

  private static MemberKey member(ArrayEntry entry) {
    return new MemberKey(entry.owner(), entry.name(), entry.desc());
  }

  private static Map<MemberKey, ArraySemantics> readArrays(List<ArrayEntry> entries) {
    Map<MemberKey, ArraySemantics> result = new LinkedHashMap<>();
    for (ArrayEntry entry : entries(entries)) result.put(member(entry), arraySemantics(entry));
    return result;
  }

  private static ArraySemantics arraySemantics(ArrayEntry entry) {
    return new ArraySemantics(dimensionDomains(entry.indexDomains()), dimensionDomains(entry.slotDomains()), entry.elementDomain());
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
