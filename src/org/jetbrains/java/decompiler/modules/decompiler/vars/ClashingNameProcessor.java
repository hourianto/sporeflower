// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.vars;

import org.jetbrains.annotations.NotNull;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.ValidationHelper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.*;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;

import java.util.*;
import java.util.Map.Entry;

/** Names locals in lexical scopes, carrying visible outer names into nested methods. */
final class ClashingNameProcessor {
  private final VarProcessor varproc;
  private final StructMethod mt;
  private final Map<VarVersionPair, String> clashingNames = new HashMap<>();
  private final Map<Statement, Set<VarInMethod>> varDefinitions = new HashMap<>();
  private final Map<VarInMethod, String> nameMap;
  private final Set<StructMethod> seenMethods;

  private ClashingNameProcessor(VarProcessor varproc, StructMethod method,
                               Map<VarInMethod, String> names, Set<StructMethod> seen) {
    this.varproc = varproc;
    this.mt = method;
    this.nameMap = names;
    this.seenMethods = seen;
  }

  static Map<VarVersionPair, String> remap(Statement root, StructMethod method, VarProcessor varproc) {
    Set<StructMethod> seen = Collections.newSetFromMap(new IdentityHashMap<>());
    seen.add(method);
    ClashingNameProcessor processor = new ClashingNameProcessor(varproc, method, new HashMap<>(), seen);
    processor.registerParameters(0, false);
    processor.iterateClashingNames(root);
    return processor.clashingNames;
  }

  private void registerParameters(int firstParameter, boolean renameConflicts) {
    MethodDescriptor descriptor = mt.methodDescriptor();
    int slot = mt.hasModifier(CodeConstants.ACC_STATIC) ? 0 : 1;
    for (int ordinal = 0; ordinal < descriptor.params.length; ordinal++) {
      if (ordinal >= firstParameter) {
        VarVersionPair pair = new VarVersionPair(slot, 0);
        String name = varproc.getVarName(pair);
        if (name != null && renameConflicts) {
          String renamed = rename(nameMap, name);
          if (!renamed.equals(name)) varproc.setClashingName(pair, renamed);
          name = renamed;
        }
        nameMap.put(new VarInMethod(pair, mt), name);
      }
      slot += descriptor.params[ordinal].stackSize;
    }
  }

  private void renameNestedMethod(MethodWrapper method, int firstParameter, boolean lambda) {
    if (method == null || method.root == null || !seenMethods.add(method.methodStruct)) return;
    // Nested bodies inherit visible names, but their declarations cannot escape
    // back into the outer body or a sibling anonymous method.
    Map<VarInMethod, String> visibleNames = new HashMap<>(nameMap);
    // Anonymous methods may shadow enclosing locals; only captured names must
    // remain accessible. Lambda parameters instead share the enclosing scope.
    if (!lambda) visibleNames.values().removeIf(name -> !method.setOuterVarNames.contains(name));
    ClashingNameProcessor nested = new ClashingNameProcessor(method.varproc, method.methodStruct, visibleNames, seenMethods);
    nested.registerParameters(firstParameter, true);
    nested.iterateClashingNames(method.root);
    nested.clashingNames.forEach(method.varproc::setClashingName);
  }

  private record VarInMethod(VarVersionPair pair, StructMethod mt) { }

  private void iterateClashingNames(Statement stat) {
    Set<VarInMethod> curVarDefs = new HashSet<>();

    boolean ownsScope = stat.getExprents() == null;
    Statement parentScope = stat.getParent() == null ? stat : stat.getParent();

    // Process var definitions as owned by the parent- they come before the statement, and so their scope extends past the actual statement.
    for (Exprent exprent : stat.getVarDefinitions()) {
      Set<VarInMethod> upDefs = new HashSet<>();
      iterateClashingExprent(stat, exprent, upDefs);
      varDefinitions.computeIfAbsent(parentScope, ignored -> new HashSet<>()).addAll(upDefs);
    }

    // Process head of if first. The head comes *before* the actual if() expression, and so it must be owned by the if's parent.
    if (stat instanceof IfStatement) {
      Set<VarInMethod> upDefs = new HashSet<>();
      BasicBlockStatement basic = stat.getBasichead();
      collectDefinitions(basic, basic.getExprents(), upDefs);

      varDefinitions.computeIfAbsent(parentScope, ignored -> new HashSet<>()).addAll(upDefs);
    }

    collectDefinitions(stat, ownsScope ? stat.getStatExprents() : stat.getExprents(), curVarDefs);

    varDefinitions.computeIfAbsent(stat, ignored -> new HashSet<>()).addAll(curVarDefs);

    boolean iterate = !(stat instanceof SwitchStatement switchStat && switchStat.isPhantom());
    List<Statement> deferred = new ArrayList<>();
    if (iterate) {
      for (Statement st : stat.getStats()) {
        if (stat instanceof IfStatement) {
          IfStatement ifstat = (IfStatement)stat;

          if (ifstat.getElsestat() == st) {
            // Defer else blocks of if statements, as they are independent from the context of the if block
            deferred.add(st);
            continue;
          }

          // We've already looked at the head- don't look again!
          if (st == stat.getBasichead()) {
            continue;
          }
        }

        iterateClashingNames(st);
      }
    }

    if (ownsScope) {
      clearStatement(stat);
    }

    for (Statement st : new HashSet<>(varDefinitions.keySet())) {
      if (st.getParent() == stat) {
        clearStatement(st);
      }
    }

    // Process deferred statements
    if (iterate) {
      for (Statement st : deferred) {
        iterateClashingNames(st);
      }
    }

    for (Statement st : new HashSet<>(varDefinitions.keySet())) {
      if (st.getParent() == stat && deferred.contains(st)) {
        clearStatement(st);
      }
    }
  }

