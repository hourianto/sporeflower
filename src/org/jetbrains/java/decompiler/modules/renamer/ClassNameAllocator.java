package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.consts.PrimitiveConstant;
import org.jetbrains.java.decompiler.util.PortablePathKey;

import java.text.Normalizer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.TreeSet;
import java.util.stream.Collectors;

/** Allocates Java type/package identities and portable output paths before any pool mutation. */
final class ClassNameAllocator {
  private final StructContext context;
  private final IIdentifierRenamer helper;
  private final ConverterHelper fallback;
  private final List<StructClass> classOrder;
  private final Map<String, Placement> placements = new LinkedHashMap<>();
  private final Map<String, List<Placement>> children = new HashMap<>();
  private final Map<String, String> reservedChildren = new HashMap<>();
  private final Map<String, String> names = new LinkedHashMap<>();
  private final Map<String, String> innerNames = new LinkedHashMap<>();
  private ClassNamespace namespace;

  ClassNameAllocator(StructContext context, IIdentifierRenamer helper, ConverterHelper fallback, List<StructClass> classOrder) {
    this.context = context;
    this.helper = helper;
    this.fallback = fallback;
    this.classOrder = classOrder;
  }

  void allocate() {
    Map<String, ClassNode> nodes = nestedClasses();
    Map<String, String> requestedInnerNames = helper instanceof Tiny2IdentifierRenamer tiny ? tiny.declarationNames().innerNames() : Map.of();
    for (StructClass owner : classOrder) {
      if (!owner.isOwn()) continue;
      Placement placement = new Placement(owner.qualifiedName, nodes.get(owner.qualifiedName));
      placements.put(owner.qualifiedName, placement);
      if (placement.node != null) children.computeIfAbsent(placement.node.parent.classStruct.qualifiedName,
        unused -> new ArrayList<>()).add(placement);
    }
    for (Placement placement : placements.values()) placement.requested = requestedName(placement);
    for (Placement placement : placements.values()) {
      if (placement.node != null && placement.node.simpleName != null) {
        String simple = requestedInnerNames.getOrDefault(placement.original, placement.node.simpleName);
        if (placement.node.type == ClassNode.Type.MEMBER && placement.requested != null
            && !simpleName(placement.requested).equals(simpleName(placement.original))) {
          String parent = placement.node.parent.classStruct.qualifiedName;
          String outer = placements.get(parent) == null ? parent : placements.get(parent).requested;
          simple = outer != null && placement.requested.startsWith(outer + "$")
            ? placement.requested.substring(outer.length() + 1) : simpleName(placement.requested);
        }
        placement.simple = legalSimple(simple) ? canonical(simple) : null;
      }
    }

    Map<String, String> packages = allocatePackageGroups();
    for (Placement placement : placements.values()) {
      if (placement.node == null) placement.packageName = packages.get(packageName(placement.original));
    }
    namespace = new ClassNamespace(context, reservedClassNames(), new HashSet<>(packages.values()));
    // Reserve all meaningful requests before generating any fallback. This also
    // covers unchanged declarations, which formerly bypassed class allocation.
    for (Placement placement : placements.values()) reserveRequest(placement, new HashSet<>());
    List<Placement> ordered = new ArrayList<>(placements.values());
    // Original and explicitly requested spellings have equal priority, as for
    // members. Stable declaration order breaks ties; generated names go last.
    ordered.sort(Comparator.comparingInt(placement -> needsGeneratedName(placement) ? 1 : 0));
    Set<String> visiting = new HashSet<>();
    for (Placement placement : ordered) place(placement, visiting);
  }

  String name(String original) { return names.getOrDefault(original, original); }
  Map<String, String> classes() { return names; }
  Map<String, String> innerNames() { return innerNames; }

