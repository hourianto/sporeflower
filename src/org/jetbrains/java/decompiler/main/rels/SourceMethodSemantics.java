// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.main.rels;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/** Java-source method relationships that are stricter than JVM descriptor identity. */
public final class SourceMethodSemantics {
  private SourceMethodSemantics() {
  }

  public static String sourceSignature(StructMethod method) {
    return sourceSignature(method.getName(), method.getDescriptor());
  }

  public static String sourceSignature(String name, String descriptor) {
    return name + " " + parameterDescriptor(descriptor);
  }

  public static String parameterDescriptor(String descriptor) {
    int end = descriptor.indexOf(')');
    return end >= 0 ? descriptor.substring(0, end + 1) : descriptor;
  }

  public static boolean canParticipateInOverride(StructMethod method) {
    return !CodeConstants.INIT_NAME.equals(method.getName())
      && !CodeConstants.CLINIT_NAME.equals(method.getName())
      && !method.hasModifier(CodeConstants.ACC_PRIVATE)
      && !method.hasModifier(CodeConstants.ACC_STATIC);
  }

  /**
   * Tests families in the supplied classes' current namespace. Call this before
   * applying member renames; recovering original names from a partially populated
   * interceptor would confuse mappings whose targets are other original names.
   */
  public static boolean areOverrideRelated(
    StructContext context,
    StructClass firstOwner,
    StructMethod first,
    StructClass secondOwner,
    StructMethod second
  ) {
    if (!canParticipateInOverride(first) || !canParticipateInOverride(second)
      || !sourceSignature(first).equals(sourceSignature(second))) {
      return false;
    }
    if (firstOwner.qualifiedName.equals(secondOwner.qualifiedName)) {
      return first.getDescriptor().equals(second.getDescriptor());
    }
    MethodIdentity firstIdentity = new MethodIdentity(first.getName(), first.getDescriptor());
    MethodIdentity secondIdentity = new MethodIdentity(second.getName(), second.getDescriptor());
    if (isSubtype(context, firstOwner.qualifiedName, secondOwner.qualifiedName)) {
      return isOverridableFrom(second, secondOwner, firstOwner)
        && hasOverrideSignature(context, firstIdentity, secondIdentity);
    }
    if (isSubtype(context, secondOwner.qualifiedName, firstOwner.qualifiedName)) {
      return isOverridableFrom(first, firstOwner, secondOwner)
        && hasOverrideSignature(context, secondIdentity, firstIdentity);
    }

    boolean firstInterface = firstOwner.hasModifier(CodeConstants.ACC_INTERFACE);
    if (firstInterface == secondOwner.hasModifier(CodeConstants.ACC_INTERFACE)) {
      return false;
    }
    StructClass interfaceOwner = firstInterface ? firstOwner : secondOwner;
    StructClass implementationOwner = firstInterface ? secondOwner : firstOwner;
    StructMethod implementation = firstInterface ? second : first;
    VarType interfaceReturn = MethodDescriptor.parseDescriptor((firstInterface ? first : second).getDescriptor()).ret;
    VarType implementationReturn = MethodDescriptor.parseDescriptor(implementation.getDescriptor()).ret;
    boolean alreadyImplements = implementation.hasModifier(CodeConstants.ACC_PUBLIC)
      && isReturnOverrideCompatible(context, implementationReturn, interfaceReturn);
    boolean canOverride = !implementation.hasModifier(CodeConstants.ACC_FINAL)
      && (isReturnOverrideCompatible(context, implementationReturn, interfaceReturn)
        || isReturnOverrideCompatible(context, interfaceReturn, implementationReturn));
    if (!alreadyImplements && !canOverride) {
      return false;
    }
    // An inherited class method may satisfy an interface introduced farther
    // down the hierarchy. An incomplete class can also need a generated method
    // with the narrower return, but only if it can override the class method.
    for (StructClass child : context.getOwnClasses()) {
      if (!child.hasModifier(CodeConstants.ACC_INTERFACE)
        && isSubtype(context, child.qualifiedName, implementationOwner.qualifiedName)
        && isSubtype(context, child.qualifiedName, interfaceOwner.qualifiedName)
        && (alreadyImplements || isOverridableFrom(implementation, implementationOwner, child))) {
        return true;
      }
    }
    return false;
  }