  private void collectDefinitions(Statement owner, List<Exprent> expressions, Set<VarInMethod> definitions) {
    for (Exprent expression : expressions) {
      // Preserve expression order: a declaration on an assignment's left must
      // be visible while processing a lambda on its right.
      for (Exprent nested : expression.getAllExprents(true, true)) {
        iterateClashingExprent(owner, nested, definitions);
      }
    }
  }

  private void clearStatement(Statement statement) {
    nameMap.keySet().removeAll(varDefinitions.remove(statement));
  }

  private void iterateClashingExprent(Statement stat, Exprent exprent, Set<VarInMethod> curVarDefs) {
    if (exprent instanceof NewExprent created && !created.isMethodReference()) {
      ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(created.getNewType().value);
      if (node != null && node.getWrapper() != null) {
        if (created.isLambda()) {
          MethodDescriptor content = MethodDescriptor.parseDescriptor(node.lambdaInformation.content_method_descriptor);
          int captured = content.params.length - MethodDescriptor.parseDescriptor(node.lambdaInformation.method_descriptor).params.length;
          renameNestedMethod(node.getWrapper().getMethods().getWithKey(node.lambdaInformation.content_method_key), captured, true);
        } else if (created.isAnonymous()) {
          for (MethodWrapper method : node.getWrapper().getMethods()) {
            if (!method.methodStruct.hasModifier(CodeConstants.ACC_SYNTHETIC)) renameNestedMethod(method, 0, false);
          }
        }
      }
    }

    if (exprent instanceof VarExprent) {
      VarExprent var = (VarExprent) exprent;

      if (var.isDefinition()) {
        curVarDefs.add(new VarInMethod(var.getVarVersionPair(), mt));

        // Only process vars that have lvt as the default var<index>_<version> names can never conflict
        if (var.getLVT() != null || this.varproc.getVarName(var.getVarVersionPair()) != null) {
          String name = var.getLVT() == null ? this.varproc.getVarName(var.getVarVersionPair()) : var.getLVT().getName();

          String originalName = name;
          name = rename(nameMap, name);

          boolean scopedSwitch = false;
          if (!originalName.equals(name)) {
            // Try to scope switch statements if possible as it's a less destructive operation when considering local variable names
            Statement parent = directParent(stat);
            if (parent instanceof SwitchStatement) {
              Set<VarInMethod> sameVarName = new HashSet<>();

              // Find vars with the same name
              for (Entry<VarInMethod, String> entry : nameMap.entrySet()) {
                if (entry.getValue().equals(originalName)) {
                  sameVarName.add(entry.getKey());
                }
              }

              SwitchStatement switchStat = (SwitchStatement)parent;
              // Iterate through all cases
              for (Statement st : switchStat.getCaseStatements()) {
                Set<VarInMethod> caseVarDefs = varDefinitions.get(st);

                // Check if the case branch has var defs
                if (caseVarDefs != null) {
                  for (VarInMethod pair : sameVarName) {
                    // Try to find var defs
                    if (caseVarDefs.contains(pair)) {
                      switchStat.scopeCaseStatement(st);
                      // Try to find the case statement that the current statement belongs to
                      Statement foundCase = findCaseOwning(stat, switchStat);

                      // If found, scope the current statement
                      if (foundCase != null) {
                        switchStat.scopeCaseStatement(foundCase);
                      }

                      // scoped switch, don't remap
                      scopedSwitch = true;
                    }
                  }
                }
              }
            }

            if (!scopedSwitch) {
              // Remapped name
              this.clashingNames.put(var.getVarVersionPair(), name);
            }
          }

          // Record the changed name if we didn't scope switch
          String value = scopedSwitch ? originalName : name;
          if (value == null) {
            ValidationHelper.validateTrue(false, "Variable name is null");
          } else {
            nameMap.put(new VarInMethod(var.getVarVersionPair(), mt), value);
          }
        }
      }
    }
  }

  private static @NotNull String rename(Map<VarInMethod, String> nameMap, String name) {
    while (nameMap.containsValue(name)) {
      name += "x";
    }
    return name;
  }

  // Finds the case statement that the given statement belongs to
  private static Statement findCaseOwning(Statement stat, SwitchStatement switchStat) {
    for (Statement caseStatement : switchStat.getCaseStatements()) {
      if (caseStatement.containsStatement(stat)) {
        return caseStatement;
      }
    }

    return null;
  }

  // Finds the owner of a statement, skipping if statement first statements as they are placed above the actual if statement
  private static Statement directParent(Statement stat) {
    Statement parent = stat.getParent();

    while (parent != null && (parent instanceof SequenceStatement || (parent.getFirst() == stat && (parent instanceof IfStatement)))) {
      parent = parent.getParent();
    }

    return parent;
  }

}
