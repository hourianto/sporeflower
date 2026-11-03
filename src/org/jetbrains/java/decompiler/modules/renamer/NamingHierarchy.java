package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/** Deterministic ancestor-first traversal without a second inheritance graph. */
final class NamingHierarchy {
  private NamingHierarchy() { }

  static List<StructClass> order(StructContext context) {
    List<StructClass> roots = new ArrayList<>(context.getOwnClasses());
    roots.sort(Comparator.comparing(owner -> !owner.hasModifier(CodeConstants.ACC_INTERFACE)));
    return ancestors(context, roots);
  }

  static List<StructClass> ancestors(StructContext context, StructClass owner) {
    return ancestors(context, List.of(owner));
  }

  private static List<StructClass> ancestors(StructContext context, List<StructClass> roots) {
    List<StructClass> result = new ArrayList<>();
    Set<String> visited = new HashSet<>();
    ArrayDeque<Visit> pending = new ArrayDeque<>();
    for (StructClass root : roots) {
      pending.push(new Visit(root, false));
      while (!pending.isEmpty()) {
        Visit visit = pending.pop();
        StructClass owner = visit.owner;
        if (visit.expanded) {
          result.add(owner);
        } else if (visited.add(owner.qualifiedName)) {
          pending.push(new Visit(owner, true));
          // Stack in reverse visitation order: superclass, then declared interfaces.
          String[] interfaces = owner.getInterfaceNames();
          for (int i = interfaces.length - 1; i >= 0; i--) push(context.getClass(interfaces[i]), pending);
          if (owner.superClass != null) push(context.getClass(owner.superClass.getString()), pending);
        }
      }
    }
    return result;
  }

  private static void push(StructClass owner, ArrayDeque<Visit> pending) {
    if (owner != null) pending.push(new Visit(owner, false));
  }

  private record Visit(StructClass owner, boolean expanded) { }
}
