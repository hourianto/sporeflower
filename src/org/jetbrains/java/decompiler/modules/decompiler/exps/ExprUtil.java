// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.exps;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructInnerClassesAttribute;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;

public final class ExprUtil {
  public static final Map<String, String> PRIMITIVE_TYPES = Map.of(
    "java/lang/Boolean", "boolean",
    "java/lang/Byte", "byte",
    "java/lang/Character", "char",
    "java/lang/Short", "short",
    "java/lang/Integer", "int",
    "java/lang/Long", "long",
    "java/lang/Float", "float",
    "java/lang/Double", "double",
    "java/lang/Void", "void"
  );

  public static List<VarVersionPair> getSyntheticParametersMask(String className, String descriptor, int parameters) {
    ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(className);
    return node != null ? getSyntheticParametersMask(node, descriptor, parameters) : null;
  }

  public static List<VarVersionPair> getSyntheticParametersMask(ClassNode node, String descriptor, int parameters) {
    List<VarVersionPair> mask = null;

    ClassWrapper wrapper = node.getWrapper();
    if (wrapper != null) {
      // own class
      MethodWrapper methodWrapper = wrapper.getMethodWrapper(CodeConstants.INIT_NAME, descriptor);
      if (methodWrapper == null) {
        if (DecompilerContext.getOption(IFernflowerPreferences.IGNORE_INVALID_BYTECODE)) {
          return null;
        }
        throw new RuntimeException("Constructor " + node.classStruct.qualifiedName + "." + CodeConstants.INIT_NAME + descriptor + " not found");
      }
      mask = methodWrapper.synthParameters;
    }
    else if (parameters > 0 && node.type == ClassNode.Type.MEMBER && !isStatic(node.classStruct)) {
      // non-static member class
      mask = new ArrayList<>(Collections.nCopies(parameters, null));
      mask.set(0, new VarVersionPair(-1, 0));
    }

    return mask;
  }

  /** Resolves the source constructor behind a trailing null access marker. */
  public static @Nullable StructMethod getSyntheticConstructorTarget(String ownerClassName, MethodDescriptor descriptor) {
    if (descriptor.params.length == 0) {
      return null;
    }

    VarType parameterType = descriptor.params[descriptor.params.length - 1];
    if (parameterType.type != CodeType.OBJECT ||
        parameterType.arrayDim != 0 ||
        parameterType.value == null) {
      return null;
    }

    StructClass owner = DecompilerContext.getStructContext().getClass(ownerClassName);
    if (owner == null) {
      return null;
    }

    StringBuilder sourceDescriptor = new StringBuilder("(");
    for (int i = 0; i < descriptor.params.length - 1; i++) {
      sourceDescriptor.append(descriptor.params[i]);
    }
    sourceDescriptor.append(")V");

    StructMethod sourceConstructor = owner.getMethod(CodeConstants.INIT_NAME, sourceDescriptor.toString());
    ClassNode current = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS_NODE);
    if (sourceConstructor != null && sourceConstructor.hasModifier(CodeConstants.ACC_PRIVATE) &&
        (current == null || !isSameSourceNest(ownerClassName, current.classStruct.qualifiedName))) {
      return null;
    }
    return sourceConstructor != null && (isSyntheticConstructorMarkerType(parameterType.value) ||
      isSyntheticConstructorForwarder(ownerClassName, descriptor, sourceConstructor)) ? sourceConstructor : null;
  }

  private static boolean isSameSourceNest(String firstClass, String secondClass) {
    if (firstClass.equals(secondClass)) {
      return true;
    }
    Map<String, ClassNode> classes = DecompilerContext.getClassProcessor().getMapRootClasses();
    ClassNode first = classes.get(firstClass);
    ClassNode second = classes.get(secondClass);
    if (first == null || second == null) {
      return false;
    }
    while (first.parent != null) {
      first = first.parent;
    }
    while (second.parent != null) {
      second = second.parent;
    }
    return first == second;
  }

  private static boolean isSyntheticConstructorForwarder(String owner, MethodDescriptor descriptor, StructMethod target) {
    ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(owner);
    if (node == null || node.getWrapper() == null) {
      return false;
    }
    MethodWrapper method = node.getWrapper().getMethodWrapper(CodeConstants.INIT_NAME, descriptor.toString());
    if (method == null || !method.methodStruct.isSynthetic() || method.root == null) {
      return false;
    }
    List<Exprent> body = method.root.getFirst().getExprents();
    if (body == null || body.size() != 1 || !(body.get(0) instanceof InvocationExprent call) ||
        call.getFunctype() != InvocationExprent.Type.INIT || !owner.equals(call.getClassname()) ||
        !target.getDescriptor().equals(call.getStringDescriptor()) ||
        call.getLstParameters().size() != descriptor.params.length - 1 ||
        !(call.getInstance() instanceof VarExprent receiver) ||
        !Integer.valueOf(0).equals(method.varproc.getVarOriginalIndex(receiver.getIndex()))) {
      return false;
    }
    // ECJ can use a real member class as its access marker. Its synthetic
    // constructor must do nothing except forward the unchanged prefix parameters;
    // an unused trailing parameter alone is not enough to discard the overload.
    int slot = 1;
    for (int i = 0; i < call.getLstParameters().size(); i++) {
      if (!(call.getLstParameters().get(i) instanceof VarExprent argument)) {
        return false;
      }
      boolean originalParameter = Integer.valueOf(slot).equals(method.varproc.getVarOriginalIndex(argument.getIndex()));
      // Nested-class processing replaces the captured outer parameter with
      // Outer.this; that synthetic variable no longer has an original slot.
      boolean enclosingThis = i == 0 && method.synthParameters != null && method.synthParameters.get(0) != null &&
        descriptor.params[0].value != null &&
        descriptor.params[0].value.equals(method.varproc.getThisVars().get(argument.getVarVersionPair()));
      if (!originalParameter && !enclosingThis) {
        return false;
      }
      slot += descriptor.params[i].stackSize;
    }
    return true;
  }

  public static boolean isSyntheticConstructorMarkerType(String className) {
    if (DecompilerContext.getClassProcessor() == null) {
      return false;
    }

    ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(className);
    if (node == null) {
      return false;
    }

    if (node.type == ClassNode.Type.ANONYMOUS) {
      return true;
    }

    StructClass cl = node.classStruct;
    return cl.isSynthetic() && cl.getFields().isEmpty() && cl.getMethods().isEmpty();
  }

  private static boolean isStatic(StructClass struct) {
    if (struct.hasModifier(CodeConstants.ACC_STATIC))
      return true;
    if (struct.hasAttribute(StructGeneralAttribute.ATTRIBUTE_INNER_CLASSES)) {
      StructInnerClassesAttribute attr = (StructInnerClassesAttribute)struct.getAttribute(StructGeneralAttribute.ATTRIBUTE_INNER_CLASSES);
      for (StructInnerClassesAttribute.Entry entry : attr.getEntries()) {
        if (entry.innerName != null && entry.innerName.equals(struct.qualifiedName)) {
          return (entry.accessFlags & CodeConstants.ACC_STATIC) == CodeConstants.ACC_STATIC;
        }
      }
    }
    return false;
  }
}
