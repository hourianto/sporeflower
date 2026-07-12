// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.struct.StructClass;

import java.util.HashSet;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

final class CheckedExceptionSupport {
  private CheckedExceptionSupport() {
  }

  static boolean isCaughtByActiveCatches(String thrownException, List<String> activeCatchTypes) {
    for (String catchType : activeCatchTypes) {
      if ("java/lang/Throwable".equals(catchType)) {
        return true;
      }
      if (thrownException.equals(catchType) || isSubtypeOf(thrownException, catchType)) {
        return true;
      }
    }
    return false;
  }

  static boolean isSubtypeOf(String subType, String superType) {
    if (subType == null || superType == null) {
      return false;
    }
    if (subType.equals(superType)) {
      return true;
    }
    if (DecompilerContext.getStructContext().instanceOf(subType, superType)) {
      return true;
    }
    if (TargetRuntimeResolver.hasSuppliedRuntimeProfile()) {
      return false;
    }
    return isSubtypeByReflection(subType, superType);
  }

  static boolean isCheckedExceptionType(String exceptionType) {
    if (exceptionType == null) {
      return false;
    }

    if (isSubtypeOf(exceptionType, "java/lang/RuntimeException") || isSubtypeOf(exceptionType, "java/lang/Error")) {
      return false;
    }
    ThrowableKind throwableKind = classifyThrowableType(exceptionType);
    if (throwableKind == ThrowableKind.THROWABLE) {
      return true;
    }
    if (throwableKind == ThrowableKind.NOT_THROWABLE) {
      return false;
    }

    // Obfuscated or missing dependencies sometimes leave no usable hierarchy.
    // Restrict the naming heuristic to that genuinely unknown case.
    String simpleName = exceptionType.substring(exceptionType.lastIndexOf('/') + 1);
    if (simpleName.endsWith("RuntimeException") || simpleName.endsWith("Error")) {
      return false;
    }
    return simpleName.endsWith("Exception");
  }

  static boolean isDeclaredCheckedExceptionType(String exceptionType) {
    if (exceptionType == null) {
      return false;
    }
    if (isSubtypeOf(exceptionType, "java/lang/RuntimeException") || isSubtypeOf(exceptionType, "java/lang/Error")) {
      return false;
    }
    // Exceptions attributes and catch tables guarantee a throwable type. When
    // its dependency hierarchy is absent, conservatively retain it instead of
    // dropping a custom checked exception merely because its name lacks a suffix.
    return classifyThrowableType(exceptionType) != ThrowableKind.NOT_THROWABLE;
  }

  static boolean needsDeclaredCheckedThrowForCatchReachability(String catchType) {
    if (!isDeclaredCheckedExceptionType(catchType)) {
      return false;
    }

    // javac accepts catch (Exception) and catch (Throwable) even when the try
    // body has no declared checked throws: unchecked RuntimeException/Error
    // flows are always possible and are assignment-compatible with those types.
    if ("java/lang/Exception".equals(catchType) || "java/lang/Throwable".equals(catchType)) {
      return false;
    }
    return !isUncheckedThrowableAssignableTo(catchType);
  }

  static boolean isCoveredBy(String exceptionType, List<String> declarations) {
    for (String declaration : declarations) {
      if (isSubtypeOf(exceptionType, declaration)) {
        return true;
      }
    }
    return false;
  }

  static boolean isShadowedBy(String candidateType, List<String> previousCatchTypes) {
    for (String previousType : previousCatchTypes) {
      if (candidateType.equals(previousType) || isSubtypeOf(candidateType, previousType)) {
        return true;
      }
    }
    return false;
  }

  static List<String> removeRedundantSubtypes(List<String> exceptionTypes) {
    if (exceptionTypes.size() < 2) {
      return exceptionTypes;
    }
    List<String> result = new ArrayList<>();
    outer:
    for (String exceptionType : exceptionTypes) {
      for (String other : exceptionTypes) {
        if (!exceptionType.equals(other) && isSubtypeOf(exceptionType, other)) {
          continue outer;
        }
      }
      result.add(exceptionType);
    }
    return result;
  }

  static List<String> sortMostSpecificFirst(List<String> exceptionTypes) {
    if (exceptionTypes.size() < 2) {
      return exceptionTypes;
    }
    // Subtyping is a partial order, so a comparator that equates unrelated
    // types is not transitive. A stable topological order keeps subtypes first.
    List<String> remaining = new ArrayList<>(new LinkedHashSet<>(exceptionTypes));
    List<String> sorted = new ArrayList<>(remaining.size());
    while (!remaining.isEmpty()) {
      int selected = 0;
      for (int i = 0; i < remaining.size(); i++) {
        String candidate = remaining.get(i);
        boolean hasSubtype = false;
        for (int j = 0; j < remaining.size(); j++) {
          if (i != j && isSubtypeOf(remaining.get(j), candidate)) {
            hasSubtype = true;
            break;
          }
        }
        if (!hasSubtype) {
          selected = i;
          break;
        }
      }
      sorted.add(remaining.remove(selected));
    }
    return sorted;
  }

  private static boolean isUncheckedThrowableAssignableTo(String catchType) {
    return isSubtypeOf("java/lang/RuntimeException", catchType) || isSubtypeOf("java/lang/Error", catchType);
  }

  static String toBinaryName(String internalName) {
    return internalName.replace('/', '.');
  }

  private static boolean isSubtypeByReflection(String subType, String superType) {
    try {
      Class<?> thrown = Class.forName(toBinaryName(subType), false, CheckedExceptionSupport.class.getClassLoader());
      Class<?> caught = Class.forName(toBinaryName(superType), false, CheckedExceptionSupport.class.getClassLoader());
      return caught.isAssignableFrom(thrown);
    }
    catch (ReflectiveOperationException | LinkageError ignored) {
      return false;
    }
  }

  private static ThrowableKind classifyThrowableType(String className) {
    StructClass current = DecompilerContext.getStructContext().getClass(className);
    String reflectionName = className;
    if (current != null) {
      Set<String> visited = new HashSet<>();
      while (current != null && visited.add(current.qualifiedName)) {
        if ("java/lang/Throwable".equals(current.qualifiedName)) {
          return ThrowableKind.THROWABLE;
        }
        if (current.superClass == null) {
          return ThrowableKind.NOT_THROWABLE;
        }
        String superName = current.superClass.getString();
        current = DecompilerContext.getStructContext().getClass(superName);
        if (current == null) {
          reflectionName = superName;
          break;
        }
      }
    }

    if (TargetRuntimeResolver.hasSuppliedRuntimeProfile()) {
      return ThrowableKind.UNKNOWN;
    }

    try {
      Class<?> resolved = Class.forName(toBinaryName(reflectionName), false, CheckedExceptionSupport.class.getClassLoader());
      return Throwable.class.isAssignableFrom(resolved) ? ThrowableKind.THROWABLE : ThrowableKind.NOT_THROWABLE;
    }
    catch (ReflectiveOperationException | LinkageError ignored) {
      return ThrowableKind.UNKNOWN;
    }
  }

  private enum ThrowableKind {
    THROWABLE,
    NOT_THROWABLE,
    UNKNOWN
  }
}
