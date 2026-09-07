// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.modules.decompiler.MethodExceptionSummary.ExceptionFlow;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.util.InterpreterUtil;
import org.jetbrains.java.decompiler.util.StronglyConnectedComponents;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.Comparator;
import java.util.Deque;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Computes and publishes source-level checked-exception summaries for own methods. */
public final class CheckedExceptionAnalyzer {
  public enum SummaryPurpose {
    INITIALIZER_EXTRACTION,
    CATCH_REPAIR,
    FINAL
  }

  private final CheckedInvocationResolver invocationResolver = new CheckedInvocationResolver();

  public synchronized void analyzeClasses(
    Collection<ClassesProcessor.ClassNode> classNodes,
    SummaryPurpose purpose
  ) {
    List<MethodInput> methods = collectMethods(classNodes);
    Map<MethodWrapper, MethodInput> inputs = new IdentityHashMap<>();
    for (MethodInput input : methods) {
      inputs.put(input.methodWrapper(), input);
      input.methodWrapper().setExceptionSummary(MethodExceptionSummary.EMPTY);
    }

    MethodRelations relations = scanRelations(methods, inputs);
    solveInferredDeclarations(methods, inputs, relations);

    // Re-run each local transfer once with stable declarations and retain only
    // the identity-mapped details consumed by the next lifecycle phase.
    for (MethodInput input : methods) {
      input.methodWrapper().setExceptionSummary(computeFinalSummary(input, relations, purpose));
    }
  }

  private List<MethodInput> collectMethods(Collection<ClassesProcessor.ClassNode> classNodes) {
    List<MethodInput> methods = new ArrayList<>();
    Set<ClassWrapper> seenWrappers = Collections.newSetFromMap(new IdentityHashMap<>());
    for (ClassesProcessor.ClassNode node : classNodes) {
      ClassWrapper wrapper = node.getWrapper();
      if (wrapper == null || !seenWrappers.add(wrapper)) {
        continue;
      }
      for (MethodWrapper methodWrapper : wrapper.getMethods()) {
        if (methodWrapper.root != null) {
          methods.add(new MethodInput(
            wrapper.getClassStruct(),
            wrapper,
            methodWrapper.methodStruct,
            methodWrapper
          ));
        }
      }
    }
    methods.sort(Comparator.comparing(MethodInput::sortKey));
    return methods;
  }

  private MethodRelations scanRelations(
    List<MethodInput> methods,
    Map<MethodWrapper, MethodInput> inputs
  ) {
    Map<MethodWrapper, Set<MethodWrapper>> dependencies = new IdentityHashMap<>();
    Map<MethodWrapper, Set<MethodWrapper>> reverseDependencies = new IdentityHashMap<>();
    Map<MethodWrapper, LinkedHashSet<String>> callsiteCaughtTypes = new IdentityHashMap<>();
    Map<MethodWrapper, SourceMethodSemantics.OverrideHierarchy> overrideHierarchies = new IdentityHashMap<>();

    for (MethodInput input : methods) {
      MethodWrapper caller = input.methodWrapper();
      dependencies.put(caller, identitySet());
      reverseDependencies.put(caller, identitySet());
      overrideHierarchies.put(
        caller,
        SourceMethodSemantics.findOverrideHierarchy(
          DecompilerContext.getStructContext(), input.ownerClass(), input.method()
        )
      );
    }

    for (MethodInput input : methods) {
      MethodWrapper caller = input.methodWrapper();
      CheckedStatementWalker.walk(
        caller.root.getFirst(),
        List.of(),
        CheckedStatementWalker.CatchAllPolicy.CATCH_ALL_IGNORED,
        (exprents, activeCatchTypes) -> {
          if (exprents == null) {
            return false;
          }
          for (Exprent exprent : InterpreterUtil.snapshotNonNullList(exprents, "checked-exception relation exprents")) {
            for (Exprent nested : exprent.getAllExprents(true, true)) {
              if (nested instanceof InvocationExprent invocation) {
                recordInvocation(input, caller, invocation, activeCatchTypes, inputs, dependencies, callsiteCaughtTypes);
              }
              else if (nested instanceof NewExprent created && created.getConstructor() != null) {
                recordInvocation(input, caller, created.getConstructor(), activeCatchTypes, inputs, dependencies, callsiteCaughtTypes);
              }
            }
          }
          return false;
        }
      );

      for (SourceMethodSemantics.InheritedMethod inherited : overrideHierarchies.get(caller).methods()) {
        ClassWrapper inheritedWrapper = invocationResolver.resolveClassWrapper(
          inherited.ownerClass(), input.ownerClass(), input.ownerWrapper()
        );
        MethodWrapper dependency = inheritedWrapper == null
          ? null
          : inheritedWrapper.getMethodWrapper(
            inherited.method().getName(), inherited.method().getDescriptor()
          );
        if (dependency != null && inputs.containsKey(dependency)) {
          dependencies.get(caller).add(dependency);
        }
      }
    }

    for (Map.Entry<MethodWrapper, Set<MethodWrapper>> entry : dependencies.entrySet()) {
      for (MethodWrapper dependency : entry.getValue()) {
        reverseDependencies.get(dependency).add(entry.getKey());
      }
    }
    return new MethodRelations(
      dependencies,
      reverseDependencies,
      callsiteCaughtTypes,
      overrideHierarchies
    );
  }

