package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.consts.LinkConstant;
import org.jetbrains.java.decompiler.struct.consts.PooledConstant;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Function;

/** Method contracts and Java visibility shared by allocation and prepared-plan validation. */
final class MethodNameAllocator {
  private MethodNameAllocator() { }

  static void allocate(StructContext context, MemberNamePolicy policy, List<StructClass> classOrder,
                       PoolInterceptor interceptor, Map<Member, String> names) {
    Model model = new Model(context, classOrder, interceptor);
    int[] components = model.components;
    Map<Integer, Family> families = model.families();
    for (Family family : families.values()) {
      if (family.fixed) {
        // Required and forwarding edges connect original same-named methods;
        // the first declaration therefore carries every pinned member's name.
        family.name = family.methods.get(0).method.getName();
      } else {
        family.choice = preferredChoice(family, policy);
        family.name = family.choice.requestedName();
      }
    }
    for (Scope scope : model.scopes) {
      Set<Family> visible = new LinkedHashSet<>();
      for (Reference method : scope.methods) visible.add(families.get(find(components, method.order)));
      if (visible.size() > 1) NameAllocator.reserve(visible);
    }
    NameAllocator.allocate(families.values(), Comparator.comparingInt(MethodNameAllocator::keeperRank),
      family -> family.choice.nextName());
    // Some inputs have conflicting library/native contracts that no renaming
    // can express in Java (for example a visible, pinned return-only bridge).
    // Preserve the contracts and report the limitation instead of aborting all
    // unrelated classes, including when this plan is applied by the toolkit.
    validateScopes(model, families, method -> families.get(find(components, method.order)).name, "Allocated");
    for (Family family : families.values()) {
      for (Reference method : family.methods) {
        if (method.owner.isOwn()) names.put(method.key(), family.name);
      }
    }
  }

  static void validate(NamingPlan plan, StructContext context, List<StructClass> classOrder, PoolInterceptor interceptor) {
    Model model = new Model(context, classOrder, interceptor);
    Function<Reference, String> name = method -> method.owner.isOwn()
      ? plan.methods().get(method.key()) : method.method.getName();
    Map<Integer, Family> families = model.families();
    for (Family family : families.values()) {
      String expected = name.apply(family.methods.get(0));
      for (Reference method : family.methods) {
        String emitted = name.apply(method);
        if (!expected.equals(emitted)) {
          throw new IllegalArgumentException("Prepared mapping splits an override contract: " + method.key());
        }
        if (method.fixed() && !method.method.getName().equals(emitted)) {
          throw new IllegalArgumentException("Prepared mapping renames a library or native contract: " + method.key());
        }
        if (method.owner.isOwn() && (emitted.startsWith("<")
            || ConverterHelper.mustRenameForJava(IIdentifierRenamer.Type.ELEMENT_METHOD, emitted))) {
          reportConflict("Invalid prepared method name: " + method.key() + " -> " + emitted, family.fixed);
        }
      }
    }
    validateScopes(model, families, name, "Prepared");
  }

  private static void validateScopes(Model model, Map<Integer, Family> families, Function<Reference, String> name, String origin) {
    for (Scope scope : model.scopes) {
      Map<String, Set<Integer>> occupied = new LinkedHashMap<>();
      Set<String> ownNames = new HashSet<>();
      Set<String> declarations = new HashSet<>();
      Set<String> collisions = new LinkedHashSet<>();
      for (Reference method : scope.methods) {
        String emitted = name.apply(method);
        // A compiler-regenerated bridge may share its family's source name,
        // but two declarations retained in the output still cannot do so.
        if (method.owner == scope.owner && model.emitted(method, name) && !declarations.add(emitted)) {
          collisions.add(emitted);
        }
        occupied.computeIfAbsent(emitted, unused -> new HashSet<>()).add(find(model.components, method.order));
        if (method.owner.isOwn()) ownNames.add(emitted);
      }
      occupied.forEach((emitted, roots) -> {
        if (roots.size() > 1 && ownNames.contains(emitted)) collisions.add(emitted);
      });
      for (String emitted : collisions) {
        reportConflict(origin + " method names collide in Java: " + scope.owner.qualifiedName + "." + emitted + scope.parameters,
          occupied.get(emitted).stream().allMatch(root -> families.get(root).fixed));
      }
    }
  }

