// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.main.rels;

import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Restores the delayed {@link AbstractMethodError} behavior of concrete JVM classes that do not
 * implement every inherited abstract method. Java source rejects such classes at declaration time.
 */
public final class MissingAbstractMethodProcessor {
  private MissingAbstractMethodProcessor() {
  }

  public static void process(ClassNode node) {
    ClassWrapper wrapper = node.getWrapper();
    if (wrapper == null) {
      return;
    }

    StructClass target = wrapper.getClassStruct();
    if (target.hasModifier(CodeConstants.ACC_INTERFACE)
      || target.hasModifier(CodeConstants.ACC_ANNOTATION)
      || target.hasModifier(CodeConstants.ACC_ABSTRACT)) {
      return;
    }

    StructContext context = DecompilerContext.getStructContext();
    Map<String, List<InheritedMethod>> obligations = collectAbstractObligations(target, context);
    for (List<InheritedMethod> group : obligations.values()) {
      InheritedMethod required = chooseMostSpecificReturn(group, context);
      if (required == null || group.stream().anyMatch(method -> !hasErasedSourceSignature(target, method))) {
        continue;
      }

      int requiredAccessRank = requiredAccessRank(group);
      ClassMethod classDeclaration = findClassDeclaration(target, required, requiredAccessRank, context);
      if (classDeclaration != null && !classDeclaration.method.hasModifier(CodeConstants.ACC_ABSTRACT)) {
        preserveImplementationIfOwned(classDeclaration);
        continue;
      }
      // A class declaration, even an abstract one, takes precedence over an
      // otherwise matching interface default.
      if (classDeclaration == null && hasUnambiguousDefaultImplementation(target, required, context)) {
        continue;
      }

      ClassMethod fallbackHost = findAbstractClassFallbackHost(target, group, requiredAccessRank, context);
      if (fallbackHost != null) {
        ClassNode hostNode = DecompilerContext.getClassProcessor().getMapRootClasses().get(fallbackHost.owner.qualifiedName);
        if (hostNode != null && hostNode.getWrapper() != null) {
          hostNode.getWrapper().addAbstractMethodFallback(InterpreterUtil.makeUniqueKey(
            fallbackHost.method.getName(),
            fallbackHost.method.getDescriptor()
          ));
          continue;
        }
      }

      int accessFlags = accessFlagsForRank(requiredAccessRank);
      MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(required.method.getDescriptor());
      ClassWrapper fallbackWrapper = wrapper;
      StructClass legacyInterfaceHost = findLegacyInterfaceFallbackHost(target, group, context);
      if (legacyInterfaceHost != null) {
        ClassNode hostNode = DecompilerContext.getClassProcessor().getMapRootClasses().get(legacyInterfaceHost.qualifiedName);
        if (hostNode != null && hostNode.getWrapper() != null) {
          fallbackWrapper = hostNode.getWrapper();
        }
      }
      fallbackWrapper.addMissingAbstractMethod(new ClassWrapper.MissingAbstractMethod(
        required.method.getName(),
        required.method.getDescriptor(),
        accessFlags,
        descriptor.ret,
        List.of(descriptor.params)
      ));
    }
  }

  private static Map<String, List<InheritedMethod>> collectAbstractObligations(StructClass target, StructContext context) {
    Map<String, List<InheritedMethod>> result = new LinkedHashMap<>();
    for (StructClass owner : collectAncestors(target, context)) {
      for (StructMethod method : owner.getMethods()) {
        if (method.hasModifier(CodeConstants.ACC_ABSTRACT)
          && SourceMethodSemantics.canParticipateInOverride(method)
          && isInheritedBy(target, owner, method)) {
          result.computeIfAbsent(SourceMethodSemantics.sourceSignature(method), ignored -> new ArrayList<>())
            .add(new InheritedMethod(owner, method));
        }
      }
    }

    return result;
  }

  private static InheritedMethod chooseMostSpecificReturn(List<InheritedMethod> methods, StructContext context) {
    for (InheritedMethod candidate : methods) {
      VarType candidateReturn = MethodDescriptor.parseDescriptor(candidate.method.getDescriptor()).ret;
      boolean satisfiesAll = true;
      for (InheritedMethod other : methods) {
        VarType otherReturn = MethodDescriptor.parseDescriptor(other.method.getDescriptor()).ret;
        if (!SourceMethodSemantics.isReturnOverrideCompatible(context, candidateReturn, otherReturn)) {
          satisfiesAll = false;
          break;
        }
      }
      if (satisfiesAll) {
        return candidate;
      }
    }

    DecompilerContext.getLogger().writeMessage(
      "Cannot represent inherited abstract methods with incompatible return types: " + methods,
      IFernflowerLogger.Severity.WARN
    );
    return null;
  }