  private static boolean hasOverrideSignature(StructContext context, MethodIdentity child, MethodIdentity parent) {
    return child.name().equals(parent.name())
      && parameterDescriptor(child.descriptor()).equals(parameterDescriptor(parent.descriptor()))
      && isReturnOverrideCompatible(context,
        MethodDescriptor.parseDescriptor(child.descriptor()).ret,
        MethodDescriptor.parseDescriptor(parent.descriptor()).ret);
  }

  /**
   * Finds the declarations inherited by {@code method} under Java source rules.
   * JVM descriptor identity is insufficient here: source signatures ignore the
   * return type, reference returns may be covariant, and package access is
   * evaluated against the class that ultimately declares the override. Use the
   * current declarations: undoing a conflict rename here can invent an override
   * that no longer exists in the emitted source.
   */
  public static List<InheritedMethod> findOverriddenMethods(
    StructContext context,
    StructClass ownerClass,
    StructMethod method
  ) {
    return findOverrideHierarchy(context, ownerClass, method).methods();
  }

  public static OverrideHierarchy findOverrideHierarchy(
    StructContext context,
    StructClass ownerClass,
    StructMethod method
  ) {
    if (!canParticipateInOverride(method)) {
      return new OverrideHierarchy(List.of(), List.of());
    }

    MethodIdentity child = new MethodIdentity(method.getName(), method.getDescriptor());
    List<InheritedMethod> result = new ArrayList<>();
    List<String> unresolvedAncestors = new ArrayList<>();
    Set<String> visitedClasses = new HashSet<>();
    if (ownerClass.superClass != null) {
      collectInheritedMethods(
        context,
        ownerClass.superClass.getString(),
        ownerClass,
        child,
        visitedClasses,
        result,
        unresolvedAncestors
      );
    }
    for (String interfaceName : ownerClass.getInterfaceNames()) {
      collectInheritedMethods(context, interfaceName, ownerClass, child, visitedClasses, result, unresolvedAncestors);
    }
    return new OverrideHierarchy(result, unresolvedAncestors);
  }

  private static void collectInheritedMethods(
    StructContext context,
    String className,
    StructClass inheritingClass,
    MethodIdentity child,
    Set<String> visitedClasses,
    List<InheritedMethod> result,
    List<String> unresolvedAncestors
  ) {
    StructClass declaringClass = resolveClass(context, className);
    if (declaringClass == null) {
      if (!unresolvedAncestors.contains(className)) {
        unresolvedAncestors.add(className);
      }
      return;
    }
    if (!visitedClasses.add(declaringClass.qualifiedName)) {
      return;
    }

    for (StructMethod candidate : declaringClass.getMethods()) {
      if (!isOverridableFrom(candidate, declaringClass, inheritingClass)) {
        continue;
      }
      MethodIdentity parent = new MethodIdentity(candidate.getName(), candidate.getDescriptor());
      if (hasOverrideSignature(context, child, parent)) {
        result.add(new InheritedMethod(declaringClass, candidate));
      }
    }

    if (declaringClass.superClass != null) {
      collectInheritedMethods(
        context,
        declaringClass.superClass.getString(),
        inheritingClass,
        child,
        visitedClasses,
        result,
        unresolvedAncestors
      );
    }
    for (String interfaceName : declaringClass.getInterfaceNames()) {
      collectInheritedMethods(
        context,
        interfaceName,
        inheritingClass,
        child,
        visitedClasses,
        result,
        unresolvedAncestors
      );
    }
  }

  /**
   * Access/finality check for a declaration in an ancestor (ancestry is checked
   * by the caller). Package access is relative to the overriding declaration:
   * a subclass back in the original package can override across a foreign
   * intermediate superclass, even though it does not inherit that method.
   * Transitive override families are connected through intervening declarations.
   */
  public static boolean isOverridableFrom(
    StructMethod method,
    StructClass declaringClass,
    StructClass inheritingClass
  ) {
    return canParticipateInOverride(method)
      && !method.hasModifier(CodeConstants.ACC_FINAL)
      && isAccessibleFrom(method, declaringClass.qualifiedName, inheritingClass.qualifiedName);
  }

