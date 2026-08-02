// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonParseException;
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
import java.io.Reader;
import java.nio.file.Files;
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
  private record Sidecar(
    int version,
    String namespace,
    List<DomainEntry> domains,
    List<ValueEntry> values,
    List<ScalarBindingEntry> scalarBindings,
    List<ArrayBindingEntry> arrayBindings,
    List<ReturnDomainSourceEntry> returnDomainSources
  ) {}
  private record DomainEntry(String id, String kind) {}
  private record ValueEntry(String domain, long value, String owner, String name, String desc, int access,
                            boolean synthetic, String elementDomain) {}
  private record TargetEntry(String kind, String owner, String name, String desc, Integer index) {}
  private record ScalarBindingEntry(TargetEntry target, String domain) {}
  private record DimensionEntry(int dimension, String domain) {}
  private record ArrayBindingEntry(TargetEntry target, List<DimensionEntry> indexDomains,
                                   List<DimensionEntry> slotDomains, String elementDomain) {}
  private record ReturnDomainSourceEntry(TargetEntry target, int sourceParameter) {}

  private final Map<String, String> domainKinds;
  private final Map<String, Map<Long, Value>> values;
  private final Map<BindingTarget, String> scalarBindings;
  private final Map<BindingTarget, ArraySemantics> arrayBindings;
  private final Map<BindingTarget, Integer> returnDomainSources;
  // Most member expressions have no explicit semantic binding. Cache misses as
  // well as hits so repeated uses do not keep walking the same class hierarchy.
  private final Map<BindingTarget, Optional<String>> scalarBindingCache = new ConcurrentHashMap<>();
  private final Map<BindingTarget, Optional<ArraySemantics>> arrayBindingCache = new ConcurrentHashMap<>();
  private final Map<BindingTarget, Optional<Integer>> returnDomainSourceCache = new ConcurrentHashMap<>();

  private SemanticMappings(
    Map<String, String> domainKinds,
    Map<String, Map<Long, Value>> values,
    Map<BindingTarget, String> scalarBindings,
    Map<BindingTarget, ArraySemantics> arrayBindings,
    Map<BindingTarget, Integer> returnDomainSources
  ) {
    this.domainKinds = domainKinds;
    this.values = values;
    this.scalarBindings = scalarBindings;
    this.arrayBindings = arrayBindings;
    this.returnDomainSources = returnDomainSources;
  }

  public static SemanticMappings load(Path path) throws IOException {
    Sidecar root;
    try (Reader reader = Files.newBufferedReader(path)) {
      root = GSON.fromJson(reader, Sidecar.class);
    }
    catch (JsonParseException ex) {
      throw new IOException("Invalid semantic map: " + path, ex);
    }
    if (root == null || root.version() != 4) {
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

    Map<BindingTarget, String> scalarBindings = new LinkedHashMap<>();
    for (ScalarBindingEntry entry : entries(root.scalarBindings())) {
      scalarBindings.put(target(entry.target()), entry.domain());
    }

    Map<BindingTarget, ArraySemantics> arrayBindings = new LinkedHashMap<>();
    for (ArrayBindingEntry entry : entries(root.arrayBindings())) {
      arrayBindings.put(target(entry.target()), new ArraySemantics(
        dimensionDomains(entry.indexDomains()),
        dimensionDomains(entry.slotDomains()),
        entry.elementDomain()
      ));
    }

    Map<BindingTarget, Integer> returnDomainSources = new LinkedHashMap<>();
    for (ReturnDomainSourceEntry entry : entries(root.returnDomainSources())) {
      returnDomainSources.put(target(entry.target()), entry.sourceParameter());
    }

    return new SemanticMappings(domainKinds, values, scalarBindings, arrayBindings, returnDomainSources);
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
    return parameterDomain(method, parameter) != null || parameterArraySemantics(method, parameter) != null;
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
    return scalarBindings.containsKey(target) || arrayBindings.containsKey(target) || returnDomainSources.containsKey(target);
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
      addDeclaredBinding(bindings, requested, declaration, found);
      if (SourceMethodSemantics.canParticipateInOverride(method)) {
        for (SourceMethodSemantics.InheritedMethod inherited : SourceMethodSemantics.findOverriddenMethods(
          DecompilerContext.getStructContext(), cl, method
        )) {
          StructMethod inheritedMethod = inherited.method();
          addDeclaredBinding(bindings, requested, new MemberKey(
            inherited.ownerClass().qualifiedName,
            inheritedMethod.getName(),
            inheritedMethod.getDescriptor()
          ), found);
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