  private static StructClass findLegacyInterfaceFallbackHost(
    StructClass target,
    List<InheritedMethod> obligations,
    StructContext context
  ) {
    if (obligations.stream().anyMatch(obligation ->
      !obligation.owner.hasModifier(CodeConstants.ACC_INTERFACE)
        || obligation.owner.getVersion().major >= BytecodeVersion.MAJOR_8
    )) {
      return null;
    }

    List<StructClass> classChain = new ArrayList<>();
    for (StructClass current = target.superClass == null ? null : context.getClass(target.superClass.getString());
         current != null;
         current = current.superClass == null ? null : context.getClass(current.superClass.getString())) {
      classChain.add(current);
    }
    Collections.reverse(classChain);

    for (StructClass candidate : classChain) {
      if (!candidate.isOwn() || !candidate.hasModifier(CodeConstants.ACC_ABSTRACT)) {
        continue;
      }
      boolean implementsEveryOwner = obligations.stream().allMatch(obligation ->
        SourceMethodSemantics.isSubtype(context, candidate.qualifiedName, obligation.owner.qualifiedName)
      );
      if (implementsEveryOwner && hasOnlyLegacyOwnedDescendants(candidate, context)) {
        return candidate;
      }
    }
    return null;
  }

  private static boolean hasOnlyLegacyOwnedDescendants(StructClass candidate, StructContext context) {
    // Adding a concrete class method would suppress an otherwise inherited Java 8
    // interface default. Share the failure body only when the complete owned
    // hierarchy predates defaults; mixed-version hierarchies keep it on each leaf.
    return context.getOwnClasses().stream().noneMatch(descendant ->
      descendant.getVersion().major >= BytecodeVersion.MAJOR_8
        && SourceMethodSemantics.isSubtype(context, descendant.qualifiedName, candidate.qualifiedName)
    );
  }

  private static ClassMethod findClassDeclaration(
    StructClass target,
    InheritedMethod required,
    int requiredAccessRank,
    StructContext context
  ) {
    String signature = SourceMethodSemantics.sourceSignature(required.method);
    StructClass current = target;
    while (current != null) {
      for (StructMethod method : current.getMethods()) {
        if (!signature.equals(SourceMethodSemantics.sourceSignature(method))
          || !SourceMethodSemantics.canParticipateInOverride(method)) {
          continue;
        }

        VarType implementationReturn = MethodDescriptor.parseDescriptor(method.getDescriptor()).ret;
        VarType requiredReturn = MethodDescriptor.parseDescriptor(required.method.getDescriptor()).ret;
        if (accessRank(current, method) < requiredAccessRank
          || !SourceMethodSemantics.isReturnOverrideCompatible(context, implementationReturn, requiredReturn)) {
          continue;
        }

        return new ClassMethod(current, method);
      }

      current = current.superClass == null ? null : context.getClass(current.superClass.getString());
    }
    return null;
  }

  private static ClassMethod findAbstractClassFallbackHost(
    StructClass target,
    List<InheritedMethod> obligations,
    int requiredAccessRank,
    StructContext context
  ) {
    StructClass current = target.superClass == null ? null : context.getClass(target.superClass.getString());
    while (current != null) {
      for (InheritedMethod obligation : obligations) {
        if (!obligation.owner.qualifiedName.equals(current.qualifiedName)
          || !current.isOwn()
          || current.hasModifier(CodeConstants.ACC_INTERFACE)
          || accessRank(current, obligation.method) < requiredAccessRank) {
          continue;
        }

        VarType candidateReturn = obligation.returnType();
        boolean satisfiesAll = obligations.stream().allMatch(other ->
          SourceMethodSemantics.isReturnOverrideCompatible(context, candidateReturn, other.returnType())
        );
        if (satisfiesAll) {
          return new ClassMethod(current, obligation.method);
        }
      }
      current = current.superClass == null ? null : context.getClass(current.superClass.getString());
    }
    return null;
  }

  private static boolean hasUnambiguousDefaultImplementation(
    StructClass target,
    InheritedMethod required,
    StructContext context
  ) {
    List<InheritedMethod> matching = collectMatchingInterfaceMethods(target, required, context);
    matching.removeIf(candidate -> matching.stream().anyMatch(other ->
      candidate != other
        && SourceMethodSemantics.isSubtype(context, other.owner.qualifiedName, candidate.owner.qualifiedName)
        && !other.owner.qualifiedName.equals(candidate.owner.qualifiedName)
    ));

    return matching.size() == 1 && !matching.get(0).method.hasModifier(CodeConstants.ACC_ABSTRACT);
  }

