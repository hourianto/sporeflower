package org.jetbrains.java.decompiler.main.rels;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructExceptionsAttribute;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

/** Source-only representation of a JVM class whose constructors were all removed. */
public final class ConstructorlessClassProcessor {
  private ConstructorlessClassProcessor() { }

  public record ConstructorStub(MethodDescriptor descriptor, List<String> exceptions) { }

  public static ConstructorStub getStub(ClassNode node) {
    StructClass cl = node.classStruct;
    if (cl.hasModifier(CodeConstants.ACC_INTERFACE) || cl.hasModifier(CodeConstants.ACC_ENUM)
      || node.type == ClassNode.Type.ANONYMOUS || node.type == ClassNode.Type.LAMBDA
      || cl.getMethods().stream().anyMatch(method -> CodeConstants.INIT_NAME.equals(method.getName()))) {
      return null;
    }

    // Java cannot express zero constructors. Suppress its implicit, callable default
    // constructor with a private, always-throwing one. Copy a real superclass
    // signature instead of inventing a super() overload or made-up argument values.
    if (cl.superClass == null || "java/lang/Object".equals(cl.superClass.getString())) {
      return new ConstructorStub(MethodDescriptor.parseDescriptor("()V"), List.of());
    }
    StructClass parent = DecompilerContext.getStructContext().getClass(cl.superClass.getString());
    if (parent == null) {
      return null; // No evidence for a callable superclass constructor.
    }
    ClassNode parentNode = DecompilerContext.getClassProcessor().getMapRootClasses().get(parent.qualifiedName);
    if (parentNode != null && parentNode.type != ClassNode.Type.ROOT
      && (parentNode.access & CodeConstants.ACC_STATIC) == 0) {
      return null; // A qualified super invocation requires an enclosing instance.
    }

    StructMethod constructor = parent.getMethods().stream()
      .filter(method -> CodeConstants.INIT_NAME.equals(method.getName()) && !method.isSynthetic())
      .filter(method -> isAccessible(method, parent, cl))
      .min(Comparator.comparingInt((StructMethod method) -> method.methodDescriptor().params.length)
        .thenComparing(StructMethod::getDescriptor))
      .orElse(null);
    if (constructor == null) {
      return null;
    }

    List<String> exceptions = new ArrayList<>();
    StructExceptionsAttribute attribute = constructor.getAttribute(StructGeneralAttribute.ATTRIBUTE_EXCEPTIONS);
    if (attribute != null) {
      for (int i = 0; i < attribute.getThrowsExceptions().size(); i++) {
        String exception = attribute.getExcClassname(i, parent.getPool());
        if (exception != null) exceptions.add(exception);
      }
    }
    return new ConstructorStub(constructor.methodDescriptor(), List.copyOf(exceptions));
  }

  private static boolean isAccessible(StructMethod constructor, StructClass parent, StructClass child) {
    if (constructor.hasModifier(CodeConstants.ACC_PRIVATE)) {
      return false;
    }
    return constructor.hasModifier(CodeConstants.ACC_PUBLIC) || constructor.hasModifier(CodeConstants.ACC_PROTECTED)
      || SourceMethodSemantics.packageName(parent.qualifiedName).equals(SourceMethodSemantics.packageName(child.qualifiedName));
  }
}
