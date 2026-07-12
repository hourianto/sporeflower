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
   * Finds the declarations inherited by {@code method} under Java source rules.
   * JVM descriptor identity is insufficient here: source signatures ignore the
   * return type, reference returns may be covariant, and package access is
   * evaluated against the class that ultimately declares the override.
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

    MethodIdentity child = originalIdentity(ownerClass, method);
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

    MethodDescriptor childDescriptor = MethodDescriptor.parseDescriptor(child.descriptor());
    for (StructMethod candidate : declaringClass.getMethods()) {
      if (!isInheritedBy(candidate, declaringClass, inheritingClass)) {
        continue;
      }
      MethodIdentity parent = originalIdentity(declaringClass, candidate);
      if (!child.name().equals(parent.name())
        || !parameterDescriptor(child.descriptor()).equals(parameterDescriptor(parent.descriptor()))) {
        continue;
      }
      MethodDescriptor parentDescriptor = MethodDescriptor.parseDescriptor(parent.descriptor());
      if (isReturnOverrideCompatible(context, childDescriptor.ret, parentDescriptor.ret)) {
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

  public static boolean isInheritedBy(
    StructMethod method,
    StructClass declaringClass,
    StructClass inheritingClass
  ) {
    if (!canParticipateInOverride(method)) {
      return false;
    }
    int visibility = method.getAccessFlags()
      & (CodeConstants.ACC_PUBLIC | CodeConstants.ACC_PROTECTED | CodeConstants.ACC_PRIVATE);
    return visibility != 0
      || packageName(declaringClass.qualifiedName).equals(packageName(inheritingClass.qualifiedName));
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
