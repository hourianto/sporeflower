// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.main.rels;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.HashSet;
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
}
