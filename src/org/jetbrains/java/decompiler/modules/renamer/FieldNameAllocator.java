package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.NamingPlan.Member;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer.Type;
import org.jetbrains.java.decompiler.main.rels.SourceFieldScope;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExprUtil;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructField;
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
import java.util.function.UnaryOperator;
import java.util.stream.Collectors;

/** Allocates distinct field identities across their source and lexical scopes. */
final class FieldNameAllocator {
  private final StructContext context;
  private final MemberNamePolicy policy;
  private final List<StructClass> classOrder;
  private final ClassNameAllocator classes;
  private final PoolInterceptor interceptor;

  private FieldNameAllocator(StructContext context, MemberNamePolicy policy, List<StructClass> classOrder,
                             ClassNameAllocator classes, PoolInterceptor interceptor) {
    this.context = context;
    this.policy = policy;
    this.classOrder = classOrder;
    this.classes = classes;
    this.interceptor = interceptor;
  }

  static void allocate(StructContext context, MemberNamePolicy policy, List<StructClass> classOrder,
                       ClassNameAllocator classes, PoolInterceptor interceptor, Map<Member, String> names) {
    new FieldNameAllocator(context, policy, classOrder, classes, interceptor).allocate(names);
  }

  private void allocate(Map<Member, String> names) {
    Map<StructField, FieldReference> references = new IdentityHashMap<>();
    List<FieldReference> ordered = new ArrayList<>();
    for (StructClass owner : classOrder) {
      if (!owner.isOwn()) continue;
      for (StructField field : owner.getFields()) {
        FieldReference ref = new FieldReference(owner, field);
        ref.choice = policy.choose(Type.ELEMENT_FIELD, ref.member());
        ref.name = ref.choice.requestedName();
        references.put(field, ref);
        ordered.add(ref);
      }
    }

    Map<StructClass, FieldScope> visible = collectScopes(context, interceptor, references, ordered);
    // A replacement must not obscure a type used earlier in this or another
    // lexical scope. Reserve qualifier tokens before choosing any fallback.
    for (StructClass owner : context.getOwnClasses()) {
      List<FieldScope> lexicalScopes = lexicalFieldScopes(owner, visible);
      for (String qualifier : visible.get(owner).qualifiers) {
        SourceQualifier target = sourceQualifier(qualifier, classes::name, classes.innerNames());
        for (FieldScope scope : lexicalScopes) {
          scope.names.reserve(target.simple);
          if (target.packageRoot != null) scope.names.reserve(target.packageRoot);
        }
      }
    }
    NameAllocator.allocate(ordered, Comparator.comparing(ref -> ref.ownerClass.hasModifier(CodeConstants.ACC_INTERFACE)), this::nextFieldName);
    Set<String> fieldNames = ordered.stream().map(ref -> ref.name).collect(Collectors.toCollection(HashSet::new));
    resolveQualifierConflicts(visible, fieldNames);
    for (FieldReference ref : ordered) {
      if (!ref.fixed) names.put(ref.member(), ref.name);
    }
  }

  private String nextFieldName(FieldReference ref) {
    return ref.choice.nextName();
  }