  private void recordInvocation(
    MethodInput callerInput,
    MethodWrapper caller,
    InvocationExprent invocation,
    List<String> activeCatchTypes,
    Map<MethodWrapper, MethodInput> inputs,
    Map<MethodWrapper, Set<MethodWrapper>> dependencies,
    Map<MethodWrapper, LinkedHashSet<String>> callsiteCaughtTypes
  ) {
    CheckedInvocationResolver.OwnMethodResolution own = invocationResolver.resolveOwnMethod(
      invocation, callerInput.ownerClass(), callerInput.ownerWrapper()
    );
    if (own == null || !inputs.containsKey(own.methodWrapper())) {
      return;
    }
    dependencies.get(caller).add(own.methodWrapper());
    if (own.classWrapper() == callerInput.ownerWrapper() && !activeCatchTypes.isEmpty()) {
      callsiteCaughtTypes
        .computeIfAbsent(own.methodWrapper(), ignored -> new LinkedHashSet<>())
        .addAll(activeCatchTypes);
    }
  }

  private void solveInferredDeclarations(
    List<MethodInput> methods,
    Map<MethodWrapper, MethodInput> inputs,
    MethodRelations relations
  ) {
    // Edges point from callers to their dependencies. Tarjan emits a dependency
    // component first, so no second graph or topological sort is necessary.
    for (List<MethodWrapper> component : StronglyConnectedComponents.find(
      methods.stream().map(MethodInput::methodWrapper).toList(), relations.dependencies()::get)) {
      component.sort(Comparator.comparing(MethodWrapper::toString));
      solveComponent(component, inputs, relations);
    }
  }

  private void solveComponent(
    List<MethodWrapper> component,
    Map<MethodWrapper, MethodInput> inputs,
    MethodRelations relations
  ) {
    Set<MethodWrapper> componentSet = identitySet();
    componentSet.addAll(component);
    Deque<MethodWrapper> worklist = new ArrayDeque<>(component);
    Set<MethodWrapper> queued = identitySet();
    queued.addAll(component);

    while (!worklist.isEmpty()) {
      MethodWrapper method = worklist.removeFirst();
      queued.remove(method);
      List<String> previous = method.getExceptionSummary().inferredThrows();
      List<String> next = computeInferredThrows(inputs.get(method), relations);
      Set<String> nextTypes = new HashSet<>(next);
      if (!nextTypes.containsAll(previous)) {
        throw new IllegalStateException("Checked-exception inference is not monotone for " + method);
      }
      if (nextTypes.equals(new HashSet<>(previous))) {
        continue;
      }
      method.setExceptionSummary(new MethodExceptionSummary(
        ExceptionFlow.EMPTY,
        next,
        List.of(),
        Map.of(),
        Map.of(),
        Map.of()
      ));
      for (MethodWrapper dependent : relations.reverseDependencies().get(method)) {
        if (componentSet.contains(dependent) && queued.add(dependent)) {
          worklist.addLast(dependent);
        }
      }
    }
  }