  private static void reportConflict(String message, boolean fixed) {
    if (!fixed) throw new IllegalArgumentException(message);
    DecompilerContext.getLogger().writeMessage(message + "; preserving fixed contracts", IFernflowerLogger.Severity.WARN);
  }

  private static Set<Member> referencedMethods(StructContext context, PoolInterceptor interceptor) {
    Set<Member> referenced = new HashSet<>();
    // A possible pool call conservatively keeps a removable helper visible.
    // Naming never decodes ordinary bodies to prove that a pool entry is used.
    for (StructClass owner : context.getOwnClasses()) {
      for (PooledConstant constant : owner.getPool().getPool()) {
        if (!(constant instanceof LinkConstant link) || link.type != CodeConstants.CONSTANT_Methodref
            && link.type != CodeConstants.CONSTANT_InterfaceMethodref) continue;
        String declaration = interceptor.originalMemberOwner(link, false);
        if (declaration != null) referenced.add(new Member(declaration, link.elementname, link.descriptor));
      }
    }
    return referenced;
  }

  private static boolean hiddenByOptions(StructMethod method) {
    return method.isSynthetic() && DecompilerContext.getOption(IFernflowerPreferences.REMOVE_SYNTHETIC)
      || method.hasModifier(CodeConstants.ACC_BRIDGE) && DecompilerContext.getOption(IFernflowerPreferences.REMOVE_BRIDGE);
  }

  private static MemberNamePolicy.Choice preferredChoice(Family family, MemberNamePolicy policy) {
    Reference preferred = null;
    int priority = Integer.MAX_VALUE;
    for (Reference method : family.methods) {
      int candidatePriority = mappingPriority(method);
      if (candidatePriority < priority && policy.mappedName(IIdentifierRenamer.Type.ELEMENT_METHOD, method.key()) != null) {
        preferred = method;
        priority = candidatePriority;
      }
    }
    if (preferred == null) {
      preferred = family.methods.stream().filter(method -> policy.shouldRename(IIdentifierRenamer.Type.ELEMENT_METHOD, method.key()))
        .findFirst().orElse(family.methods.get(0));
    }
    return policy.choose(IIdentifierRenamer.Type.ELEMENT_METHOD, preferred.key());
  }

  private static int mappingPriority(Reference method) {
    if (method.owner.hasModifier(CodeConstants.ACC_INTERFACE)) return 2;
    return method.method.hasModifier(CodeConstants.ACC_ABSTRACT) ? 1 : 0;
  }

  private static int keeperRank(Family family) {
    int rank = 4;
    for (Reference method : family.methods) {
      if (method.owner.hasModifier(CodeConstants.ACC_INTERFACE)) continue;
      if (method.method.hasModifier(CodeConstants.ACC_FINAL)) return 1;
      rank = Math.min(rank, method.method.hasModifier(CodeConstants.ACC_ABSTRACT) ? 3 : 2);
    }
    return rank;
  }

  private static final class Model {
    private final Set<Member> referencedMethods;
    private final List<Reference> methods = new ArrayList<>();
    private final Map<Reference, Reference> forwardingTargets = new LinkedHashMap<>();
    private final List<Scope> scopes = new ArrayList<>();
    private final int[] components;