  /** Prepared names must satisfy the same conservative scopes as automatic allocation. */
  static void validate(NamingPlan plan, StructContext context, PoolInterceptor interceptor) {
    Map<StructField, FieldReference> references = new IdentityHashMap<>();
    List<FieldReference> ordered = new ArrayList<>();
    for (StructClass owner : context.getOwnClasses()) {
      for (StructField field : owner.getFields()) {
        FieldReference ref = new FieldReference(owner, field);
        ref.name = plan.fields().get(ref.member());
        references.put(field, ref);
        ordered.add(ref);
      }
    }
    UnaryOperator<String> className = original -> plan.classes().getOrDefault(original, original);
    Map<StructClass, FieldScope> visible = collectScopes(context, interceptor, references, ordered);
    for (StructClass owner : context.getOwnClasses()) {
      Map<String, FieldReference> occupied = new LinkedHashMap<>();
      for (FieldReference field : visible.get(owner).fields) {
        FieldReference previous = occupied.putIfAbsent(field.name, field);
        if (previous != null && (!previous.fixed || !field.fixed)) {
          throw new IllegalArgumentException("Prepared field names collide in source scope: " + owner.qualifiedName + "." + field.name);
        }
      }
      Set<String> fields = new HashSet<>();
      Set<String> fixedFields = new HashSet<>();
      for (FieldScope scope : lexicalFieldScopes(owner, visible)) {
        for (FieldReference field : scope.fields) {
          fields.add(field.name);
          if (field.fixed) fixedFields.add(field.name);
        }
      }
      for (String qualifier : visible.get(owner).qualifiers) {
        SourceQualifier target = sourceQualifier(qualifier, className, plan.innerNames());
        if (!target.isObscured(fields)) continue;
        if (!plan.classes().containsKey(qualifier) && target.isObscured(fixedFields)) {
          warnFixedQualifier(className.apply(qualifier), owner.qualifiedName, target.simple);
        } else throw new IllegalArgumentException("Prepared field names obscure class " + className.apply(qualifier) + " in " + owner.qualifiedName);
      }
    }
  }

  private static Map<StructClass, FieldScope> collectScopes(StructContext context, PoolInterceptor interceptor,
                                                            Map<StructField, FieldReference> references,
                                                            List<FieldReference> ordered) {
    // Field identities never merge. Each declaration reserves a name in every
    // class where it is visible, using field-specific package-path access rules.
    Map<StructClass, FieldScope> visible = new IdentityHashMap<>();
    for (StructClass owner : context.getOwnClasses()) {
      Set<FieldReference> fields = new LinkedHashSet<>();
      SourceFieldScope.visit(context, owner, UnaryOperator.identity(), (declaringClass, field) -> {
        FieldReference ref = references.get(field);
        if (ref == null) {
          ref = new FieldReference(declaringClass, field);
          references.put(field, ref);
          ordered.add(ref);
        }
        fields.add(ref);
      });
      visible.put(owner, new FieldScope(new ArrayList<>(fields), collectStaticQualifiers(context, interceptor, owner),
        NameAllocator.reserve(fields)));
    }
    return visible;
  }

  private static Set<String> collectStaticQualifiers(StructContext context, PoolInterceptor interceptor, StructClass owner) {
    Set<String> qualifiers = new LinkedHashSet<>();
    for (PooledConstant constant : owner.getPool().getPool()) {
      if (!(constant instanceof LinkConstant link)) continue;
      boolean isField = link.type == CodeConstants.CONSTANT_Fieldref;
      if (!isField && link.type != CodeConstants.CONSTANT_Methodref && link.type != CodeConstants.CONSTANT_InterfaceMethodref) continue;
      String declaration = interceptor.originalMemberOwner(link, isField);
      StructClass declaringClass = declaration == null ? null : context.getClass(declaration);
      if (declaringClass == null) continue;
      var member = isField ? declaringClass.getField(link.elementname, link.descriptor) : declaringClass.getMethod(link.elementname, link.descriptor);
      if (member == null || !member.hasModifier(CodeConstants.ACC_STATIC)) continue;
      if (!declaringClass.isOwn() && ExprUtil.PRIMITIVE_TYPES.containsKey(link.classname)) {
        // FieldExprent renders these unchanged library fields as int.class,
        // void.class, etc.; no expression qualifier remains to be obscured.
        if (isField && link.elementname.equals("TYPE")) continue;
      }
      ClassNode qualifier = DecompilerContext.getClassProcessor().getMapRootClasses().get(link.classname);
      while (qualifier != null && qualifier.type == ClassNode.Type.MEMBER && qualifier.parent != null) qualifier = qualifier.parent;
      qualifiers.add(qualifier == null ? link.classname : qualifier.classStruct.qualifiedName);
    }
    return qualifiers;
  }

  private static List<FieldScope> lexicalFieldScopes(StructClass owner, Map<StructClass, FieldScope> visible) {
    List<FieldScope> scopes = new ArrayList<>();
    scopes.add(visible.get(owner));
    ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(owner.qualifiedName);
    for (ClassNode parent = node == null ? null : node.parent; parent != null; parent = parent.parent) {
      FieldScope scope = visible.get(parent.classStruct);
      if (scope != null) scopes.add(scope);
    }
    return scopes;
  }