  void moveObscuredClass(String original, Set<String> blockedSimpleNames) {
    Placement placement = placements.get(original);
    if (placement == null) throw new IllegalArgumentException("Cannot rename external class " + original);
    String current = name(original);
    namespace.remove(original, current);
    String replacement;
    String simple;
    do {
      simple = fallback.getNextClassName(original, placement.simple == null ? simpleName(current) : placement.simple);
      replacement = placement.node != null && placement.node.simpleName != null
        ? nestedTarget(placement, name(placement.node.parent.classStruct.qualifiedName), simple)
        : ConverterHelper.replaceSimpleClassName(current, simple);
    } while (blockedSimpleNames.contains(simple) || obscuresEnclosing(placement, simple) || !namespace.available(original, replacement, true));
    names.put(original, replacement);
    namespace.add(original, replacement);
    if (placement.node != null && placement.node.simpleName != null) {
      placement.simple = simple;
      innerNames.put(original, simple);
    }
    // The enclosing class is the lexical qualifier. Moving it changes every
    // descendant binary name, including locals and anonymous classes.
    Set<String> descendants = new LinkedHashSet<>();
    collectDescendants(original, descendants);
    Map<String, String> previous = new HashMap<>();
    previous.put(original, current);
    for (String child : descendants) {
      previous.put(child, names.remove(child));
      namespace.remove(child, previous.get(child));
    }
    for (String child : descendants) place(placements.get(child), new HashSet<>(), previous);
  }

  private void collectDescendants(String parent, Set<String> result) {
    for (Placement placement : children.getOrDefault(parent, List.of())) {
      if (result.add(placement.original)) {
        collectDescendants(placement.original, result);
      }
    }
  }