    private Model(StructContext context, List<StructClass> classOrder, PoolInterceptor interceptor) {
      Map<StructMethod, Reference> references = new IdentityHashMap<>();
      for (StructClass owner : classOrder) {
        for (StructMethod method : owner.getMethods()) {
          if (!method.getName().startsWith("<") && (owner.isOwn() || !method.hasModifier(CodeConstants.ACC_PRIVATE))) {
            Reference reference = new Reference(owner, method, methods.size());
            methods.add(reference);
            references.put(method, reference);
          }
        }
      }
      referencedMethods = methods.stream().anyMatch(method -> method.owner.isOwn() && hiddenByOptions(method.method))
        ? referencedMethods(context, interceptor) : Set.of();
      components = components(context);
      for (Reference method : methods) {
        Reference target = references.get(SourceMethodSemantics.forwardingBridgeTarget(context, method.owner, method.method));
        if (target != null) forwardingTargets.put(method, target);
        if ((!method.owner.isOwn() || hiddenByOptions(method.method) && !referencedMethods.contains(method.key()))
            && !DecompilerContext.shouldUseLegacySourceCompatibility(method.owner, BytecodeVersion.MAJOR_5)) {
          target = references.get(SourceMethodSemantics.covariantBridgeTarget(context, method.owner, method.method));
          if (target != null) union(components, method.order, target.order);
        }
      }
      for (StructClass owner : context.getOwnClasses()) {
        Map<String, List<Reference>> visible = new LinkedHashMap<>();
        for (StructClass current : NamingHierarchy.ancestors(context, owner)) {
          for (StructMethod method : current.getMethods()) {
            Reference reference = references.get(method);
            if (reference == null || current != owner && !visibleFrom(method, current, owner)) continue;
            // Class allocation preserves package groups and injective type
            // identities, so source scope and parameter equality stay original.
            String parameters = SourceMethodSemantics.parameterDescriptor(method.getDescriptor());
            visible.computeIfAbsent(parameters, unused -> new ArrayList<>()).add(reference);
          }
        }
        visible.forEach((parameters, declarations) -> scopes.add(new Scope(owner, parameters, declarations)));
      }
    }

    private int[] components(StructContext context) {
      int[] components = new int[methods.size()];
      for (int i = 0; i < components.length; i++) components[i] = i;
      Map<String, List<Reference>> byIdentity = new LinkedHashMap<>();
      for (Reference method : methods) {
        if (SourceMethodSemantics.canParticipateInOverride(method.method)) {
          // Return-only overloads remain different JVM contracts. Only a proven
          // forwarding bridge can later coalesce their source declarations.
          String identity = method.method.getName() + " " + method.method.getDescriptor();
          byIdentity.computeIfAbsent(identity, unused -> new ArrayList<>()).add(method);
        }
      }
      for (List<Reference> bucket : byIdentity.values()) {
        for (int i = 0; i < bucket.size(); i++) {
          Reference first = bucket.get(i);
          for (int j = i + 1; j < bucket.size(); j++) {
            Reference second = bucket.get(j);
            if (SourceMethodSemantics.areOverrideRelated(context, first.owner, first.method, second.owner, second.method)) {
              union(components, first.order, second.order);
            }
          }
        }
      }
      return components;
    }

    private Map<Integer, Family> families() {
      Map<Integer, Family> families = new LinkedHashMap<>();
      for (Reference method : methods) {
        Family family = families.computeIfAbsent(find(components, method.order), unused -> new Family());
        family.methods.add(method);
        family.fixed |= method.fixed();
      }
      return families;
    }

    private boolean emitted(Reference method, Function<Reference, String> name) {
      if (!method.owner.isOwn()) return true;
      Reference target = forwardingTargets.get(method);
      if (target != null && !name.apply(method).equals(name.apply(target))) return true;
      return !hiddenByOptions(method.method) || referencedMethods.contains(method.key());
    }

  }

  private static boolean visibleFrom(StructMethod method, StructClass declaring, StructClass owner) {
    return !(declaring.hasModifier(CodeConstants.ACC_INTERFACE) && method.hasModifier(CodeConstants.ACC_STATIC))
      && SourceMethodSemantics.isAccessibleFrom(method, declaring.qualifiedName, owner.qualifiedName);
  }

  private static int find(int[] components, int index) {
    if (components[index] != index) components[index] = find(components, components[index]);
    return components[index];
  }

  private static void union(int[] components, int first, int second) {
    int firstRoot = find(components, first);
    int secondRoot = find(components, second);
    if (firstRoot != secondRoot) components[secondRoot] = firstRoot;
  }

  private static final class Family extends NameAllocator.Slot {
    private final List<Reference> methods = new ArrayList<>();
    private MemberNamePolicy.Choice choice;
  }

  private record Reference(StructClass owner, StructMethod method, int order) {
    private Member key() { return new Member(owner.qualifiedName, method.getName(), method.getDescriptor()); }
    private boolean fixed() { return !owner.isOwn() || method.hasModifier(CodeConstants.ACC_NATIVE); }
  }

  private record Scope(StructClass owner, String parameters, List<Reference> methods) { }
}