  private void resolveQualifierConflicts(Map<StructClass, FieldScope> visible, Set<String> fieldNames) {
    for (StructClass owner : context.getOwnClasses()) {
      Set<FieldReference> lexicalFields = new LinkedHashSet<>();
      for (FieldScope scope : lexicalFieldScopes(owner, visible)) lexicalFields.addAll(scope.fields);
      Map<String, List<FieldReference>> fields = new LinkedHashMap<>();
      for (FieldReference field : lexicalFields) {
        fields.computeIfAbsent(field.name, unused -> new ArrayList<>()).add(field);
      }
      for (String qualifierOwner : visible.get(owner).qualifiers) {
        String target = classes.name(qualifierOwner);
        SourceQualifier qualifier = sourceQualifier(qualifierOwner, classes::name, classes.innerNames());
        String simple = qualifier.simple;
        List<FieldReference> shadowing = fields.get(simple);
        if (!qualifier.isObscured(fields.keySet())) continue;
        // Neither a simple nor a package-qualified expression can name this type.
        if (shadowing.stream().anyMatch(field -> field.fixed)) {
          StructClass targetClass = context.getClass(qualifierOwner);
          if (targetClass != null && targetClass.isOwn()) {
            classes.moveObscuredClass(qualifierOwner, fieldNames);
            continue;
          }
          if (qualifier.packageRoot == null || fields.get(qualifier.packageRoot).stream().anyMatch(field -> field.fixed)) {
            warnFixedQualifier(target, owner.qualifiedName, simple);
            continue;
          }
          // The type and its simple-name blocker are fixed, but renaming an
          // own field can still make the fully qualified spelling usable.
          simple = qualifier.packageRoot;
          shadowing = fields.get(simple);
        }
        // Detach the bucket before renaming it; there is no mutation of a list
        // being iterated. Later qualifiers observe the updated lexical name map.
        Set<String> blocked = new HashSet<>(fields.keySet());
        fields.remove(simple);
        for (FieldReference field : shadowing) {
          field.rename(() -> nextFieldName(field), blocked);
          fields.computeIfAbsent(field.name, unused -> new ArrayList<>()).add(field);
          blocked.add(field.name);
          fieldNames.add(field.name);
        }
      }
    }
  }

  private static void warnFixedQualifier(String target, String owner, String simple) {
    DecompilerContext.getLogger().writeMessage("Cannot qualify external class " + target + " in " + owner
      + ": inherited field " + simple + " also obscures its package; preserving library contracts", IFernflowerLogger.Severity.WARN);
  }

  private static SourceQualifier sourceQualifier(String original, UnaryOperator<String> className,
                                                   Map<String, String> innerNames) {
    ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(original);
    String target = className.apply(original);
    if (node != null && node.type == ClassNode.Type.LOCAL) {
      // A local type is named by its source identifier, not Outer$1Local, and
      // has no package-qualified source spelling if that identifier is hidden.
      return new SourceQualifier(innerNames.getOrDefault(original, node.simpleName), null);
    }
    int slash = target.indexOf('/');
    return new SourceQualifier(ConverterHelper.getSimpleClassName(target), slash < 0 ? null : target.substring(0, slash));
  }

  private record SourceQualifier(String simple, String packageRoot) {
    boolean isObscured(Set<String> fields) {
      return fields.contains(simple) && (packageRoot == null || fields.contains(packageRoot));
    }
  }

  private record FieldScope(List<FieldReference> fields, Set<String> qualifiers, NameAllocator.Scope names) { }

  private static final class FieldReference extends NameAllocator.Slot {
    private final StructClass ownerClass;
    private final StructField field;
    private MemberNamePolicy.Choice choice;

    private FieldReference(StructClass ownerClass, StructField field) {
      this.ownerClass = ownerClass;
      this.field = field;
      name = field.getName();
      fixed = !ownerClass.isOwn();
    }

    private Member member() { return new Member(ownerClass.qualifiedName, field.getName(), field.getDescriptor()); }
  }

}
