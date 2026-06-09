// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.plugins.PluginContext;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructExceptionsAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

final class CheckedInvocationResolver {
  @FunctionalInterface
  interface MissingThrowsInferencer {
    List<String> inferMissingCheckedExceptions(StructClass ownerClass, ClassWrapper ownerWrapper, StructMethod method, MethodWrapper methodWrapper);
  }

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

  private static final class MethodResolution {
    final StructClass ownerClass;
    final StructMethod method;

    MethodResolution(StructClass ownerClass, StructMethod method) {
      this.ownerClass = ownerClass;
      this.method = method;
    }
  }

  private final MissingThrowsInferencer missingThrowsInferencer;
  private final Map<String, ClassWrapper> detachedOwnClassWrappers = new HashMap<>();

  CheckedInvocationResolver(MissingThrowsInferencer missingThrowsInferencer) {
    this.missingThrowsInferencer = missingThrowsInferencer;
  }

  List<String> getInvocationCheckedExceptions(InvocationExprent invocation, @Nullable StructClass ownerClass, @Nullable ClassWrapper ownerWrapper) {
    return resolveInvocationThrowInfo(invocation, ownerClass, ownerWrapper).checkedExceptions;
  }

  boolean invocationThrows(
    InvocationExprent invocation,
    String exceptionType,
    @Nullable StructClass ownerClass,
    @Nullable ClassWrapper ownerWrapper
  ) {
    InvocationThrowInfo throwInfo = resolveInvocationThrowInfo(invocation, ownerClass, ownerWrapper);
    for (String checkedType : throwInfo.checkedExceptions) {
      if (CheckedExceptionSupport.isSubtypeOf(checkedType, exceptionType)) {
        return true;
      }
    }
    return throwInfo.unknownThrowability;
  }