  /** Member access only; callers establish ancestry and exclude non-inherited interface statics. */
  public static boolean isAccessibleFrom(StructMethod method, String declaringClass, String inheritingClass) {
    return !method.hasModifier(CodeConstants.ACC_PRIVATE)
      && (method.hasModifier(CodeConstants.ACC_PUBLIC) || method.hasModifier(CodeConstants.ACC_PROTECTED)
        || packageName(declaringClass).equals(packageName(inheritingClass)));
  }

  public static boolean isReturnOverrideCompatible(StructContext context, VarType childReturn, VarType parentReturn) {
    if (childReturn.equals(parentReturn)) {
      return true;
    }

    if (isReferenceType(childReturn) && isReferenceType(parentReturn)) {
      if (parentReturn.higherEqualInLatticeThan(childReturn)) {
        return true;
      }

      if (childReturn.arrayDim == 0 && parentReturn.arrayDim == 0
        && childReturn.type == CodeType.OBJECT && parentReturn.type == CodeType.OBJECT) {
        return isSubtype(context, childReturn.value, parentReturn.value);
      }
    }

    return false;
  }

  public static boolean isSubtype(StructContext context, String child, String parent) {
    return isSubtype(context, child, parent, new HashSet<>());
  }

  private static boolean isSubtype(StructContext context, String child, String parent, Set<String> visited) {
    if (child == null || parent == null) {
      return false;
    }
    if (child.equals(parent)) {
      return true;
    }
    if (!visited.add(child)) {
      return false;
    }

    StructClass cls = context.getClass(child);
    if (cls != null) {
      if (cls.superClass != null && isSubtype(context, cls.superClass.getString(), parent, visited)) {
        return true;
      }
      for (String interfaceName : cls.getInterfaceNames()) {
        if (isSubtype(context, interfaceName, parent, visited)) {
          return true;
        }
      }
    }

    return context.instanceOf(child, parent);
  }

  private static boolean isReferenceType(VarType type) {
    return type.type == CodeType.OBJECT || type.arrayDim > 0;
  }

  public static String packageName(String className) {
    int split = className.lastIndexOf('/');
    return split < 0 ? "" : className.substring(0, split);
  }

  private static StructClass resolveClass(StructContext context, String className) {
    StructClass cls = context.getClass(className);
    if (cls != null) {
      return cls;
    }

    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor == null) {
      return null;
    }
    String originalName = interceptor.getOldName(className);
    if (originalName != null && (cls = context.getClass(originalName)) != null) {
      return cls;
    }
    String currentName = interceptor.getName(className);
    return currentName == null ? null : context.getClass(currentName);
  }

  private static MethodIdentity originalIdentity(StructClass ownerClass, StructMethod method) {
    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor != null) {
      String oldName = interceptor.getOldName(
        ownerClass.qualifiedName + " " + method.getName() + " " + method.getDescriptor());
      MethodIdentity original = parseMappedIdentity(oldName);
      if (original != null) {
        return original;
      }
    }
    return new MethodIdentity(method.getName(), method.getDescriptor());
  }

  private static @Nullable MethodIdentity parseMappedIdentity(@Nullable String mappedIdentity) {
    if (mappedIdentity == null) {
      return null;
    }
    int first = mappedIdentity.indexOf(' ');
    int second = first < 0 ? -1 : mappedIdentity.indexOf(' ', first + 1);
    if (first <= 0 || second <= first) {
      return null;
    }
    return new MethodIdentity(
      mappedIdentity.substring(first + 1, second),
      mappedIdentity.substring(second + 1)
    );
  }

  public record InheritedMethod(StructClass ownerClass, StructMethod method) { }

  public record OverrideHierarchy(List<InheritedMethod> methods, List<String> unresolvedAncestors) {
    public OverrideHierarchy {
      methods = List.copyOf(methods);
      unresolvedAncestors = List.copyOf(unresolvedAncestors);
    }
  }

  public static String sourceName(StructClass ownerClass, StructMethod method) {
    return originalIdentity(ownerClass, method).name();
  }

  public static String sourceDescriptor(StructClass ownerClass, StructMethod method) {
    return originalIdentity(ownerClass, method).descriptor();
  }

  private record MethodIdentity(String name, String descriptor) { }
}
