package org.jetbrains.java.decompiler.main.rels;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructField;

import java.util.HashSet;
import java.util.Set;
import java.util.function.BiConsumer;
import java.util.function.UnaryOperator;

/** Fields in one lexical class scope, excluding inaccessible ancestor declarations. */
public final class SourceFieldScope {
  private SourceFieldScope() { }

  public static void visit(StructContext context, StructClass scope, UnaryOperator<String> className,
                           BiConsumer<StructClass, StructField> consumer) {
    visit(context, scope, scope, className, true, new HashSet<>(), consumer);
  }

  private static void visit(StructContext context, StructClass scope, StructClass owner,
                            UnaryOperator<String> className, boolean packagePath, Set<String> visited,
                            BiConsumer<StructClass, StructField> consumer) {
    if (owner == null || !visited.add(owner.qualifiedName + ":" + packagePath)) return;
    boolean samePackage = packagePath && packageName(className.apply(scope.qualifiedName))
      .equals(packageName(className.apply(owner.qualifiedName)));
    for (StructField field : owner.getFields()) {
      if (owner == scope || !field.hasModifier(CodeConstants.ACC_PRIVATE)
          && (field.hasModifier(CodeConstants.ACC_PUBLIC) || field.hasModifier(CodeConstants.ACC_PROTECTED) || samePackage)) {
        consumer.accept(owner, field);
      }
    }
    if (owner.superClass != null) visit(context, scope, context.getClass(owner.superClass.getString()), className, samePackage, visited, consumer);
    for (String iface : owner.getInterfaceNames()) visit(context, scope, context.getClass(iface), className, samePackage, visited, consumer);
  }

  private static String packageName(String name) { return name.substring(0, Math.max(0, name.lastIndexOf('/'))); }
}