  private MethodExceptionSummary computeFinalSummary(
    MethodInput input,
    MethodRelations relations,
    SummaryPurpose purpose
  ) {
    CheckedExceptionFlowAnalyzer.MethodFlow flow = analyzeFlow(input, relations, purpose);
    List<String> inferred = computeInferredThrows(input, relations, flow);
    LinkedHashSet<String> escaping = missingEscapingExceptions(input, relations, flow);
    escaping.removeAll(inferred);

    List<String> wrapped = new ArrayList<>();
    for (String exceptionType : escaping) {
      // Unknown calls provide inference evidence, but do not make a specific
      // synthetic catch legal unless source-visible flow can throw that type.
      if (flow.methodFlow().hasVisibleThrow(exceptionType)) {
        wrapped.add(exceptionType);
      }
    }
    return new MethodExceptionSummary(
      flow.methodFlow(),
      inferred,
      CheckedExceptionSupport.sortMostSpecificFirst(
        CheckedExceptionSupport.removeRedundantSubtypes(wrapped)
      ),
      flow.statementFlows(),
      flow.protectedFlows(),
      flow.exprentFlows()
    );
  }

  private List<String> computeInferredThrows(MethodInput input, MethodRelations relations) {
    return computeInferredThrows(input, relations, analyzeFlow(input, relations, SummaryPurpose.FINAL));
  }

  private List<String> computeInferredThrows(
    MethodInput input,
    MethodRelations relations,
    CheckedExceptionFlowAnalyzer.MethodFlow flow
  ) {
    if (CodeConstants.CLINIT_NAME.equals(input.method().getName())) {
      return List.of();
    }
    List<String> escaping = new ArrayList<>(missingEscapingExceptions(input, relations, flow));
    return List.copyOf(filterByOverrideCompatibility(input, relations, escaping));
  }

  private LinkedHashSet<String> missingEscapingExceptions(
    MethodInput input,
    MethodRelations relations,
    CheckedExceptionFlowAnalyzer.MethodFlow flow
  ) {
    LinkedHashSet<String> escaping = new LinkedHashSet<>(flow.methodFlow().checkedExceptions());
    Set<String> callsiteTypes = relations.callsiteCaughtTypes().getOrDefault(input.methodWrapper(), new LinkedHashSet<>());
    if (input.method().hasModifier(CodeConstants.ACC_PRIVATE)
      && flow.methodFlow().unknownThrowability()) {
      for (String catchType : callsiteTypes) {
        if (isUsefulCallsiteType(catchType)) {
          escaping.add(catchType);
        }
      }
    }

    if (!CodeConstants.INIT_NAME.equals(input.method().getName())
      && !input.method().hasModifier(CodeConstants.ACC_PRIVATE)
      && !input.method().hasModifier(CodeConstants.ACC_STATIC)) {
      for (List<String> declaration : inheritedThrows(input, relations)) {
        for (String catchType : callsiteTypes) {
          if (!isUsefulCallsiteType(catchType)) {
            continue;
          }
          for (String declared : declaration) {
            if (CheckedExceptionSupport.isSubtypeOf(declared, catchType)) {
              escaping.add(declared);
            }
          }
        }
      }
    }

    List<String> existing = CheckedInvocationResolver.getDeclaredCheckedExceptions(
      input.ownerClass(), input.method()
    );
    escaping.removeIf(exceptionType -> CheckedExceptionSupport.isCoveredBy(exceptionType, existing));
    return escaping;
  }

  private CheckedExceptionFlowAnalyzer.MethodFlow analyzeFlow(
    MethodInput input,
    MethodRelations relations,
    SummaryPurpose purpose
  ) {
    return new CheckedExceptionFlowAnalyzer(
      invocationResolver,
      input.ownerClass(),
      input.ownerWrapper(),
      !DecompilerContext.shouldUseLegacySourceCompatibility(input.ownerClass(), BytecodeVersion.MAJOR_7),
      input.methodWrapper().varproc,
      purpose == SummaryPurpose.INITIALIZER_EXTRACTION,
      purpose == SummaryPurpose.CATCH_REPAIR,
      purpose == SummaryPurpose.INITIALIZER_EXTRACTION
    ).analyze(input.methodWrapper().root.getFirst());
  }

