// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FieldExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;

/** The semantic pipeline runs while SSA-derived local identities are still intact. */
public final class SemanticConstantsProcessor {
  private SemanticConstantsProcessor() {}

  public static void process(Statement root, StructClass owner, StructMethod method, VarProcessor variables, SemanticMappings mappings) {
    if (!hasBindings(root, owner, method, mappings))
      return;
    SemanticAnalysis analysis = new SemanticAnalysis(root, owner, method, variables, mappings);
    analysis.graph.solveProducers();
    analysis.graph.solveRequirements(new SemanticUses(root, analysis, analysis.graph));
    SemanticRenderer renderer = new SemanticRenderer(analysis);
    new SemanticUses(root, analysis, renderer).visit();
    renderer.finish();
  }

  private static boolean hasBindings(Statement root, StructClass owner, StructMethod method, SemanticMappings mappings) {
    MemberKey current = new MemberKey(owner.qualifiedName, method.getName(), method.getDescriptor());
    if (methodBindings(current, mappings) || !mappings.callBindings(current).isEmpty())
      return true;
    boolean[] found = {false};
    for (var expression : roots(root)) {
      walk(expression, child -> {
        if (found[0])
          return;
        if (child instanceof FieldExprent field)
          found[0] = !mappings.contract(fieldKey(field), "field", -1).equals(SemanticContract.NONE);
        else if (child instanceof InvocationExprent invocation)
          found[0] = methodBindings(invocationKey(invocation), mappings);
      });
      if (found[0])
        return true;
    }
    // Without a declaration or a scoped call there is no semantic evidence to
    // seed. Avoid constructing a control-flow analysis for unrelated methods.
    return false;
  }

  private static boolean methodBindings(MemberKey method, SemanticMappings mappings) {
    if (!mappings.contract(method, "return", -1).equals(SemanticContract.NONE))
      return true;
    int count = MethodDescriptor.parseDescriptor(method.desc()).params.length;
    for (int index = 0; index < count; index++)
      if (!mappings.contract(method, "parameter", index).equals(SemanticContract.NONE))
        return true;
    return false;
  }
}