  private String requestedName(Placement placement) {
    String original = placement.original;
    String simple = simpleName(original);
    String requested = original;
    if (helper instanceof Tiny2IdentifierRenamer tiny) {
      String mapped = tiny.getMappedClassName(original);
      if (mapped != null) requested = mapped;
    } else if (helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_CLASS, simple, null, null)) {
      if (helper.getClass() == ConverterHelper.class) return null;
      placement.custom = true;
      requested = helper.getNextClassName(original, simple);
      if (requested == null) return null;
      if (requested.indexOf('/') < 0) requested = ConverterHelper.replaceSimpleClassName(original, requested);
    }
    // Package requests survive an illegal simple name and are collected once
    // for the original package, independently of individual class allocation.
    if (!packageName(requested).equals(packageName(original))) placement.requestedPackage = packageName(requested);
    return legal(simpleName(requested)) ? requested : null;
  }

  private String reserveRequest(Placement placement, Set<String> visiting) {
    if (placement.reserved != null) return placement.reserved;
    if (!visiting.add(placement.original)) throw new IllegalArgumentException("Circular member-class ownership: " + placement.original);
    String target;
    if (placement.node == null) {
      target = inPackage(placement.packageName, canonical(simpleName(placement.requested == null ? placement.original : placement.requested)));
    } else {
      String parent = placement.node.parent.classStruct.qualifiedName;
      Placement enclosing = placements.get(parent);
      String outer = enclosing == null ? parent : reserveRequest(enclosing, visiting);
      target = nestedTarget(placement, outer, placement.simple == null ? placement.node.simpleName : placement.simple);
    }
    namespace.reserve(placement.original, target);
    placement.reserved = target;
    visiting.remove(placement.original);
    return target;
  }

  private void place(Placement placement, Set<String> visiting) {
    place(placement, visiting, Map.of());
  }

  private void place(Placement placement, Set<String> visiting, Map<String, String> previous) {
    if (names.containsKey(placement.original)) return;
    if (!visiting.add(placement.original)) throw new IllegalArgumentException("Circular member-class ownership: " + placement.original);
    String outer = null;
    if (placement.node != null) {
      String parent = placement.node.parent.classStruct.qualifiedName;
      Placement enclosing = placements.get(parent);
      if (enclosing != null) place(enclosing, visiting, previous);
      outer = name(parent);
      // Reserve the whole sibling group after its enclosing class moves, so a
      // replacement cannot take a later sibling's preferred spelling.
      if (!outer.equals(reservedChildren.put(parent, outer))) {
        for (Placement sibling : children.getOrDefault(parent, List.of())) {
          namespace.reserve(sibling.original, preferredCandidate(sibling, outer, previous).target);
        }
      }
    }
    boolean generated = !previous.containsKey(placement.original) && needsGeneratedName(placement);
    Candidate preferred = preferredCandidate(placement, outer, previous);
    Candidate candidate = generated ? generatedCandidate(placement, outer, preferred.simple) : preferred;
    boolean replacement = generated;
    int suffix = 1;
    while (!namespace.available(placement.original, candidate.target, replacement) || obscuresEnclosing(placement, candidate.simple)) {
      if (placement.custom && previous.isEmpty()) generated = true;
      candidate = generated ? generatedCandidate(placement, outer, preferred.simple)
        : new Candidate(preferred.simple == null ? null : preferred.simple + "_" + suffix, preferred.target + "_" + suffix);
      suffix++;
      replacement = true;
    }
    names.put(placement.original, candidate.target);
    namespace.addPackage(packageName(candidate.target));
    namespace.add(placement.original, candidate.target);
    if (candidate.simple != null && placement.node != null) {
      placement.simple = candidate.simple;
      innerNames.put(placement.original, candidate.simple);
    }
    visiting.remove(placement.original);
  }

  private static boolean needsGeneratedName(Placement placement) {
    if (placement.node == null) return placement.requested == null;
    if (placement.node.type == ClassNode.Type.MEMBER) return placement.requested == null || placement.simple == null;
    return placement.node.simpleName != null && placement.simple == null;
  }

  private Candidate preferredCandidate(Placement placement, String outer, Map<String, String> previous) {
    String simple = placement.node == null
      ? canonical(simpleName(placement.requested == null ? placement.original : placement.requested)) : placement.simple;
    String oldTarget = previous.get(placement.original);
    if (oldTarget != null) {
      if (placement.node == null) return new Candidate(simpleName(oldTarget), inPackage(placement.packageName, simpleName(oldTarget)));
      String oldOuter = previous.get(placement.node.parent.classStruct.qualifiedName);
      // Rebase the already allocated binary suffix. Regenerating it would lose
      // repaired nested names or collision suffixes on anonymous/local classes.
      String target = oldTarget.startsWith(oldOuter + "$") ? outer + oldTarget.substring(oldOuter.length())
        : inPackage(packageName(outer), simpleName(oldTarget));
      return new Candidate(simple, target);
    }
    return candidate(placement, outer, simple);
  }

  private Candidate generatedCandidate(Placement placement, String outer, String simple) {
    return candidate(placement, outer, fallback.getNextClassName(placement.original, simple));
  }

  private static Candidate candidate(Placement placement, String outer, String simple) {
    return new Candidate(simple, placement.node == null ? inPackage(placement.packageName, simple) : nestedTarget(placement, outer, simple));
  }

  private record Candidate(String simple, String target) { }

  private boolean obscuresEnclosing(Placement placement, String simple) {
    if (placement.node == null || simple == null) return false;
    for (ClassNode parent = placement.node.parent; parent != null; parent = parent.parent) {
      String parentName = innerNames.getOrDefault(parent.classStruct.qualifiedName, simpleName(name(parent.classStruct.qualifiedName)));
      if (simple.equals(parentName)) return true;
    }
    return false;
  }

  private static String nestedTarget(Placement placement, String outer, String simple) {
    if (placement.node.type == ClassNode.Type.MEMBER) return outer + "$" + simple;
    String parent = placement.node.parent.classStruct.qualifiedName;
    if (placement.original.startsWith(parent + "$")) {
      String suffix = placement.original.substring(parent.length());
      if (simple != null && !Objects.equals(simple, placement.node.simpleName)) {
        // Keep the compiler's local ordinal, rather than collapsing all local
        // classes to $1 when their source simple names are repaired.
        int index = 1;
        while (index < suffix.length() && Character.isDigit(suffix.charAt(index))) index++;
        suffix = suffix.substring(0, index) + simple;
      }
      String target = outer + suffix;
      return simple == null && !legal(target) ? outer + "$anonymous" : target;
    }
    if (simple != null && !Objects.equals(simple, placement.node.simpleName)) return outer + "$1" + simple;
    String target = inPackage(packageName(outer), canonical(simpleName(placement.requested == null ? placement.original : placement.requested)));
    return simple == null && !legal(target) ? outer + "$anonymous" : target;
  }

  private Map<String, String> allocatePackageGroups() {
    String defaultPackage = defaultRelocation();
    Set<String> libraries = libraryPackages(context);
    Map<String, String> requests = new LinkedHashMap<>();
    for (Placement placement : placements.values()) {
      if (placement.node == null && placement.requestedPackage != null) {
        requests.putIfAbsent(packageName(placement.original), placement.requestedPackage);
      }
    }
    for (Placement placement : placements.values()) {
      if (placement.node == null) {
        String original = packageName(placement.original);
        requests.putIfAbsent(original, original.isEmpty() && defaultPackage != null ? defaultPackage : original);
      }
    }
    requests.replaceAll((original, requested) -> libraries.contains(original) ? original : requested);
    Set<String> pinned = new HashSet<>(requests.keySet());
    pinned.retainAll(libraries);
    List<String> ordered = new ArrayList<>(requests.keySet());
    ordered.sort(Comparator.comparing(original -> !pinned.contains(original)));
    Set<String> reserved = new HashSet<>(requests.values());
    Set<String> used = new HashSet<>();
    Map<String, String> distinct = new LinkedHashMap<>();
    // Keep the original package equality relation in both directions. Splits
    // lose package access; merges can introduce new overriding and hiding.
    // A request also cannot displace another package's original identity.
    for (String original : ordered) {
      String preferred = requests.get(original);
      String target = preferred;
      for (int suffix = 1; used.contains(target) || !target.equals(original) && (libraries.contains(target) || requests.containsKey(target))
          || !target.equals(preferred) && reserved.contains(target); suffix++) {
        target = preferred.isEmpty() ? "package_" + suffix : preferred + "_" + suffix;
      }
      distinct.put(original, target);
      used.add(target);
    }
    Map<String, String> repaired = allocatePackages(new TreeSet<>(distinct.values()), libraries);
    distinct.replaceAll((original, requested) -> repaired.get(requested));
    for (String original : pinned) {
      if (!original.equals(distinct.get(original))) {
        DecompilerContext.getLogger().writeMessage("Cannot preserve fixed library package " + original
          + ": Java source requires " + distinct.get(original), IFernflowerLogger.Severity.WARN);
      }
    }
    Map<String, Set<String>> ignoredRequests = new LinkedHashMap<>();
    for (Placement placement : placements.values()) {
      if (placement.node == null && placement.requestedPackage != null
          && !placement.requestedPackage.equals(distinct.get(packageName(placement.original)))) {
        ignoredRequests.computeIfAbsent(packageName(placement.original), unused -> new TreeSet<>())
          .add(displayPackage(placement.requestedPackage));
      }
    }
    ignoredRequests.forEach((original, ignored) -> DecompilerContext.getLogger().writeMessage(
      "Package " + displayPackage(original) + " uses " + displayPackage(distinct.get(original))
        + "; ignored requested packages " + ignored + "; original packages move as units", IFernflowerLogger.Severity.WARN));
    return distinct;
  }

  private Map<String, String> allocatePackages(Set<String> requested, Set<String> libraries) {
    Set<String> pinnedPrefixes = new HashSet<>();
    libraries.forEach(pkg -> addPackagePrefixes(pkg, pinnedPrefixes));
    // Prefix keys include the terminating slash. That distinguishes the root
    // from an empty first segment in malformed names such as /pkg or pkg//sub.
    Set<String> prefixes = new HashSet<>();
    for (String pkg : requested) {
      if (pkg.isEmpty()) continue;
      String prefix = "";
      for (String segment : pkg.split("/", -1)) {
        prefix += segment + "/";
        prefixes.add(prefix);
      }
    }
    Map<String, String> targets = new HashMap<>();
    targets.put("", "");
    Map<String, String> assignedPaths = new HashMap<>();
    Set<String> reservedPaths = prefixes.stream().map(prefix -> prefix.substring(0, prefix.length() - 1))
      .filter(ClassNameAllocator::legal).map(ClassNameAllocator::fold).collect(Collectors.toSet());
    List<String> ordered = new ArrayList<>(prefixes);
    ordered.sort(Comparator.comparingLong((String value) -> value.chars().filter(ch -> ch == '/').count())
      .thenComparingInt(value -> pinnedPrefixes.contains(value.substring(0, value.length() - 1)) ? 0 : 1)
      .thenComparingInt(value -> legal(simpleName(value.substring(0, value.length() - 1))) ? 0 : 1)
      .thenComparingInt(value -> Normalizer.isNormalized(value, Normalizer.Form.NFC) ? 0 : 1)
      .thenComparing(Comparator.naturalOrder()));
    int generated = 0;
    Set<String> reservedClasses = reservedClassNames();
    for (String prefix : ordered) {
      String original = prefix.substring(0, prefix.length() - 1);
      int slash = original.lastIndexOf('/');
      String parent = targets.get(original.substring(0, slash + 1));
      String simple = original.substring(slash + 1);
      String preferred = legal(simple) ? canonical(simple) : "package_" + generated++;
      String target = inPackage(parent, preferred);
      for (int suffix = 1; assignedPaths.containsKey(fold(target))
          || !fold(target).equals(fold(original)) && reservedPaths.contains(fold(target))
          || libraries.contains(target) && !target.equals(original)
          || reservedClasses.contains(target) || externalClass(context, target); suffix++) {
        target = inPackage(parent, preferred + "_" + suffix);
      }
      targets.put(prefix, target);
      assignedPaths.put(fold(target), prefix);
    }
    Map<String, String> result = new HashMap<>();
    for (String pkg : requested) result.put(pkg, targets.get(pkg.isEmpty() ? "" : pkg + "/"));
    return result;
  }

  private String defaultRelocation() {
    boolean defaultClasses = false;
    boolean referenced = false;
    for (StructClass owner : context.getOwnClasses()) {
      if (packageName(owner.qualifiedName).isEmpty()) defaultClasses = true;
      else for (var constant : owner.getPool().getPool()) {
        if (constant instanceof PrimitiveConstant primitive && primitive.type == CodeConstants.CONSTANT_Class) {
          String type = primitive.getString();
          if (type.indexOf('/') < 0 && !type.startsWith("[")) {
            StructClass target = context.getClass(type);
            if (target != null && target.isOwn()) referenced = true;
          }
        }
      }
      // Field and method descriptors need not have a CONSTANT_Class entry.
      if (!packageName(owner.qualifiedName).isEmpty()) {
        for (var field : owner.getFields()) if (referencesDefaultClass(field.getDescriptor())) referenced = true;
        for (var method : owner.getMethods()) if (referencesDefaultClass(method.getDescriptor())) referenced = true;
      }
    }
    String configured = (String)DecompilerContext.getProperty(IFernflowerPreferences.DEFAULT_PACKAGE);
    if (!defaultClasses || !referenced && configured.isEmpty()) return null;
    String base = configured.isEmpty() ? "decompiled/defaultpkg" : configured;
    Set<String> occupied = new HashSet<>();
    for (String reserved : reservedClassNames()) occupied.add(packageName(reserved));
    for (Placement placement : placements.values()) {
      occupied.add(packageName(placement.original));
      if (placement.requested != null && !packageName(placement.original).isEmpty()) occupied.add(packageName(placement.requested));
    }
    String target = base;
    for (int suffix = 1; occupied.contains(target); suffix++) target = base + suffix;
    return target;
  }

  private boolean referencesDefaultClass(String descriptor) {
    for (int start = descriptor.indexOf('L'); start >= 0; start = descriptor.indexOf('L', start + 1)) {
      int end = descriptor.indexOf(';', start);
      if (end < 0) break;
      String type = descriptor.substring(start + 1, end);
      if (type.indexOf('/') < 0) {
        StructClass referenced = context.getClass(type);
        if (referenced != null && referenced.isOwn()) return true;
      }
      start = end;
    }
    return false;
  }

  static void validate(NamingPlan plan, StructContext context) {
    Map<String, ClassNode> nested = nestedClasses();
    Set<String> libraries = libraryPackages(context);
    Map<String, String> destinations = new HashMap<>();
    Map<String, String> origins = new HashMap<>();
    for (var entry : plan.classes().entrySet()) {
      String original = packageName(entry.getKey()), target = packageName(entry.getValue());
      String previous = destinations.putIfAbsent(original, target);
      if (previous != null && !previous.equals(target)) throw new IllegalArgumentException("Prepared mapping splits original package: " + original);
      previous = origins.putIfAbsent(target, original);
      if (previous != null && !previous.equals(original)) throw new IllegalArgumentException("Prepared mapping merges original packages: " + target);
      if (!target.equals(original) && (libraries.contains(original) || libraries.contains(target))) {
        throw new IllegalArgumentException("Prepared mapping changes a library package boundary: " + original + " -> " + target);
      }
    }
    Set<String> packages = plan.classes().values().stream().map(ClassNameAllocator::packageName).collect(Collectors.toSet());
    ClassNamespace namespace = new ClassNamespace(context, reservedClassNames(), packages);
    Map<String, String> packagePaths = new HashMap<>();
    for (String pkg : namespace.packages) {
      if (!legal(pkg)) throw new IllegalArgumentException("Illegal prepared package name: " + pkg);
      String previous = packagePaths.putIfAbsent(fold(pkg), pkg);
      if (previous != null && !previous.equals(pkg)) throw new IllegalArgumentException("Prepared package output paths collide: " + previous + " and " + pkg);
      if (namespace.reserved.contains(pkg) || externalClass(context, pkg)) {
        throw new IllegalArgumentException("Prepared package collides with a reserved or library class: " + pkg);
      }
    }
    for (var entry : plan.classes().entrySet()) {
      String original = entry.getKey(), target = entry.getValue();
      if (!Normalizer.isNormalized(target, Normalizer.Form.NFC)) {
        throw new IllegalArgumentException("Prepared class output name is not Unicode-normalized: " + target);
      }
      if (!namespace.available(original, target, false)) {
        if (!original.equals(target) && namespace.originals.contains(target)) {
          throw new IllegalArgumentException("Prepared class name reuses another class's original name: " + original + " -> " + target
            + "; class rename chains and swaps are not supported");
        }
        throw new IllegalArgumentException("Prepared class name collides or is illegal: " + target);
      }
      namespace.add(original, target);
      ClassNode node = nested.get(original);
      if (node != null) {
        String outer = plan.classes().getOrDefault(node.parent.classStruct.qualifiedName, node.parent.classStruct.qualifiedName);
        String simple = plan.innerNames().getOrDefault(original, node.simpleName);
        if (simple != null && !legalSimple(simple)) throw new IllegalArgumentException("Illegal prepared nested name: " + simple);
        if (!packageName(target).equals(packageName(outer))
            || node.type == ClassNode.Type.MEMBER && !target.equals(outer + "$" + simple)) {
          throw new IllegalArgumentException("Prepared nested class disagrees with its enclosing class: " + original);
        }
        // Local/anonymous binary ordinals are compiler choices, and valid
        // bytecode can omit the enclosing-name prefix entirely. Their source
        // correspondence is structural; only member binary names are fixed
        // by the enclosing class and declared simple name.
        for (ClassNode parent = node.parent; simple != null && parent != null; parent = parent.parent) {
          String parentName = plan.innerNames().getOrDefault(parent.classStruct.qualifiedName,
            simpleName(plan.classes().getOrDefault(parent.classStruct.qualifiedName, parent.classStruct.qualifiedName)));
          if (simple.equals(parentName)) throw new IllegalArgumentException("Prepared nested class repeats an enclosing name: " + original);
        }
      }
    }
    for (String original : plan.innerNames().keySet()) {
      if (!nested.containsKey(original)) throw new IllegalArgumentException("Unknown prepared nested class: " + original);
    }
  }

  private static Map<String, ClassNode> nestedClasses() {
    Map<String, ClassNode> result = new HashMap<>();
    if (DecompilerContext.getOption(IFernflowerPreferences.DECOMPILE_INNER)) {
      for (ClassNode node : DecompilerContext.getClassProcessor().getMapRootClasses().values()) {
        if (node.parent != null && node.classStruct.isOwn()) result.put(node.classStruct.qualifiedName, node);
      }
    }
    return result;
  }

  private static Set<String> reservedClassNames() {
    return Arrays.stream(((String)DecompilerContext.getProperty(IFernflowerPreferences.RESERVED_CLASS_NAMES)).split(","))
      .map(String::trim).filter(name -> !name.isEmpty()).collect(Collectors.toSet());
  }

  private static Set<String> libraryPackages(StructContext context) {
    return context.getLibraryClassNames().stream().map(ClassNameAllocator::packageName).collect(Collectors.toSet());
  }

  private static boolean externalClass(StructContext context, String name) {
    if (!context.hasClass(name)) return false;
    StructClass owner = context.getClass(name);
    return owner != null && !owner.isOwn();
  }

  private static String simpleName(String name) { return ConverterHelper.getSimpleClassName(name); }
  private static String packageName(String name) { return SourceMethodSemantics.packageName(name); }
  private static String displayPackage(String name) { return name.isEmpty() ? "<default>" : name; }
  private static String inPackage(String pkg, String name) { return pkg.isEmpty() ? name : pkg + "/" + name; }
  private static String canonical(String path) { return Normalizer.normalize(path, Normalizer.Form.NFC); }
  private static String fold(String path) { return PortablePathKey.of(path); }
  private static boolean legalSimple(String name) { return name != null && name.indexOf('/') < 0 && legal(name); }
  private static boolean legal(String name) {
    return name != null && !name.endsWith("/") && !ConverterHelper.mustRenameForJava(IIdentifierRenamer.Type.ELEMENT_CLASS, name);
  }

  private static void addPackagePrefixes(String pkg, Set<String> prefixes) {
    if (pkg.isEmpty()) return;
    prefixes.add(pkg);
    for (int slash = pkg.indexOf('/'); slash >= 0; slash = pkg.indexOf('/', slash + 1)) prefixes.add(pkg.substring(0, slash));
  }

  private static final class Placement {
    final String original;
    final ClassNode node;
    String requested;
    String packageName;
    String simple;
    String reserved;
    String requestedPackage;
    boolean custom;

    Placement(String original, ClassNode node) { this.original = original; this.node = node; }
  }

  /** Exact Java identities and folded artifact paths are different namespaces. */
  private static final class ClassNamespace {
    final StructContext context;
    final Set<String> reserved;
    final Set<String> reservedPaths;
    final Set<String> packages = new HashSet<>();
    final Map<String, String> paths = new HashMap<>();
    final Map<String, Set<String>> requests = new HashMap<>();
    final Set<String> originals;

    ClassNamespace(StructContext context, Set<String> reserved, Set<String> packages) {
      this.context = context;
      this.reserved = reserved;
      this.originals = context.getOwnClasses().stream().map(owner -> owner.qualifiedName).collect(Collectors.toSet());
      this.reservedPaths = reserved.stream().map(ClassNameAllocator::fold).collect(Collectors.toSet());
      packages.forEach(this::addPackage);
    }

    void addPackage(String pkg) {
      addPackagePrefixes(pkg, packages);
    }

    void reserve(String original, String target) {
      requests.computeIfAbsent(fold(target), unused -> new HashSet<>()).add(original);
    }

    boolean available(String original, String target, boolean generated) {
      // javac also rejects packages matching a member class's binary name
      // (Outer$Nested), even though its source spelling is Outer.Nested.
      if (!legal(target) || reservedPaths.contains(fold(target)) || packages.contains(target) || externalClass(context, target)) return false;
      if (!target.equals(original) && originals.contains(target)) return false;
      String owner = paths.get(fold(target));
      if (owner != null && !owner.equals(original)) return false;
      Set<String> requested = requests.get(fold(target));
      return !generated || requested == null || requested.size() == 1 && requested.contains(original);
    }

    void add(String original, String target) {
      paths.put(fold(target), original);
      reserve(original, target);
    }

    void remove(String original, String target) {
      if (target != null) {
        paths.remove(fold(target), original);
      }
    }
  }
}
