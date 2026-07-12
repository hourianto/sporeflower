// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/** Resolves platform capabilities without confusing the host JDK with a supplied target profile. */
final class TargetRuntimeResolver {
  enum Status {
    RESOLVED,
    ABSENT,
    UNKNOWN
  }

  record MethodThrows(Status status, List<String> checkedExceptions) {
    MethodThrows {
      checkedExceptions = List.copyOf(checkedExceptions);
    }

    static MethodThrows unknown() {
      return new MethodThrows(Status.UNKNOWN, List.of());
    }
  }

  private static final Map<String, MethodThrows> HOST_THROWS_CACHE = new ConcurrentHashMap<>();

  private TargetRuntimeResolver() { }

  static MethodThrows resolveMissingPlatformMethod(String className, String methodName, String descriptor) {
    if (hasSuppliedRuntimeProfile()) {
      return MethodThrows.unknown();
    }
    String key = className + ' ' + methodName + ' ' + descriptor;
    return HOST_THROWS_CACHE.computeIfAbsent(
      key,
      ignored -> reflectMethodThrows(className, methodName, descriptor)
    );
  }

  static boolean supportsRuntimeExceptionCause() {
    StructClass runtimeException = DecompilerContext.getStructContext().getClass("java/lang/RuntimeException");
    if (runtimeException != null) {
      return runtimeException.getMethod(CodeConstants.INIT_NAME, "(Ljava/lang/Throwable;)V") != null;
    }
    if (hasSuppliedRuntimeProfile()) {
      return false;
    }
    try {
      RuntimeException.class.getConstructor(Throwable.class);
      return true;
    }
    catch (NoSuchMethodException ignored) {
      return false;
    }
  }

  static boolean hasSuppliedRuntimeProfile() {
    return DecompilerContext.getStructContext().getClass("java/lang/Object") != null;
  }

  private static MethodThrows reflectMethodThrows(String className, String methodName, String descriptor) {
    try {
      Class<?> owner = Class.forName(
        CheckedExceptionSupport.toBinaryName(className),
        false,
        TargetRuntimeResolver.class.getClassLoader()
      );
      Class<?>[] params = toParameterClasses(MethodDescriptor.parseDescriptor(descriptor).params);
      Class<?>[] exceptionTypes;
      if (CodeConstants.INIT_NAME.equals(methodName)) {
        Constructor<?> constructor = findConstructor(owner, params);
        if (constructor == null) {
          return new MethodThrows(Status.ABSENT, List.of());
        }
        exceptionTypes = constructor.getExceptionTypes();
      }
      else {
        Method method = findMethod(owner, methodName, params);
        if (method == null) {
          return new MethodThrows(Status.ABSENT, List.of());
        }
        exceptionTypes = method.getExceptionTypes();
      }

      List<String> checked = new ArrayList<>();
      for (Class<?> exceptionType : exceptionTypes) {
        if (!RuntimeException.class.isAssignableFrom(exceptionType)
          && !Error.class.isAssignableFrom(exceptionType)) {
          checked.add(exceptionType.getName().replace('.', '/'));
        }
      }
      return new MethodThrows(Status.RESOLVED, checked);
    }
    catch (ReflectiveOperationException | LinkageError ignored) {
      return MethodThrows.unknown();
    }
  }

  private static Constructor<?> findConstructor(Class<?> owner, Class<?>[] params) {
    try {
      return owner.getDeclaredConstructor(params);
    }
    catch (NoSuchMethodException ignored) {
      try {
        return owner.getConstructor(params);
      }
      catch (NoSuchMethodException absent) {
        return null;
      }
    }
  }

  private static Method findMethod(Class<?> owner, String name, Class<?>[] params) {
    try {
      return owner.getDeclaredMethod(name, params);
    }
    catch (NoSuchMethodException ignored) {
      try {
        return owner.getMethod(name, params);
      }
      catch (NoSuchMethodException absent) {
        return null;
      }
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
      return Class.forName(type.toString().replace('/', '.'), false, TargetRuntimeResolver.class.getClassLoader());
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
      case OBJECT -> Class.forName(
        CheckedExceptionSupport.toBinaryName(type.value),
        false,
        TargetRuntimeResolver.class.getClassLoader()
      );
      default -> Object.class;
    };
  }
}
