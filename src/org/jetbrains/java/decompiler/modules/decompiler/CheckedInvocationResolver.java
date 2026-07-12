// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructExceptionsAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

final class CheckedInvocationResolver {
  static final class InvocationThrowInfo {
    final List<String> checkedExceptions;
    final boolean unknownThrowability;

    InvocationThrowInfo(List<String> checkedExceptions, boolean unknownThrowability) {
      this.checkedExceptions = checkedExceptions;
      this.unknownThrowability = unknownThrowability;
    }
  }

  private static final class DeclaredThrowsInfo {
    final boolean hasAttribute;
    final boolean malformed;
    final List<String> checkedExceptions;

    DeclaredThrowsInfo(boolean hasAttribute, boolean malformed, List<String> checkedExceptions) {
      this.hasAttribute = hasAttribute;
      this.malformed = malformed;
      this.checkedExceptions = checkedExceptions;
    }
  }

  static final class MethodResolution {
    final StructClass ownerClass;
    final StructMethod method;

    MethodResolution(StructClass ownerClass, StructMethod method) {
      this.ownerClass = ownerClass;
      this.method = method;
    }
  }

  record OwnMethodResolution(StructClass ownerClass, ClassWrapper classWrapper, MethodWrapper methodWrapper) { }

  CheckedInvocationResolver() { }

  InvocationThrowInfo resolveInvocationThrowInfo(
    InvocationExprent invocation,
    @Nullable StructClass ownerClass,
    @Nullable ClassWrapper ownerWrapper
  ) {
    // Unified call-site throw model used by both catch reachability checks
    // and checked-throws signature synthesis.
    if (invocation.getInvocationType() == InvocationExprent.InvocationType.DYNAMIC
      || invocation.getInvocationType() == InvocationExprent.InvocationType.CONSTANT_DYNAMIC) {
      // Creating a lambda/constant-dynamic value has no source-level checked
      // throws clause. Bootstrap failures are linkage errors and therefore
      // unchecked; resolving the bootstrap method as an ordinary invocation
      // also confuses initializer extraction.
      return new InvocationThrowInfo(Collections.emptyList(), false);
    }
    String className = invocation.getClassname();
    if (className == null) {
      return new InvocationThrowInfo(Collections.emptyList(), true);
    }

    StructClass invokedClass = DecompilerContext.getStructContext().getClass(className);
    if (invokedClass == null) {
      if (className.startsWith("java/")) {
        TargetRuntimeResolver.MethodThrows platform = TargetRuntimeResolver.resolveMissingPlatformMethod(
          className,
          invocation.getName(),
          invocation.getStringDescriptor()
        );
        return new InvocationThrowInfo(
          platform.checkedExceptions(),
          platform.status() != TargetRuntimeResolver.Status.RESOLVED
        );
      }
      return new InvocationThrowInfo(Collections.emptyList(), true);
    }

    MethodResolution resolution = findMethodResolution(invokedClass, invocation);
    if (resolution == null) {
      ClassWrapper invokedWrapper = resolveClassWrapper(invokedClass, ownerClass, ownerWrapper);
      ClassWrapper.SourceOnlyMethod sourceOnlyMethod = invokedWrapper == null
        ? null
        : invokedWrapper.getSourceOnlyMethod(invocation.getName(), invocation.getStringDescriptor());
      if (sourceOnlyMethod != null) {
        return new InvocationThrowInfo(sourceOnlyMethod.thrownExceptions(), false);
      }
      return new InvocationThrowInfo(Collections.emptyList(), true);
    }

    StructClass methodOwner = resolution.ownerClass;
    StructMethod invokedMethod = resolution.method;
    DeclaredThrowsInfo declaredInfo = getDeclaredThrowsInfo(methodOwner, invokedMethod);
    if (methodOwner.isOwn() && invokedMethod.containsCode()) {
      ClassWrapper invokedClassWrapper = resolveClassWrapper(methodOwner, ownerClass, ownerWrapper);
      MethodWrapper invokedWrapper = invokedClassWrapper == null
        ? null
        : invokedClassWrapper.getMethodWrapper(invokedMethod.getName(), invokedMethod.getDescriptor());
      if (invokedWrapper != null) {
        // Own methods can have a partial Exceptions attribute. Their rendered
        // declaration is the union of bytecode metadata and inferred additions.
        List<String> combined = new ArrayList<>(declaredInfo.checkedExceptions);
        for (String inferred : invokedWrapper.getExceptionSummary().inferredThrows()) {
          if (!combined.contains(inferred)) {
            combined.add(inferred);
          }
        }
        return new InvocationThrowInfo(combined, declaredInfo.malformed);
      }
      return new InvocationThrowInfo(
        declaredInfo.checkedExceptions,
        declaredInfo.malformed || !declaredInfo.hasAttribute
      );
    }

    if (declaredInfo.hasAttribute) {
      return new InvocationThrowInfo(declaredInfo.checkedExceptions, declaredInfo.malformed);
    }

    // Resolved method without an Exceptions attribute. Recompilation sees the very
    // same class (a library jar, or an own abstract/native method rendered without a
    // throws clause), so javac resolves this call as declaring no checked exceptions.
    // Treating it as "could throw anything" would keep checked catches that javac
    // then rejects as unreachable. Trusting the resolved class also intentionally
    // outranks desktop reflection: a provided API jar (e.g. CLDC/MIDP) is what the
    // output gets compiled against, and the desktop JDK may declare more throws.
    return new InvocationThrowInfo(Collections.emptyList(), false);
  }