  InvocationThrowInfo resolveInvocationThrowInfo(
    InvocationExprent invocation,
    @Nullable StructClass ownerClass,
    @Nullable ClassWrapper ownerWrapper
  ) {
    // Unified call-site throw model used by both catch reachability checks
    // and checked-throws signature synthesis.
    String className = invocation.getClassname();
    if (className == null) {
      return new InvocationThrowInfo(Collections.emptyList(), true);
    }

    StructClass invokedClass = DecompilerContext.getStructContext().getClass(className);
    if (invokedClass == null) {
      if (className.startsWith("java/")) {
        List<String> reflected = getReflectionCheckedExceptionsOrNull(invocation);
        return new InvocationThrowInfo(reflected == null ? Collections.emptyList() : reflected, false);
      }
      return new InvocationThrowInfo(Collections.emptyList(), true);
    }

    MethodResolution resolution = findMethodResolution(invokedClass, invocation);
    if (resolution == null) {
      return new InvocationThrowInfo(Collections.emptyList(), true);
    }

    StructClass methodOwner = resolution.ownerClass;
    StructMethod invokedMethod = resolution.method;
    DeclaredThrowsInfo declaredInfo = getDeclaredThrowsInfo(methodOwner, invokedMethod);
    if (declaredInfo.hasAttribute) {
      return new InvocationThrowInfo(declaredInfo.checkedExceptions, declaredInfo.malformed);
    }

    if (methodOwner.isOwn() && invokedMethod.containsCode()) {
      ClassWrapper invokedClassWrapper = resolveClassWrapper(methodOwner, ownerClass, ownerWrapper);
      MethodWrapper invokedWrapper = invokedClassWrapper == null
        ? null
        : invokedClassWrapper.getMethodWrapper(invokedMethod.getName(), invokedMethod.getDescriptor());
      if (invokedWrapper != null) {
        List<String> inferred = missingThrowsInferencer.inferMissingCheckedExceptions(methodOwner, invokedClassWrapper, invokedMethod, invokedWrapper);
        return new InvocationThrowInfo(inferred, false);
      }
      // The rendered throws clause of this own method will come from inference we
      // cannot perform here, so its source-level declaration is genuinely unknown.
      return new InvocationThrowInfo(Collections.emptyList(), true);
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

    return getOrCreateDetachedClassWrapper(invokedClass);
  }

  static List<String> getDeclaredCheckedExceptions(StructClass ownerClass, StructMethod method) {
    return getDeclaredThrowsInfo(ownerClass, method).checkedExceptions;
  }

  static @Nullable List<String> reflectionMethodCheckedExceptions(String className, String methodName, String descriptor) {
    return reflectionCheckedExceptions(className, methodName, descriptor);
  }

  private @Nullable ClassWrapper getOrCreateDetachedClassWrapper(StructClass invokedClass) {
    if (!invokedClass.isOwn()) {
      return null;
    }

    ClassWrapper cached = detachedOwnClassWrappers.get(invokedClass.qualifiedName);
    if (cached != null) {
      return cached;
    }

    ClassWrapper created = createDetachedClassWrapper(invokedClass);
    if (created != null) {
      detachedOwnClassWrappers.put(invokedClass.qualifiedName, created);
    }
    return created;
  }

  private static @Nullable ClassWrapper createDetachedClassWrapper(StructClass invokedClass) {
    StructClass previousClass = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS);
    ClassWrapper previousWrapper = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS_WRAPPER);
    try {
      ClassWrapper wrapper = new ClassWrapper(invokedClass);
      wrapper.init(PluginContext.getCurrentContext().getLanguageSpec(invokedClass));
      return wrapper;
    }
    catch (Throwable ignored) {
      return null;
    }
    finally {
      DecompilerContext.setProperty(DecompilerContext.CURRENT_CLASS, previousClass);
      DecompilerContext.setProperty(DecompilerContext.CURRENT_CLASS_WRAPPER, previousWrapper);
    }
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
      if (CheckedExceptionSupport.isCheckedExceptionType(exceptionClass)) {
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

  private static @Nullable List<String> getReflectionCheckedExceptionsOrNull(InvocationExprent invocation) {
    return reflectionCheckedExceptions(invocation.getClassname(), invocation.getName(), invocation.getStringDescriptor());
  }

  private static @Nullable List<String> reflectionCheckedExceptions(String className, String methodName, String descriptor) {
    try {
      Class<?> owner = Class.forName(CheckedExceptionSupport.toBinaryName(className), false, CheckedInvocationResolver.class.getClassLoader());
      Class<?>[] params = toParameterClasses(MethodDescriptor.parseDescriptor(descriptor).params);

      List<String> checked = new ArrayList<>();
      if (CodeConstants.INIT_NAME.equals(methodName)) {
        Constructor<?> constructor;
        try {
          constructor = owner.getDeclaredConstructor(params);
        }
        catch (NoSuchMethodException ignored) {
          constructor = owner.getConstructor(params);
        }
        for (Class<?> thrown : constructor.getExceptionTypes()) {
          String internal = thrown.getName().replace('.', '/');
          if (CheckedExceptionSupport.isCheckedExceptionType(internal)) {
            checked.add(internal);
          }
        }
      }
      else {
        Method method;
        try {
          method = owner.getDeclaredMethod(methodName, params);
        }
        catch (NoSuchMethodException ignored) {
          method = owner.getMethod(methodName, params);
        }
        for (Class<?> thrown : method.getExceptionTypes()) {
          String internal = thrown.getName().replace('.', '/');
          if (CheckedExceptionSupport.isCheckedExceptionType(internal)) {
            checked.add(internal);
          }
        }
      }
      return checked;
    }
    catch (ReflectiveOperationException | LinkageError ignored) {
      return null;
    }
  }

  private static Class<?>[] toParameterClasses(VarType[] params) throws ClassNotFoundException {
    Class<?>[] result = new Class<?>[params.length];
    for (int i = 0; i < params.length; i++) {
      result[i] = toClass(params[i]);
    }
    return result;
  }

  private static Class<?> toClass(VarType type) throws ClassNotFoundException {
    if (type.arrayDim > 0) {
      return Class.forName(type.toString().replace('/', '.'), false, CheckedInvocationResolver.class.getClassLoader());
    }

    return switch (type.type) {
      case BOOLEAN -> boolean.class;
      case BYTE -> byte.class;
      case CHAR -> char.class;
      case SHORT -> short.class;
      case INT -> int.class;
      case LONG -> long.class;
      case FLOAT -> float.class;
      case DOUBLE -> double.class;
      case OBJECT -> Class.forName(CheckedExceptionSupport.toBinaryName(type.value), false, CheckedInvocationResolver.class.getClassLoader());
      default -> Class.forName("java.lang.Object", false, CheckedInvocationResolver.class.getClassLoader());
    };
  }
}