  private List<String> filterByOverrideCompatibility(
    MethodInput input,
    MethodRelations relations,
    List<String> inferred
  ) {
    if (inferred.isEmpty()
      || CodeConstants.INIT_NAME.equals(input.method().getName())
      || input.method().hasModifier(CodeConstants.ACC_PRIVATE)
      || input.method().hasModifier(CodeConstants.ACC_STATIC)) {
      return inferred;
    }
    List<List<String>> inherited = inheritedThrows(input, relations);
    if (inherited.isEmpty()) {
      return inferred;
    }

    List<String> compatible = new ArrayList<>();
    outer:
    for (String exceptionType : inferred) {
      for (List<String> declaration : inherited) {
        if (!CheckedExceptionSupport.isCoveredBy(exceptionType, declaration)) {
          continue outer;
        }
      }
      compatible.add(exceptionType);
    }
    return compatible;
  }

  private List<List<String>> inheritedThrows(MethodInput input, MethodRelations relations) {
    SourceMethodSemantics.OverrideHierarchy hierarchy =
      relations.overrideHierarchies().get(input.methodWrapper());
    List<List<String>> declarations = new ArrayList<>();
    for (SourceMethodSemantics.InheritedMethod inherited : hierarchy.methods()) {
      List<String> declared = CheckedInvocationResolver.getDeclaredCheckedExceptions(
        inherited.ownerClass(), inherited.method()
      );
      if (inherited.ownerClass().isOwn() && inherited.method().containsCode()) {
        ClassWrapper wrapper = invocationResolver.resolveClassWrapper(
          inherited.ownerClass(), input.ownerClass(), input.ownerWrapper()
        );
        MethodWrapper method = wrapper == null
          ? null
          : wrapper.getMethodWrapper(inherited.method().getName(), inherited.method().getDescriptor());
        if (method != null) {
          LinkedHashSet<String> combined = new LinkedHashSet<>(declared);
          combined.addAll(method.getExceptionSummary().inferredThrows());
          declared = List.copyOf(combined);
        }
      }
      declarations.add(declared);
    }

    String sourceName = SourceMethodSemantics.sourceName(input.ownerClass(), input.method());
    String sourceDescriptor = SourceMethodSemantics.sourceDescriptor(input.ownerClass(), input.method());
    for (String unresolved : hierarchy.unresolvedAncestors()) {
      if (!unresolved.startsWith("java/")) {
        declarations.add(List.of());
        continue;
      }
      TargetRuntimeResolver.MethodThrows platform =
        TargetRuntimeResolver.resolveMissingPlatformMethod(unresolved, sourceName, sourceDescriptor);
      if (platform.status() == TargetRuntimeResolver.Status.RESOLVED) {
        declarations.add(platform.checkedExceptions());
      }
      else if (platform.status() == TargetRuntimeResolver.Status.UNKNOWN) {
        declarations.add(List.of());
      }
    }
    return declarations;
  }

  private static boolean isUsefulCallsiteType(String catchType) {
    return CheckedExceptionSupport.isDeclaredCheckedExceptionType(catchType)
      && !"java/lang/Exception".equals(catchType)
      && !"java/lang/Throwable".equals(catchType);
  }

  private static <T> Set<T> identitySet() {
    return Collections.newSetFromMap(new IdentityHashMap<>());
  }

  private record MethodInput(
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    StructMethod method,
    MethodWrapper methodWrapper
  ) {
    String sortKey() {
      return ownerClass.qualifiedName + ' ' + method.getName() + ' ' + method.getDescriptor();
    }
  }

  private record MethodRelations(
    Map<MethodWrapper, Set<MethodWrapper>> dependencies,
    Map<MethodWrapper, Set<MethodWrapper>> reverseDependencies,
    Map<MethodWrapper, LinkedHashSet<String>> callsiteCaughtTypes,
    Map<MethodWrapper, SourceMethodSemantics.OverrideHierarchy> overrideHierarchies
  ) { }

}