  @Nullable ClassWrapper resolveClassWrapper(StructClass invokedClass, @Nullable StructClass ownerClass, @Nullable ClassWrapper ownerWrapper) {
    if (ownerClass != null
      && ownerWrapper != null
      && ownerClass.qualifiedName.equals(invokedClass.qualifiedName)) {
      return ownerWrapper;
    }

    ClassesProcessor processor = DecompilerContext.getClassProcessor();
    if (processor == null) {
      return null;
    }

    ClassesProcessor.ClassNode node = processor.getMapRootClasses().get(invokedClass.qualifiedName);
    if (node != null) {
      ClassWrapper wrapper = node.getWrapper();
      if (wrapper != null) {
        return wrapper;
      }
    }

    return null;
  }

  @Nullable OwnMethodResolution resolveOwnMethod(
    InvocationExprent invocation,
    @Nullable StructClass ownerClass,
    @Nullable ClassWrapper ownerWrapper
  ) {
    String className = invocation.getClassname();
    if (className == null) {
      return null;
    }
    StructClass invokedClass = DecompilerContext.getStructContext().getClass(className);
    if (invokedClass == null) {
      return null;
    }
    MethodResolution resolution = findMethodResolution(invokedClass, invocation);
    if (resolution == null || !resolution.ownerClass.isOwn()) {
      return null;
    }
    ClassWrapper wrapper = resolveClassWrapper(resolution.ownerClass, ownerClass, ownerWrapper);
    if (wrapper == null) {
      return null;
    }
    MethodWrapper methodWrapper = wrapper.getMethodWrapper(
      resolution.method.getName(),
      resolution.method.getDescriptor()
    );
    return methodWrapper == null ? null : new OwnMethodResolution(resolution.ownerClass, wrapper, methodWrapper);
  }

  static List<String> getDeclaredCheckedExceptions(StructClass ownerClass, StructMethod method) {
    return getDeclaredThrowsInfo(ownerClass, method).checkedExceptions;
  }

  private static DeclaredThrowsInfo getDeclaredThrowsInfo(StructClass ownerClass, StructMethod method) {
    StructExceptionsAttribute exceptionsAttribute = method.getAttribute(StructGeneralAttribute.ATTRIBUTE_EXCEPTIONS);
    if (exceptionsAttribute == null) {
      return new DeclaredThrowsInfo(false, false, Collections.emptyList());
    }

    boolean malformed = false;
    List<String> checkedExceptions = new ArrayList<>();
    for (int i = 0; i < exceptionsAttribute.getThrowsExceptions().size(); i++) {
      String exceptionClass = exceptionsAttribute.getExcClassname(i, ownerClass.getPool());
      if (exceptionClass == null) {
        malformed = true;
        continue;
      }
      if (CheckedExceptionSupport.isDeclaredCheckedExceptionType(exceptionClass)) {
        checkedExceptions.add(exceptionClass);
      }
    }

    return new DeclaredThrowsInfo(true, malformed, checkedExceptions);
  }

  private static @Nullable MethodResolution findMethodResolution(StructClass cls, InvocationExprent invocation) {
    String invocationName = invocation.getName();
    String descriptor = invocation.getStringDescriptor();

    MethodResolution resolved = findMethodInHierarchy(cls, invocationName, descriptor, new HashSet<>());
    if (resolved != null) {
      return resolved;
    }

    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor == null) {
      return null;
    }

    String mapped = interceptor.getName(cls.qualifiedName + " " + invocationName + " " + descriptor);
    if (mapped == null) {
      return null;
    }

    int firstSpace = mapped.indexOf(' ');
    int secondSpace = mapped.indexOf(' ', firstSpace + 1);
    if (firstSpace <= 0 || secondSpace <= firstSpace) {
      return null;
    }

    String renamedMethodName = mapped.substring(firstSpace + 1, secondSpace);
    return findMethodInHierarchy(cls, renamedMethodName, descriptor, new HashSet<>());
  }

  private static @Nullable MethodResolution findMethodInHierarchy(
    StructClass cls,
    String methodName,
    String descriptor,
    Set<String> visited
  ) {
    if (!visited.add(cls.qualifiedName)) {
      return null;
    }

    StructMethod method = cls.getMethod(methodName, descriptor);
    if (method != null) {
      return new MethodResolution(cls, method);
    }
    if (CodeConstants.INIT_NAME.equals(methodName)) {
      return null;
    }

    if (cls.superClass != null) {
      StructClass superClass = DecompilerContext.getStructContext().getClass(cls.superClass.getString());
      if (superClass != null) {
        MethodResolution fromSuper = findMethodInHierarchy(superClass, methodName, descriptor, visited);
        if (fromSuper != null) {
          return fromSuper;
        }
      }
    }

    for (String interfaceName : cls.getInterfaceNames()) {
      StructClass intf = DecompilerContext.getStructContext().getClass(interfaceName);
      if (intf != null) {
        MethodResolution fromInterface = findMethodInHierarchy(intf, methodName, descriptor, visited);
        if (fromInterface != null) {
          return fromInterface;
        }
      }
    }

    return null;
  }

}
