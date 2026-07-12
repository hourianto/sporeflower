// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.CounterContainer;
import org.jetbrains.java.decompiler.main.collectors.VarNamesCollector;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ConstExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.IfStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;

import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/** Applies source-compatibility repairs to the structured tree before rendering. */
public final class CheckedExceptionRepairProcessor {
  private CheckedExceptionRepairProcessor() { }

  public static void repairClasses(Collection<ClassesProcessor.ClassNode> classNodes) {
    Set<ClassWrapper> seen = Collections.newSetFromMap(new IdentityHashMap<>());
    for (ClassesProcessor.ClassNode node : classNodes) {
      ClassWrapper wrapper = node.getWrapper();
      if (wrapper == null || !seen.add(wrapper)) {
        continue;
      }
      for (MethodWrapper method : wrapper.getMethods()) {
        if (method.root == null) {
          continue;
        }
        DecompilerContext.resetMethod(method);
        repairCatchStatements(method.root.getFirst(), method.getExceptionSummary());
        if (DecompilerContext.getOption(IFernflowerPreferences.WRAP_UNDECLARED_CHECKED_EXCEPTIONS)
          && !method.getExceptionSummary().wrappedExceptions().isEmpty()) {
          wrapMethodBody(method, method.getExceptionSummary().wrappedExceptions());
        }
      }
    }
  }

  private static void repairCatchStatements(Statement statement, MethodExceptionSummary summary) {
    for (Statement child : new ArrayList<>(statement.getStats())) {
      repairCatchStatements(child, summary);
    }
    if (!(statement instanceof CatchStatement catchStatement)) {
      return;
    }

    MethodExceptionSummary.ExceptionFlow protectedFlow = summary.protectedFlowOf(catchStatement);
    List<String> previousTypes = new ArrayList<>();
    LinkedHashSet<String> markerTypes = new LinkedHashSet<>();
    int handler = 0;
    while (handler < catchStatement.getExctStrings().size()) {
      List<String> retained = new ArrayList<>();
      List<String> alternatives = CheckedExceptionSupport.removeRedundantSubtypes(
        new ArrayList<>(new LinkedHashSet<>(catchStatement.getExctStrings().get(handler)))
      );
      for (String exceptionType : alternatives) {
        if (!CheckedExceptionSupport.isShadowedBy(exceptionType, previousTypes)) {
          retained.add(exceptionType);
        }
      }
      if (retained.isEmpty()) {
        catchStatement.removeHandler(handler);
        continue;
      }

      catchStatement.replaceHandlerTypes(handler, retained);
      previousTypes.addAll(retained);
      for (String exceptionType : retained) {
        if (CheckedExceptionSupport.needsDeclaredCheckedThrowForCatchReachability(exceptionType)
          && !protectedFlow.hasVisibleThrow(exceptionType)) {
          markerTypes.add(exceptionType);
        }
      }
      handler++;
    }

    if (!markerTypes.isEmpty()) {
      List<Statement> markers = new ArrayList<>(markerTypes.size());
      for (String markerType : markerTypes) {
        BasicBlockStatement throwBlock = BasicBlockStatement.create();
        throwBlock.setExprents(List.of(new ExitExprent(
          ExitExprent.Type.THROW,
          new FunctionExprent(
            FunctionExprent.FunctionType.CAST,
            List.of(
              new ConstExprent(VarType.VARTYPE_NULL, null, null),
              new ConstExprent(new VarType(markerType, true), null, null)
            ),
            null
          ),
          null,
          null,
          null
        )));
        markers.add(IfStatement.createSourceOnly(
          new ConstExprent(VarType.VARTYPE_BOOLEAN, 0, null),
          throwBlock
        ));
      }
      catchStatement.prependReachabilityMarkers(markers);
    }
  }

  private static void wrapMethodBody(MethodWrapper method, List<String> exceptionTypes) {
    Statement body = method.root.getFirst();
    CatchStatement wrapper = createRuntimeWrapper(body, method, exceptionTypes);
    method.root.replaceStatement(body, wrapper);
  }

  static CatchStatement createRuntimeWrapper(
    Statement protectedBody,
    MethodWrapper owner,
    List<String> exceptionTypes
  ) {
    DecompilerContext.resetMethod(owner);
    List<String> orderedTypes = CheckedExceptionSupport.sortMostSpecificFirst(exceptionTypes);
    List<Statement> handlers = new ArrayList<>(orderedTypes.size());
    List<List<String>> handlerTypes = new ArrayList<>(orderedTypes.size());
    List<VarExprent> catchVariables = new ArrayList<>(orderedTypes.size());
    boolean supportsCause = TargetRuntimeResolver.supportsRuntimeExceptionCause();
    VarNamesCollector names = owner.varproc.getVarNamesCollector();
    // Mappings and late variable processing can assign names without updating the collector.
    names.addNames(owner.varproc.getVarNames());
    names.addNames(owner.varproc.clashingNames());

    for (int i = 0; i < orderedTypes.size(); i++) {
      String exceptionType = orderedTypes.get(i);
      int variableIndex = owner.counter.getCounterAndIncrement(CounterContainer.VAR_COUNTER);
      VarExprent catchVariable = new VarExprent(
        variableIndex,
        new VarType(exceptionType, true),
        owner.varproc
      );
      VarVersionPair pair = catchVariable.getVarVersionPair();
      owner.varproc.setVarName(pair, names.getFreeName(i == 0 ? "$VF$ex" : "$VF$ex" + i));

      BasicBlockStatement handler = BasicBlockStatement.create();
      handler.setExprents(List.of(new ExitExprent(
        ExitExprent.Type.THROW,
        newRuntimeException(catchVariable, supportsCause),
        null,
        null,
        null
      )));
      handlers.add(handler);
      handlerTypes.add(List.of(exceptionType));
      catchVariables.add(catchVariable);
    }
    return CatchStatement.createSourceOnly(protectedBody, handlers, handlerTypes, catchVariables);
  }

  private static NewExprent newRuntimeException(VarExprent caught, boolean supportsCause) {
    Exprent argument = supportsCause ? caught.copy() : toStringInvocation(caught);
    String descriptor = supportsCause
      ? "(Ljava/lang/Throwable;)V"
      : "(Ljava/lang/String;)V";
    InvocationExprent constructor = new InvocationExprent();
    constructor.setName(CodeConstants.INIT_NAME);
    constructor.setClassname("java/lang/RuntimeException");
    constructor.setStringDescriptor(descriptor);
    constructor.setDescriptor(MethodDescriptor.parseDescriptor(descriptor));
    constructor.setFunctype(InvocationExprent.Type.INIT);
    constructor.setLstParameters(List.of(argument));

    NewExprent created = new NewExprent(
      new VarType(CodeType.OBJECT, 0, "java/lang/RuntimeException"),
      List.of(),
      null
    );
    created.setConstructor(constructor);
    return created;
  }

  private static InvocationExprent toStringInvocation(VarExprent caught) {
    InvocationExprent invocation = new InvocationExprent();
    invocation.setName("toString");
    invocation.setClassname("java/lang/Throwable");
    invocation.setStringDescriptor("()Ljava/lang/String;");
    invocation.setDescriptor(MethodDescriptor.parseDescriptor("()Ljava/lang/String;"));
    invocation.setFunctype(InvocationExprent.Type.GENERAL);
    invocation.setInstance(caught.copy());
    invocation.setLstParameters(List.of());
    return invocation;
  }
}