  private static List<InheritedMethod> collectMatchingInterfaceMethods(
    StructClass target,
    InheritedMethod required,
    StructContext context
  ) {
    List<InheritedMethod> result = new ArrayList<>();
    String signature = SourceMethodSemantics.sourceSignature(required.method);
    for (StructClass intf : collectAncestors(target, context)) {
      if (!intf.hasModifier(CodeConstants.ACC_INTERFACE)) {
        continue;
      }
      for (StructMethod method : intf.getMethods()) {
        if (signature.equals(SourceMethodSemantics.sourceSignature(method))
          && SourceMethodSemantics.canParticipateInOverride(method)) {
          result.add(new InheritedMethod(intf, method));
        }
      }
    }
    return result;
  }

  private static List<StructClass> collectAncestors(StructClass target, StructContext context) {
    List<StructClass> result = new ArrayList<>();
    ArrayDeque<String> worklist = new ArrayDeque<>();
    Set<String> visited = new HashSet<>();
    if (target.superClass != null) {
      worklist.add(target.superClass.getString());
    }
    for (String interfaceName : target.getInterfaceNames()) {
      worklist.add(interfaceName);
    }

    while (!worklist.isEmpty()) {
      StructClass ancestor = context.getClass(worklist.removeFirst());
      if (ancestor == null || !visited.add(ancestor.qualifiedName)) {
        continue;
      }
      result.add(ancestor);
      if (ancestor.superClass != null) {
        worklist.add(ancestor.superClass.getString());
      }
      for (String interfaceName : ancestor.getInterfaceNames()) {
        worklist.add(interfaceName);
      }
    }
    return result;
  }

  private static boolean hasErasedSourceSignature(StructClass target, InheritedMethod required) {
    Map<VarType, VarType> substitutions = target.getAllGenerics().get(required.owner.qualifiedName);
    if (substitutions == null || substitutions.isEmpty()) {
      return true;
    }

    // Substituting a parameterized inherited signature makes javac create bridge methods.
    // Those bridges can cast arguments or turn an original AME into a successful dispatch,
    // so do not claim semantic preservation for that shape yet.
    return substitutions.entrySet().stream().allMatch(entry -> entry.getKey().equals(entry.getValue()));
  }

  private static boolean isInheritedBy(StructClass target, StructClass owner, StructMethod method) {
    int flags = method.getAccessFlags();
    if ((flags & (CodeConstants.ACC_PUBLIC | CodeConstants.ACC_PROTECTED)) != 0
      || owner.hasModifier(CodeConstants.ACC_INTERFACE)) {
      return true;
    }
    return packageName(target.qualifiedName).equals(packageName(owner.qualifiedName));
  }

  private static int requiredAccessRank(List<InheritedMethod> methods) {
    return methods.stream()
      .mapToInt(method -> accessRank(method.owner, method.method))
      .max()
      .orElse(1);
  }

  private static int accessRank(StructClass owner, StructMethod method) {
    if (owner.hasModifier(CodeConstants.ACC_INTERFACE) || method.hasModifier(CodeConstants.ACC_PUBLIC)) {
      return 3;
    }
    if (method.hasModifier(CodeConstants.ACC_PROTECTED)) {
      return 2;
    }
    return 1;
  }

  private static int accessFlagsForRank(int rank) {
    return rank == 3 ? CodeConstants.ACC_PUBLIC : rank == 2 ? CodeConstants.ACC_PROTECTED : 0;
  }

  private static String packageName(String className) {
    int separator = className.lastIndexOf('/');
    return separator < 0 ? "" : className.substring(0, separator);
  }

  private static void preserveImplementationIfOwned(ClassMethod implementation) {
    if (!implementation.owner.isOwn()) {
      return;
    }
    ClassNode ownerNode = DecompilerContext.getClassProcessor().getMapRootClasses().get(implementation.owner.qualifiedName);
    if (ownerNode != null && ownerNode.getWrapper() != null) {
      ownerNode.getWrapper().requireMethodInSource(InterpreterUtil.makeUniqueKey(
        implementation.method.getName(),
        implementation.method.getDescriptor()
      ));
    }
  }

  private record InheritedMethod(StructClass owner, StructMethod method) {
    private VarType returnType() {
      return MethodDescriptor.parseDescriptor(method.getDescriptor()).ret;
    }
  }

  private record ClassMethod(StructClass owner, StructMethod method) {
  }
}
