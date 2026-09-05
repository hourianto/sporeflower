// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.decompiler.CancelationManager;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.decompiler.exps.*;
import org.jetbrains.java.decompiler.modules.decompiler.exps.FunctionExprent.FunctionType;
import org.jetbrains.java.decompiler.modules.decompiler.stats.BasicBlockStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchAllStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.CatchStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.IfStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.SequenceStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statements;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructField;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.InterpreterUtil;
import org.jetbrains.java.decompiler.util.StatementIterator;

import java.util.*;

public final class InitializerProcessor {
  private static final String PREINIT_HELPER_PREFIX = "$sporeflower$preinit$";

  public static void extractInitializers(ClassWrapper wrapper) {
    MethodWrapper method = wrapper.getMethodWrapper(CodeConstants.CLINIT_NAME, "()V");
    try {
      if (method != null && method.root != null) {  // successfully decompiled static constructor
        extractStaticInitializers(wrapper, method);
      }
    } catch (CancelationManager.CanceledException e) {
      throw e;
    } catch (Throwable t) {
      StructMethod mt = method.methodStruct;
      String message = "Method " + mt.getName() + " " + mt.getDescriptor() + " in class " + wrapper.getClassStruct().qualifiedName + " couldn't be written.";
      DecompilerContext.getLogger().writeMessage(message, IFernflowerLogger.Severity.WARN, t);

      method.decompileError = t;
    }

    extractDynamicInitializers(wrapper);
    normalizeConstructorSourceLegality(wrapper);
    normalizeConstructorDelegationArguments(wrapper);

    // required e.g. if anonymous class is being decompiled as a standard one.
    // This can happen if InnerClasses attributes are erased
    liftConstructor(wrapper);

    if (DecompilerContext.getOption(IFernflowerPreferences.HIDE_EMPTY_SUPER)) {
      hideEmptySuper(wrapper);
    }
  }

  private static void normalizeConstructorDelegationArguments(ClassWrapper wrapper) {
    if (!canEmitStaticSourceOnlyHelpers(wrapper)) {
      return;
    }

    for (MethodWrapper method : wrapper.getMethods()) {
      if (!CodeConstants.INIT_NAME.equals(method.methodStruct.getName()) || method.root == null) {
        continue;
      }

      boolean changed;
      do {
        changed = normalizeConstructorPrelude(wrapper, method, method.root);
      }
      while (changed);
    }
  }

  private static boolean canEmitStaticSourceOnlyHelpers(ClassWrapper wrapper) {
    StructClass cl = wrapper.getClassStruct();
    if (cl.hasModifier(CodeConstants.ACC_INTERFACE) || cl.hasModifier(CodeConstants.ACC_ANNOTATION)) {
      return false;
    }

    ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(cl.qualifiedName);
    return node != null &&
      (node.type == ClassNode.Type.ROOT ||
        (node.type == ClassNode.Type.MEMBER && (node.access & CodeConstants.ACC_STATIC) != 0));
  }

  /*
   * Constructor-prelude lifting is intentionally conservative. Helpers are only
   * emitted in classes where static methods are legal for the old source target,
   * for slices that do not touch uninitialized `this`, do not need synthetic
   * parameters, and preserve Java's left-to-right argument side-effect order.
   */
  private static boolean normalizeConstructorPrelude(ClassWrapper wrapper, MethodWrapper method, Statement stat) {
    if (stat instanceof IfStatement ifStatement) {
      ConstructorCall ifLocation = findConstructorCall(ifStatement.getIfstat(), method, wrapper, true);
      ConstructorCall elseLocation = findConstructorCall(ifStatement.getElsestat(), method, wrapper, true);
      if (ifLocation != null ^ elseLocation != null) {
        Statement otherBranch = ifLocation == null ? ifStatement.getIfstat() : ifStatement.getElsestat();
        if (terminatesWithExit(otherBranch, ExitExprent.Type.THROW)) {
          if (liftBranchingPrelude(wrapper, method, ifStatement, ifLocation != null ? ifLocation : elseLocation)) {
            return true;
          }
        }
      }
    }

    if (stat instanceof SequenceStatement sequence) {
      for (int i = 0; i < sequence.getStats().size(); i++) {
        Statement child = sequence.getStats().get(i);
        ConstructorCall location = findConstructorCall(child, method, wrapper, false);
        if (location != null && i > 0) {
          return liftStructuredPrelude(wrapper, method, sequence, i, location.invocation);
        }
      }
    }

    if (stat.getExprents() != null && normalizeConstructorPreludeExprents(wrapper, method, stat.getExprents())) {
      return true;
    }

    for (Statement child : stat.getStats()) {
      if (normalizeConstructorPrelude(wrapper, method, child)) {
        return true;
      }
    }

    return false;
  }

  private static boolean normalizeConstructorPreludeExprents(ClassWrapper wrapper, MethodWrapper method, List<Exprent> exprents) {
    ConstructorCall call = findConstructorCall(exprents, method, wrapper);
    if (call == null || call.index <= 0) {
      return false;
    }

    List<PreludeElement> prelude = preludeExprents(exprents.subList(0, call.index));
    if (!liftPreludeArguments(wrapper, method, call.invocation, prelude, true)) {
      return false;
    }
    removePrefix(exprents, call.index);
    return true;
  }

  private static boolean liftStructuredPrelude(
    ClassWrapper wrapper,
    MethodWrapper method,
    SequenceStatement sequence,
    int initStatementIndex,
    InvocationExprent invocation
  ) {
    List<Statement> prelude = new ArrayList<>(sequence.getStats().subList(0, initStatementIndex));
    List<PreludeElement> elements = preludeStatements(prelude);
    if (!liftPreludeArguments(wrapper, method, invocation, elements, false)) {
      return false;
    }

    removePrefix(sequence.getStats(), initStatementIndex);
    if (!sequence.getStats().isEmpty()) {
      sequence.setFirst(sequence.getStats().get(0));
    }

    return true;
  }

  private static boolean liftBranchingPrelude(
    ClassWrapper wrapper,
    MethodWrapper method,
    IfStatement ifStatement,
    ConstructorCall location
  ) {
    if (location.statement == null || location.index <= 0 || location.index != location.exprents.size() - 1) {
      return false;
    }

    List<PreludeElement> prelude = preludeExprents(location.exprents.subList(0, location.index));
    Set<VarVersionPair> assignedPreludeVars = varUseElements(prelude).assigned;
    if (assignedPreludeVars.isEmpty()) {
      return false;
    }

    int dependentParameter = findSingleDependentParameter(location.invocation, assignedPreludeVars);
    if (dependentParameter < 0) {
      return false;
    }

    List<PreludeElement> branchBody = preludeStatements(Collections.singletonList(ifStatement));
    if (!Collections.disjoint(varUse(ifStatement).assigned, variablesOutsidePrelude(method, branchBody, location.invocation))) {
      return false;
    }
    for (int i = 0; i < dependentParameter; i++) {
      if (!isLocalOrConstant(location.invocation.getLstParameters().get(i))) {
        return false;
      }
    }

    Exprent returnValue = location.invocation.getLstParameters().get(dependentParameter).copy();
    VarType returnType = getConstructorParameterType(location.invocation, dependentParameter);
    if (returnType == null) {
      returnType = returnValue.getExprType();
    }

    Statement parent = ifStatement.getParent();
    if (parent == null) {
      return false;
    }

    InvocationExprent constructorCall = (InvocationExprent)location.invocation.copy();
    Exprent previous = location.exprents.set(
      location.index,
      new ExitExprent(ExitExprent.Type.RETURN, returnValue, returnType, location.invocation.bytecode, null));

    InvocationExprent helperCall = createPreinitHelperCall(
      wrapper,
      method,
      returnType,
      new HelperBody(Collections.singletonList(ifStatement), Collections.emptyList()),
      null);
    if (helperCall == null) {
      location.exprents.set(location.index, previous);
      return false;
    }

    removeDirectSuccessors(location.statement);
    constructorCall.getLstParameters().set(dependentParameter, helperCall);
    BasicBlockStatement replacement = BasicBlockStatement.create();
    replacement.setExprents(Collections.singletonList(constructorCall));

    parent.replaceStatement(ifStatement, replacement);
    return true;
  }

  private static boolean terminatesWithExit(Statement stat, ExitExprent.Type exitType) {
    if (stat == null) {
      return false;
    }

    if (stat.getExprents() != null) {
      List<Exprent> exprents = stat.getExprents();
      return !exprents.isEmpty() &&
        exprents.get(exprents.size() - 1) instanceof ExitExprent exit &&
        exit.getExitType() == exitType;
    }

    if (stat instanceof SequenceStatement sequence) {
      List<Statement> stats = sequence.getStats();
      return !stats.isEmpty() && terminatesWithExit(stats.get(stats.size() - 1), exitType);
    }

    if (stat instanceof IfStatement ifStatement && ifStatement.iftype == IfStatement.IFTYPE_IFELSE) {
      return terminatesWithExit(ifStatement.getIfstat(), exitType) &&
        terminatesWithExit(ifStatement.getElsestat(), exitType);
    }

    return false;
  }

  private static void removeDirectSuccessors(Statement stat) {
    for (StatEdge edge : new ArrayList<>(stat.getAllDirectSuccessorEdges())) {
      edge.remove();
    }
  }

  private static boolean preservesPreludeOrder(List<PreludeElement> prelude, List<ParameterSlice> slices) {
    int lastOrderedSideEffect = -1;

    // Completed slices cover the entire prelude. In left-to-right argument
    // order, strictly increasing effect indexes rule out both duplication and
    // reordering. Local copies and declarations may be repeated across helpers.
    for (ParameterSlice slice : slices) {
      for (int index : slice.indexes) {
        if (!prelude.get(index).isRepeatable()) {
          if (index <= lastOrderedSideEffect) {
            return false;
          }
          lastOrderedSideEffect = index;
        }
      }
    }

    return true;
  }

  private static ConstructorCall findConstructorCall(List<Exprent> exprents, MethodWrapper method, ClassWrapper wrapper) {
    for (int i = 0; i < exprents.size(); i++) {
      if (exprents.get(i) instanceof InvocationExprent invocation
        && Statements.isInvocationInitConstructor(invocation, method, wrapper, true)) {
          return new ConstructorCall(exprents, i, invocation, null);
      }
    }

    return null;
  }

  private static void normalizeConstructorSourceLegality(ClassWrapper wrapper) {
    for (MethodWrapper method : wrapper.getMethods()) {
      if (!CodeConstants.INIT_NAME.equals(method.methodStruct.getName()) || method.root == null) {
        continue;
      }

      relocatePreConstructorCopies(wrapper, method);
      normalizeFinalFieldReceiverWrites(wrapper, method);
    }
  }

  private static void relocatePreConstructorCopies(ClassWrapper wrapper, MethodWrapper method) {
    ConstructorCall call = findConstructorCall(method.root, method, wrapper, true);
    if (call == null || call.exprents == null || call.index <= 0) {
      return;
    }

    // A catch handler could observe the old local values when initialization
    // throws. Within an unprotected basic block, local/constant copies cannot
    // throw or affect the constructor, provided their data dependencies commute.
    for (Statement parent = call.statement; parent != null; parent = parent.getParent()) {
      if (parent instanceof CatchStatement || parent instanceof CatchAllStatement) {
        return;
      }
    }

    if (call.invocation.getInstance() instanceof VarExprent instance &&
        method.varproc.isReceiverEquivalent(new VarVersionPair(instance))) {
      call.invocation.replaceExprent(instance, createCurrentReceiver(wrapper, method, instance));
    }

    List<Integer> copyIndexes = new ArrayList<>();
    List<Exprent> copies = new ArrayList<>();
    VarUse crossed = varUse(call.invocation);
    for (int i = call.index - 1; i >= 0; i--) {
      Exprent exprent = call.exprents.get(i);
      VarUse use = varUse(exprent);
      Set<VarVersionPair> writes = new HashSet<>(use.assigned);
      writes.addAll(use.definitions);
      if (isLocalCopy(exprent) && !toVarIndexes(writes).contains(0) &&
          Collections.disjoint(writes, crossed.all) &&
          Collections.disjoint(use.reads, crossed.assigned)) {
        copyIndexes.add(i);
        copies.add(exprent);
      } else {
        crossed.add(use);
      }
    }

    // Keep the copies in their original order, including parameter-slot swaps.
    for (int index : copyIndexes) {
      call.exprents.remove(index);
    }
    Collections.reverse(copies);
    int constructorIndex = call.exprents.indexOf(call.invocation);
    call.exprents.addAll(constructorIndex + 1, copies);
  }

  private static boolean isLocalCopy(Exprent exprent) {
    if (exprent instanceof VarExprent var) {
      return var.isDefinition();
    }
    return exprent instanceof AssignmentExprent assignment && assignment.getCondType() == null &&
      assignment.getLeft() instanceof VarExprent && isLocalOrConstant(assignment.getRight());
  }

  private static boolean isLocalOrConstant(Exprent exprent) {
    if (exprent instanceof VarExprent) {
      return true;
    }
    // Resolving a class literal can throw; it is not an inert local copy.
    return exprent instanceof ConstExprent constant &&
      (constant.getConstType().type != CodeType.OBJECT || VarType.VARTYPE_STRING.equals(constant.getConstType()));
  }

  private static void normalizeFinalFieldReceiverWrites(ClassWrapper wrapper, MethodWrapper method) {
    StructClass cl = wrapper.getClassStruct();
    StatementIterator.iterate(method.root, exprent -> {
      if (exprent instanceof AssignmentExprent assignment &&
          assignment.getLeft() instanceof FieldExprent field &&
          !field.isStatic() &&
          cl.qualifiedName.equals(field.getClassname()) &&
          field.getInstance() instanceof VarExprent instance &&
          method.varproc.isReceiverEquivalent(new VarVersionPair(instance))) {
        StructField structField = cl.getField(field.getName(), field.getDescriptor().descriptorString);
        if (structField != null && structField.hasModifier(CodeConstants.ACC_FINAL) &&
            !method.varproc.getThisVars().containsKey(new VarVersionPair(instance))) {
          field.replaceExprent(instance, createCurrentReceiver(wrapper, method, instance));
        }
      }
      return 0;
    });
  }

  private static VarExprent createCurrentReceiver(ClassWrapper wrapper, MethodWrapper method, Exprent source) {
    return new VarExprent(
      0,
      new VarType(CodeType.OBJECT, 0, wrapper.getClassStruct().qualifiedName),
      method.varproc,
      source.bytecode
    );
  }

  private static ConstructorCall findConstructorCall(Statement stat, MethodWrapper method, ClassWrapper wrapper, boolean mutableOnly) {
    if (stat == null) {
      return null;
    }

    if (stat.getExprents() != null) {
      ConstructorCall call = findConstructorCall(stat.getExprents(), method, wrapper);
      if (call != null) {
        return new ConstructorCall(call.exprents, call.index, call.invocation, stat);
      }
    }

    if (!mutableOnly) {
      for (Exprent exprent : stat.getStatExprents()) {
        if (exprent instanceof InvocationExprent invocation
          && Statements.isInvocationInitConstructor(invocation, method, wrapper, true)) {
          return new ConstructorCall(null, -1, invocation, null);
        }
      }
    }

    for (Statement child : stat.getStats()) {
      ConstructorCall call = findConstructorCall(child, method, wrapper, mutableOnly);
      if (call != null) {
        return call;
      }
    }

    return null;
  }

  private static List<PreludeElement> preludeExprents(List<? extends Exprent> exprents) {
    List<PreludeElement> result = new ArrayList<>(exprents.size());
    for (Exprent exprent : exprents) {
      result.add(new PreludeElement(null, exprent));
    }
    return result;
  }

  private static List<PreludeElement> preludeStatements(List<Statement> statements) {
    List<PreludeElement> result = new ArrayList<>(statements.size());
    for (Statement statement : statements) {
      result.add(new PreludeElement(statement, null));
    }
    return result;
  }

  private static int findSingleDependentParameter(InvocationExprent invocation, Set<VarVersionPair> assignedPreludeVars) {
    int result = -1;
    for (int i = 0; i < invocation.getLstParameters().size(); i++) {
      if (!Collections.disjoint(varUse(invocation.getLstParameters().get(i)).all, assignedPreludeVars)) {
        if (result >= 0) {
          return -2;
        }
        result = i;
      }
    }
    return result;
  }

  // Either extract the whole prelude or leave the constructor and helper list unchanged.
  private static boolean liftPreludeArguments(
    ClassWrapper wrapper,
    MethodWrapper method,
    InvocationExprent invocation,
    List<PreludeElement> prelude,
    boolean allowMultiple
  ) {
    Set<VarVersionPair> assignedPreludeVars = varUseElements(prelude).assigned;
    if (assignedPreludeVars.isEmpty()) {
      return false;
    }

    List<ParameterSlice> slices = new ArrayList<>();
    List<Exprent> parameters = invocation.getLstParameters();
    boolean sawDependentParameter = false;
    for (int i = 0; i < parameters.size(); i++) {
      Exprent parameter = parameters.get(i);
      if (Collections.disjoint(varUse(parameter).all, assignedPreludeVars)) {
        continue;
      }

      if (!allowMultiple && sawDependentParameter) {
        return false;
      }
      sawDependentParameter = true;

      List<Integer> slice = dependencySlice(prelude, parameter);
      if (!slice.isEmpty()) {
        slices.add(new ParameterSlice(i, slice));
      }
    }

    if (slices.isEmpty()) {
      return false;
    }

    completePreludeSlices(prelude, parameters, slices);
    if (!preservesPreludeOrder(prelude, slices)) {
      return false;
    }

    Set<VarVersionPair> outside = variablesOutsidePrelude(method, prelude, invocation);
    if (!Collections.disjoint(assignedPreludeVars, outside)) {
      return false; // Helpers cannot carry changed locals back into the constructor body.
    }
    int lastParameter = slices.get(slices.size() - 1).parameter;
    Set<Integer> liftedParameters = new HashSet<>();
    for (ParameterSlice slice : slices) {
      liftedParameters.add(slice.parameter);
    }
    for (int i = 0; i < parameters.size(); i++) {
      Exprent parameter = parameters.get(i);
      // All argument expressions originally ran after the prelude. A read of a
      // field, a cast or a call cannot run ahead of a later helper's work, even
      // when its result has no local-variable dependency on that work.
      if ((i < lastParameter && !isLocalOrConstant(parameter)) ||
          (!liftedParameters.contains(i) && !Collections.disjoint(varUse(parameter).all, assignedPreludeVars))) {
        return false;
      }
    }

    List<PreparedHelper> helpers = new ArrayList<>();
    for (ParameterSlice slice : slices) {
      Exprent parameter = parameters.get(slice.parameter);
      VarType returnType = getConstructorParameterType(invocation, slice.parameter);
      if (returnType == null) {
        returnType = parameter.getExprType();
      }
      PreparedHelper helper = preparePreinitHelper(method, returnType, bodyFromSlice(prelude, slice.indexes), parameter);
      if (helper == null) {
        return false;
      }
      helpers.add(helper);
    }

    for (int i = 0; i < slices.size(); i++) {
      ParameterSlice slice = slices.get(i);
      parameters.set(slice.parameter, createPreinitHelperCall(wrapper, method, helpers.get(i)));
    }
    return true;
  }

  private static void completePreludeSlices(List<PreludeElement> prelude, List<Exprent> parameters, List<ParameterSlice> slices) {
    // Value slicing alone misses work such as advancing a cursor after decoding
    // an argument. Assign every remaining element to the next argument region
    // (or the final region for trailing work), then close its data dependencies.
    // The completed slices must pass the ordering and scope checks before any
    // part of the constructor is changed.
    Set<Integer> covered = new HashSet<>();
    for (ParameterSlice slice : slices) {
      covered.addAll(slice.indexes);
    }
    for (int i = 0; i < prelude.size(); i++) {
      if (!covered.contains(i)) {
        ParameterSlice owner = slices.get(slices.size() - 1);
        for (ParameterSlice slice : slices) {
          if (i <= Collections.max(slice.indexes)) {
            owner = slice;
            break;
          }
        }
        owner.indexes.add(i);
      }
    }
    for (int i = 0; i < slices.size(); i++) {
      ParameterSlice slice = slices.get(i);
      slices.set(i, new ParameterSlice(slice.parameter,
        dependencySlice(prelude, parameters.get(slice.parameter), new HashSet<>(slice.indexes))));
    }
  }

  private static Set<VarVersionPair> variablesOutsidePrelude(
    MethodWrapper method, List<PreludeElement> prelude, InvocationExprent invocation
  ) {
    Set<Exprent> moved = Collections.newSetFromMap(new IdentityHashMap<>());
    moved.addAll(invocation.getAllExprents(true, true));
    for (PreludeElement element : prelude) {
      if (element.statement != null) {
        StatementIterator.iterate(element.statement, exprent -> {
          moved.add(exprent);
          return 0;
        });
      } else {
        moved.addAll(element.exprent.getAllExprents(true, true));
      }
    }
    Set<VarVersionPair> outside = new HashSet<>();
    StatementIterator.iterate(method.root, exprent -> {
      if (exprent instanceof VarExprent var && !moved.contains(var)) {
        outside.add(new VarVersionPair(var));
      }
      return 0;
    });
    return outside;
  }

  private static List<Integer> dependencySlice(List<PreludeElement> prelude, Exprent returnValue) {
    return dependencySlice(prelude, returnValue, Collections.emptySet());
  }

  private static List<Integer> dependencySlice(List<PreludeElement> prelude, Exprent returnValue, Set<Integer> required) {
    VarUse returnUse = varUse(returnValue);
    Set<VarVersionPair> dependencies = new HashSet<>(returnUse.reads);
    Set<VarVersionPair> neededDefinitions = new HashSet<>(returnUse.lefts);

    LinkedHashSet<Integer> selected = new LinkedHashSet<>();
    for (int i = prelude.size() - 1; i >= 0; i--) {
      VarUse use = prelude.get(i).varUse();
      Set<VarVersionPair> assigned = use.assigned;
      Set<VarVersionPair> reads = use.reads;

      boolean valueDependency = !Collections.disjoint(assigned, dependencies);
      boolean sideEffectDependency = !Collections.disjoint(reads, dependencies) && assigned.isEmpty();
      boolean definitionDependency = !Collections.disjoint(use.definitions, neededDefinitions);
      if (!required.contains(i) && !valueDependency && !sideEffectDependency && !definitionDependency) {
        continue;
      }

      selected.add(i);
      if (valueDependency) {
        dependencies.removeAll(use.definitelyAssigned);
      }
      dependencies.addAll(reads);
      neededDefinitions.addAll(use.lefts);
    }

    List<Integer> result = new ArrayList<>(selected);
    Collections.reverse(result);
    return result;
  }

  private static HelperBody bodyFromSlice(List<PreludeElement> prelude, List<Integer> slice) {
    List<Statement> statements = new ArrayList<>();
    List<Exprent> exprents = new ArrayList<>();
    for (Integer index : slice) {
      prelude.get(index).appendTo(statements, exprents);
    }
    return new HelperBody(statements, exprents);
  }

  private static void removePrefix(List<?> list, int size) {
    // Use remove(int): statement lists also maintain keys, which subList.clear()
    // would bypass in VBStyleCollection.
    for (int i = size - 1; i >= 0; i--) {
      list.remove(i);
    }
  }

  private static InvocationExprent createPreinitHelperCall(
    ClassWrapper wrapper,
    MethodWrapper method,
    VarType returnType,
    HelperBody body,
    Exprent returnValue
  ) {
    PreparedHelper helper = preparePreinitHelper(method, returnType, body, returnValue);
    return helper == null ? null : createPreinitHelperCall(wrapper, method, helper);
  }

  private static PreparedHelper preparePreinitHelper(MethodWrapper method, VarType returnType, HelperBody body, Exprent returnValue) {
    List<ClassWrapper.SourceOnlyParameter> parameters = collectSourceOnlyParameters(method, body, returnValue, Collections.emptyList());
    return parameters == null ? null : new PreparedHelper(returnType, body, returnValue, parameters);
  }

  private static InvocationExprent createPreinitHelperCall(ClassWrapper wrapper, MethodWrapper method, PreparedHelper prepared) {
    VarType returnType = prepared.returnType;
    HelperBody body = prepared.body;
    Exprent returnValue = prepared.returnValue;
    List<ClassWrapper.SourceOnlyParameter> helperParameters = prepared.parameters;
    ClassWrapper.SourceOnlyMethod helper = new ClassWrapper.SourceOnlyMethod(
      wrapper.nextSourceOnlyMethodName(PREINIT_HELPER_PREFIX),
      returnType,
      helperParameters,
      checkedExceptionsOf(method, body, returnValue),
      createSourceOnlyBody(method, body, returnValue, returnType),
      method);
    wrapper.addSourceOnlyMethod(helper);

    String descriptorString = helper.descriptorString();
    InvocationExprent invocation = new InvocationExprent();
    invocation.setName(helper.name());
    invocation.setClassname(wrapper.getClassStruct().qualifiedName);
    invocation.setStatic(true);
    invocation.setFunctype(InvocationExprent.Type.GENERAL);
    invocation.setStringDescriptor(descriptorString);
    invocation.setDescriptor(MethodDescriptor.parseDescriptor(descriptorString));

    List<Exprent> arguments = new ArrayList<>();
    for (ClassWrapper.SourceOnlyParameter parameter : helperParameters) {
      VarExprent argument = (VarExprent)parameter.exprent().copy();
      argument.setDefinition(false);
      arguments.add(argument);
    }
    invocation.setLstParameters(arguments);
    return invocation;
  }

  private static List<String> checkedExceptionsOf(
    MethodWrapper owner,
    HelperBody body,
    Exprent returnValue
  ) {
    MethodExceptionSummary.ExceptionFlow flow = MethodExceptionSummary.ExceptionFlow.EMPTY;
    for (Statement statement : body.statements) {
      flow = flow.union(owner.getExceptionSummary().flowOf(statement));
    }
    for (Exprent exprent : body.exprents) {
      flow = flow.union(owner.getExceptionSummary().flowOf(exprent));
    }
    if (returnValue != null) {
      flow = flow.union(owner.getExceptionSummary().flowOf(returnValue));
    }
    return List.copyOf(CheckedExceptionSupport.removeRedundantSubtypes(flow.checkedExceptions()));
  }

  private static List<Statement> createSourceOnlyBody(
    MethodWrapper owner,
    HelperBody body,
    Exprent returnValue,
    VarType returnType
  ) {
    DecompilerContext.resetMethod(owner);
    List<Statement> statements = new ArrayList<>(body.statements);
    if (!body.exprents.isEmpty() || returnValue != null) {
      List<Exprent> tailExprents = new ArrayList<>(body.exprents);
      if (returnValue != null) {
        tailExprents.add(new ExitExprent(
          ExitExprent.Type.RETURN,
          returnValue,
          returnType,
          returnValue.bytecode,
          null
        ));
      }
      BasicBlockStatement tail = BasicBlockStatement.create();
      tail.setExprents(tailExprents);
      statements.add(tail);
    }
    return statements;
  }

  private static List<ClassWrapper.SourceOnlyParameter> collectSourceOnlyParameters(
    MethodWrapper method,
    HelperBody body,
    Exprent returnValue,
    List<VarExprent> plannedDefinitions
  ) {
    MethodDescriptor descriptor = method.desc();
    Set<Integer> methodParameterIndexes = getMethodParameterIndexes(descriptor);

    VarUse bodyUse = varUseBody(body);
    VarUse returnUse = varUse(returnValue);
    Set<Integer> referencedIndexes = toVarIndexes(bodyUse.all);
    referencedIndexes.addAll(toVarIndexes(returnUse.all));
    Set<Integer> internalIndexes = toVarIndexes(bodyUse.definitions);
    internalIndexes.addAll(toVarIndexes(returnUse.definitions));
    // Some callers have already proved that an assignment can declare a new
    // helper-local variable, but only apply that change once extraction succeeds.
    for (VarExprent definition : plannedDefinitions) {
      internalIndexes.add(definition.getIndex());
    }
    internalIndexes.removeAll(methodParameterIndexes);
    referencedIndexes.removeAll(internalIndexes);

    if (referencedIndexes.contains(0)) {
      return null;
    }

    List<ClassWrapper.SourceOnlyParameter> parameters = new ArrayList<>();
    int localIndex = 1;
    for (int i = 0; i < descriptor.params.length; i++) {
      VarType parameterType = descriptor.params[i];
      if (referencedIndexes.contains(localIndex)) {
        if (method.synthParameters != null &&
            i < method.synthParameters.size() &&
            method.synthParameters.get(i) != null) {
          return null;
        }

        VarExprent parameterExprent = new VarExprent(localIndex, parameterType, method.varproc);
        VarVersionPair pair = new VarVersionPair(localIndex, 0);
        String name = method.varproc.getVarName(pair);
        parameters.add(new ClassWrapper.SourceOnlyParameter(
          parameterType,
          name == null ? "var" + localIndex : name,
          parameterExprent));
      }

      localIndex += parameterType.stackSize;
    }

    for (Integer referencedIndex : referencedIndexes) {
      if (referencedIndex >= localIndex || !methodParameterIndexes.contains(referencedIndex)) {
        return null;
      }
    }

    return parameters;
  }

  private static VarType getConstructorParameterType(InvocationExprent invocation, int parameterIndex) {
    MethodDescriptor descriptor = invocation.getDescriptor();
    return parameterIndex >= 0 && parameterIndex < descriptor.params.length ? descriptor.params[parameterIndex] : null;
  }

  private static Set<Integer> getMethodParameterIndexes(MethodDescriptor descriptor) {
    Set<Integer> result = new HashSet<>();
    int localIndex = 1;
    for (VarType parameter : descriptor.params) {
      result.add(localIndex);
      localIndex += parameter.stackSize;
    }
    return result;
  }

  private static Set<Integer> toVarIndexes(Set<VarVersionPair> vars) {
    Set<Integer> result = new HashSet<>();
    for (VarVersionPair var : vars) {
      result.add(var.var);
    }
    return result;
  }

  private static VarUse varUseBody(HelperBody body) {
    VarUse result = new VarUse();
    result.add(varUseExprents(body.exprents));
    for (Statement statement : body.statements) {
      result.add(varUse(statement));
    }
    return result;
  }

  private static VarUse varUseElements(List<PreludeElement> elements) {
    VarUse result = new VarUse();
    for (PreludeElement element : elements) {
      result.add(element.varUse());
    }
    return result;
  }

  private static VarUse varUseExprents(List<? extends Exprent> exprents) {
    VarUse result = new VarUse();
    for (Exprent exprent : exprents) {
      result.then(varUse(exprent));
    }
    return result;
  }

  private static VarUse varUse(Statement statement) {
    VarUse result = new VarUse();
    result.add(varUseExprents(statement.getVarDefinitions()));
    if (statement.getExprents() != null) {
      result.then(varUseExprents(statement.getExprents()));
    } else {
      result.add(varUseExprents(statement.getStatExprents()));
      for (Statement child : statement.getStats()) {
        result.add(varUse(child));
      }
    }
    return result;
  }

  private static VarUse varUse(Exprent exprent) {
    VarUse result = new VarUse();
    if (exprent == null) {
      return result;
    }

    if (exprent instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent left) {
      result.then(varUse(assignment.getRight()));
      VarVersionPair pair = new VarVersionPair(left);
      result.all.add(pair);
      // Stack duplication can leave x = x inside a condition. It reads x but
      // does not change the value that an unchanged constructor argument sees.
      boolean selfCopy = assignment.getCondType() == null && assignment.getRight() instanceof VarExprent right &&
        pair.equals(new VarVersionPair(right));
      if (!selfCopy) {
        result.lefts.add(pair);
        result.assigned.add(pair);
        result.definitelyAssigned.add(pair);
      }
      if (left.isDefinition()) result.definitions.add(pair);
      if (assignment.getCondType() != null) result.reads.add(pair);
    } else if (exprent instanceof VarExprent var) {
      VarVersionPair pair = new VarVersionPair(var);
      result.all.add(pair);
      if (var.isDefinition()) {
        result.definitions.add(pair);
      } else {
        result.reads.add(pair);
      }
    } else if (exprent instanceof FunctionExprent function && function.getFuncType() == FunctionType.TERNARY) {
      result.then(varUse(function.getLstOperands().get(0)));
      VarUse ifUse = varUse(function.getLstOperands().get(1));
      VarUse elseUse = varUse(function.getLstOperands().get(2));
      VarUse branches = new VarUse();
      branches.add(ifUse);
      branches.add(elseUse);
      branches.definitelyAssigned.addAll(ifUse.definitelyAssigned);
      branches.definitelyAssigned.retainAll(elseUse.definitelyAssigned);
      result.then(branches);
    } else {
      for (Exprent child : exprent.getAllExprents()) {
        result.then(varUse(child));
      }
      if (exprent instanceof FunctionExprent function) {
        if (function.getFuncType().isPPMM() && function.getLstOperands().get(0) instanceof VarExprent var) {
          VarVersionPair pair = new VarVersionPair(var);
          result.assigned.add(pair);
          result.definitelyAssigned.add(pair);
          result.lefts.add(pair);
        } else if (function.getFuncType() == FunctionType.BOOLEAN_AND || function.getFuncType() == FunctionType.BOOLEAN_OR) {
          result.definitelyAssigned.retainAll(varUse(function.getLstOperands().get(0)).definitelyAssigned);
        }
      }
    }
    return result;
  }

  private record ConstructorCall(List<Exprent> exprents, int index, InvocationExprent invocation, Statement statement) {}

  private record PreparedHelper(VarType returnType, HelperBody body, Exprent returnValue,
                                List<ClassWrapper.SourceOnlyParameter> parameters) {}

  private record HelperBody(List<Statement> statements, List<Exprent> exprents) {}

  private record ParameterSlice(int parameter, List<Integer> indexes) {}

  private record PreludeElement(Statement statement, Exprent exprent) {
    private VarUse varUse() {
      return statement != null ? InitializerProcessor.varUse(statement) : InitializerProcessor.varUse(exprent);
    }

    private void appendTo(List<Statement> statements, List<Exprent> exprents) {
      if (statement != null) {
        statements.add(statement);
      }
      else {
        // Exception summaries are keyed by expression identity. Keep the
        // analyzed nodes when transferring work into a source-only helper.
        exprents.add(exprent);
      }
    }

    private boolean isRepeatable() {
      return isLocalCopy(exprent);
    }
  }

  private static class VarUse {
    private final Set<VarVersionPair> all = new HashSet<>();
    private final Set<VarVersionPair> lefts = new HashSet<>();
    private final Set<VarVersionPair> definitions = new HashSet<>();
    private final Set<VarVersionPair> assigned = new HashSet<>();
    // Only unconditional writes kill an incoming value dependency.
    private final Set<VarVersionPair> definitelyAssigned = new HashSet<>();
    private final Set<VarVersionPair> reads = new HashSet<>();

    // Reads are incoming value dependencies, not every syntactic variable use.
    // In (x = read()) != 0 ? x : fallback(), the reads of x use the new value.
    private void then(VarUse other) {
      Set<VarVersionPair> incoming = new HashSet<>(other.reads);
      incoming.removeAll(definitelyAssigned);
      all.addAll(other.all);
      lefts.addAll(other.lefts);
      definitions.addAll(other.definitions);
      assigned.addAll(other.assigned);
      reads.addAll(incoming);
      definitelyAssigned.addAll(other.definitelyAssigned);
    }

    private void add(VarUse other) {
      // Combine possible accesses without assuming a particular execution path.
      all.addAll(other.all);
      lefts.addAll(other.lefts);
      definitions.addAll(other.definitions);
      assigned.addAll(other.assigned);
      reads.addAll(other.reads);
    }
  }

  private static void liftConstructor(ClassWrapper wrapper) {
    for (MethodWrapper method : wrapper.getMethods()) {
      if (CodeConstants.INIT_NAME.equals(method.methodStruct.getName()) && method.root != null) {
        Statement firstData = Statements.findFirstData(method.root);
        if (firstData == null) {
          return;
        }

        int index = 0;
        List<Exprent> lstExprents = firstData.getExprents();

        for (Exprent exprent : lstExprents) {
          int action = 0;

          if (exprent instanceof AssignmentExprent) {
            AssignmentExprent assignExpr = (AssignmentExprent)exprent;
            if (assignExpr.getLeft() instanceof FieldExprent && assignExpr.getRight() instanceof VarExprent) {
              FieldExprent fExpr = (FieldExprent)assignExpr.getLeft();
              if (fExpr.getClassname().equals(wrapper.getClassStruct().qualifiedName)) {
                StructField structField = wrapper.getClassStruct().getField(fExpr.getName(), fExpr.getDescriptor().descriptorString);
                if (structField != null && structField.hasModifier(CodeConstants.ACC_FINAL)) {
                  action = 1;
                }
              }
            }
          }
          else if (index > 0 && exprent instanceof InvocationExprent &&
                   Statements.isInvocationInitConstructor((InvocationExprent)exprent, method, wrapper, true)) {
            // this() or super()
            lstExprents.add(0, lstExprents.remove(index));
            action = 2;
          }

          if (action != 1) {
            break;
          }

          index++;
        }
      }
    }
  }

  private static void hideEmptySuper(ClassWrapper wrapper) {
    for (MethodWrapper method : wrapper.getMethods()) {
      if (CodeConstants.INIT_NAME.equals(method.methodStruct.getName()) && method.root != null) {
        Statement firstData = method.root.getBasichead();
        if (firstData == null || firstData.getExprents().isEmpty()) {
          return;
        }

        Exprent exprent = firstData.getExprents().get(0);
        if (exprent instanceof InvocationExprent) {
          InvocationExprent invExpr = (InvocationExprent)exprent;
          if (Statements.isInvocationInitConstructor(invExpr, method, wrapper, false)) {
            List<VarVersionPair> mask = ExprUtil.getSyntheticParametersMask(invExpr.getClassname(), invExpr.getStringDescriptor(), invExpr.getLstParameters().size());
            boolean hideSuper = true;

            //searching for non-synthetic params
            for (int i = 0; i < invExpr.getDescriptor().params.length; ++i) {
              if (mask != null && mask.get(i) != null) {
                continue;
              }
              VarType type = invExpr.getDescriptor().params[i];
              if (type.type == CodeType.OBJECT) {
                ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(type.value);
                if (node != null && (node.type == ClassNode.Type.ANONYMOUS || (node.access & CodeConstants.ACC_SYNTHETIC) != 0)) {
                  break; // Should be last
                }
              }
              hideSuper = false; // found non-synthetic param so we keep the call
              break;
            }

            if (hideSuper) {
              firstData.getExprents().remove(0);
            }
          }
        }
      }
    }
  }

  public static void hideInitalizers(ClassWrapper wrapper) {
    // hide initializers with anon class arguments
    for (MethodWrapper method : wrapper.getMethods()) {
      StructMethod mt = method.methodStruct;
      String name = mt.getName();
      String desc = mt.getDescriptor();

      if (mt.isSynthetic() && CodeConstants.INIT_NAME.equals(name)) {
        MethodDescriptor md = MethodDescriptor.parseDescriptor(desc);
        if (md.params.length > 0) {
          VarType type = md.params[md.params.length - 1];
          if (type.type == CodeType.OBJECT) {
            ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(type.value);
            if (node != null && ((node.type == ClassNode.Type.ANONYMOUS) || (node.access & CodeConstants.ACC_SYNTHETIC) != 0)) {
              //TODO: Verify that the body is JUST a this([args]) call?
              wrapper.hideMember(InterpreterUtil.makeUniqueKey(name, desc));
            }
          }
        }
      }
    }
  }

  private static void extractStaticInitializers(ClassWrapper wrapper, MethodWrapper method) {
    RootStatement root = method.root;
    StructClass cl = wrapper.getClassStruct();
    Set<String> whitelist = new HashSet<String>();
    Statement firstData = Statements.findFirstData(root);
    if (firstData != null) {
      boolean inlineInitializers = cl.hasModifier(CodeConstants.ACC_ENUM);
      List<AssignmentExprent> exprentsToRemove = new LinkedList<>();//when we loop back through the list, stores ones we need to remove outside iterator loop
      Map<Integer, AssignmentExprent> nonFieldAssigns = new HashMap<>();

      // Store fields that have been assigned to more than once. These aren't safe to inline.
      Set<String> seen = new HashSet<>();
      Set<String> multiAssign = new HashSet<>();

      if (cl.hasModifier(CodeConstants.ACC_INTERFACE)) {
        splitLeadingInterfaceFieldAssignments(firstData, cl);
      }

      for (Exprent exprent : firstData.getExprents()) {
        for (Exprent nested : exprent.getAllExprents(true, true)) {
          if (!(nested instanceof AssignmentExprent assignExpr)) {
            continue;
          }
          if (assignExpr.getLeft() instanceof FieldExprent) {
            FieldExprent fExpr = (FieldExprent) assignExpr.getLeft();

            // If the field has been seen already, add it to the list of multi-assigned fields
            String key = InterpreterUtil.makeUniqueKey(fExpr.getName(), fExpr.getDescriptor().descriptorString);
            if (!seen.add(key)) {
              multiAssign.add(key);
            }
          }
        }
      }

      if (cl.hasModifier(CodeConstants.ACC_INTERFACE)) {
        extractInterfaceStaticInitializers(wrapper, method, cl, firstData, whitelist, multiAssign);
        return;
      }

      List<FieldExprent> notInlined = new ArrayList<>();
      boolean seenRetainedClinitExprent = false;
      int previousInlinedStaticFieldIndex = -1;

      Iterator<Exprent> itr = firstData.getExprents().iterator();
      while (itr.hasNext()) {
        Exprent exprent = itr.next();
        boolean removedExprent = false;

        if (exprent instanceof AssignmentExprent) {
          AssignmentExprent assignExpr = (AssignmentExprent)exprent;
          if (assignExpr.getLeft() instanceof FieldExprent) {
            FieldExprent fExpr = (FieldExprent)assignExpr.getLeft();
            if (fExpr.isStatic() && fExpr.getClassname().equals(cl.qualifiedName) &&
                cl.hasField(fExpr.getName(), fExpr.getDescriptor().descriptorString)) {

              String keyField = InterpreterUtil.makeUniqueKey(fExpr.getName(), fExpr.getDescriptor().descriptorString);
              int fieldIndex = cl.getFields().getIndexByKey(keyField);
              // Lifted static initializers are emitted with fields, so they execute in declaration order.
              // Stop lifting when bytecode assignment order would move backwards in that order.
              boolean preservesClinitOrder = previousInlinedStaticFieldIndex <= fieldIndex;
              boolean canConsiderInitializer = inlineInitializers || (preservesClinitOrder && !seenRetainedClinitExprent);
              List<String> checkedInitializerExceptions = canConsiderInitializer
                ? method.getExceptionSummary().flowOf(assignExpr.getRight()).checkedExceptions()
                : Collections.emptyList();
              boolean exprentIndependent = canConsiderInitializer &&
                  isExprentIndependent(fExpr, assignExpr.getRight(), method, cl, whitelist, multiAssign, notInlined, fieldIndex, true) &&
                  (inlineInitializers || checkedInitializerExceptions.isEmpty());
              if (inlineInitializers || exprentIndependent) {
                if (!wrapper.getStaticFieldInitializers().containsKey(keyField)) {
                  if (exprentIndependent) {
                    Exprent initializer = assignExpr.getRight();
                    StructField field = cl.getFields().getWithKey(keyField);
                    boolean needsCheckedHolder = !checkedInitializerExceptions.isEmpty()
                      && cl.hasModifier(CodeConstants.ACC_ENUM)
                      && !field.hasModifier(CodeConstants.ACC_ENUM);
                    if (needsCheckedHolder) {
                      initializer = createCheckedInlineInitializer(
                        wrapper,
                        method,
                        new VarType(field.getDescriptor()),
                        initializer,
                        checkedInitializerExceptions
                      );
                    }
                    wrapper.getStaticFieldInitializers().addWithKey(initializer, keyField);
                    whitelist.add(keyField);
                    itr.remove();
                    removedExprent = true;
                    previousInlinedStaticFieldIndex = fieldIndex;
                  } else { //inlineInitializers
                    if (assignExpr.getRight() instanceof NewExprent){
                      NewExprent newExprent = (NewExprent) assignExpr.getRight();
                      if (newExprent.getConstructor() == null) {
                        continue;
                      }

                      Exprent instance = newExprent.getConstructor().getInstance();
                      if (instance instanceof VarExprent && nonFieldAssigns.containsKey(((VarExprent) instance).getIndex())){
                        AssignmentExprent nonFieldAssignment = nonFieldAssigns.remove(((VarExprent) instance).getIndex());
                        newExprent.getConstructor().setInstance(nonFieldAssignment.getRight());
                        exprentsToRemove.add(nonFieldAssignment);
                        wrapper.getStaticFieldInitializers().addWithKey(assignExpr.getRight(), keyField);
                        whitelist.add(keyField);
                        itr.remove();
                        removedExprent = true;
                        previousInlinedStaticFieldIndex = fieldIndex;
                      } else {
//                        DecompilerContext.getLogger().writeMessage("Don't know how to handle non independent "+assignExpr.getRight().getClass().getName(), IFernflowerLogger.Severity.ERROR);
                      }
                    } else {
//                      DecompilerContext.getLogger().writeMessage("Don't know how to handle non independent "+assignExpr.getRight().getClass().getName(), IFernflowerLogger.Severity.ERROR);
                    }
                  }
                }
              } else {
                notInlined.add(fExpr);
              }
            }
          } else if (inlineInitializers) {
//            DecompilerContext.getLogger().writeMessage("Found non field assignment when needing to force inline: "+assignExpr.toString(), IFernflowerLogger.Severity.TRACE);
            if (assignExpr.getLeft() instanceof VarExprent) {
              nonFieldAssigns.put(((VarExprent) assignExpr.getLeft()).getIndex(), assignExpr);
            } else {
//              DecompilerContext.getLogger().writeMessage("Left is not VarExprent!", IFernflowerLogger.Severity.ERROR);
            }
          }
        }

        if (!inlineInitializers && !removedExprent) {
          seenRetainedClinitExprent = true;
        }
      }
      if (exprentsToRemove.size() > 0){
        firstData.getExprents().removeAll(exprentsToRemove);
      }
    }

    // Ensure enum fields have been inlined
    if (cl.hasModifier(CodeConstants.ACC_ENUM)) {
      for (StructField fd : cl.getFields()) {
        if (fd.hasModifier(CodeConstants.ACC_ENUM)) {
          if (wrapper.getStaticFieldInitializers().getWithKey(InterpreterUtil.makeUniqueKey(fd.getName(), fd.getDescriptor())) == null) {
            method.addComment("$VF: Failed to inline enum fields");
            method.addErrorComment = true;
            break;
          }
        }
      }
    }
  }

  private static void extractInterfaceStaticInitializers(
    ClassWrapper wrapper,
    MethodWrapper method,
    StructClass cl,
    Statement firstData,
    Set<String> whitelist,
    Set<String> multiAssign
  ) {
    List<Exprent> pending = new ArrayList<>();
    List<Exprent> exprentsToRemove = new ArrayList<>();
    int previousFieldIndex = -1;
    boolean blockedByUnrepresentableAssignment = false;

    for (Exprent exprent : new ArrayList<>(firstData.getExprents())) {
      AssignmentExprent assignment = exprent instanceof AssignmentExprent ? (AssignmentExprent)exprent : null;
      FieldExprent field = assignment != null && assignment.getLeft() instanceof FieldExprent
        ? (FieldExprent)assignment.getLeft()
        : null;
      boolean ownStaticField = field != null && isOwnStaticField(field, cl);

      if (!blockedByUnrepresentableAssignment && ownStaticField) {
        String key = InterpreterUtil.makeUniqueKey(field.getName(), field.getDescriptor().descriptorString);
        int fieldIndex = cl.getFields().getIndexByKey(key);
        boolean preservesDeclarationOrder = previousFieldIndex <= fieldIndex;
        boolean assignedOnce = !multiAssign.contains(key);

        if (preservesDeclarationOrder && assignedOnce && !wrapper.getStaticFieldInitializers().containsKey(key)) {
          StructField structField = cl.getFields().getWithKey(key);
          VarType fieldType = new VarType(structField.getDescriptor());
          Exprent initializer = createInterfaceFieldInitializer(
            wrapper,
            method,
            cl,
            field,
            fieldType,
            assignment.getRight(),
            pending,
            whitelist,
            multiAssign,
            fieldIndex
          );

          if (initializer != null) {
            // Interface source has no initializer blocks. Keep each residual slice immediately before the
            // field assignment that followed it in bytecode by evaluating both inside one helper call.
            wrapper.getStaticFieldInitializers().addWithKey(initializer, key);
            whitelist.add(key);
            exprentsToRemove.addAll(pending);
            pending.clear();
            exprentsToRemove.add(exprent);
            previousFieldIndex = fieldIndex;
            continue;
          }
        }
      }

      pending.add(exprent);
      if (ownStaticField) {
        // A source field initializer cannot reproduce a second, conditional, or backwards write to an
        // interface final field. Later extraction must not move anything across that write.
        blockedByUnrepresentableAssignment = true;
      }
    }

    firstData.getExprents().removeAll(exprentsToRemove);
  }

  private static void splitLeadingInterfaceFieldAssignments(Statement firstData, StructClass cl) {
    List<Exprent> normalized = new ArrayList<>();
    for (Exprent exprent : firstData.getExprents()) {
      appendWithLeadingInterfaceFieldAssignments(normalized, exprent, cl);
    }
    firstData.setExprents(normalized);
  }

  private static void appendWithLeadingInterfaceFieldAssignments(
    List<Exprent> result,
    Exprent exprent,
    StructClass cl
  ) {
    if (exprent instanceof AssignmentExprent outer
      && outer.getLeft() instanceof FieldExprent outerField
      && isOwnStaticField(outerField, cl)) {
      LeadingInterfaceFieldAssignment leading = findLeadingInterfaceFieldAssignment(outer.getRight(), null, cl);
      if (leading != null) {
        AssignmentExprent assignment = leading.assignment();
        FieldExprent field = (FieldExprent)assignment.getLeft();
        leading.replaceWith((FieldExprent)field.copy(), outer);

        // A putstatic whose value remains on the operand stack can be folded into the next field's
        // initializer expression. Split only an assignment on the first-evaluated operand path, where
        // making the two field writes explicit cannot cross another evaluation or side effect.
        appendWithLeadingInterfaceFieldAssignments(result, assignment, cl);
      }
    }
    result.add(exprent);
  }

  private static Exprent createInterfaceFieldInitializer(
    ClassWrapper wrapper,
    MethodWrapper method,
    StructClass cl,
    FieldExprent field,
    VarType fieldType,
    Exprent value,
    List<Exprent> pending,
    Set<String> whitelist,
    Set<String> multiAssign,
    int fieldIndex
  ) {
    if (containsOwnStaticFieldAssignment(value, cl)
      || pending.stream().anyMatch(exprent -> containsOwnStaticFieldAssignment(exprent, cl))) {
      // Assignments to an interface field are only legal in that field's declaration. Any nested
      // write which could not be split above must remain a conservative extraction barrier.
      return null;
    }

    if (!pending.isEmpty()) {
      return createResidualInterfaceInitializer(wrapper, method, fieldType, pending, value);
    }

    if (!isExprentIndependent(
      field,
      value,
      method,
      cl,
      whitelist,
      multiAssign,
      Collections.emptyList(),
      fieldIndex,
      true
    )) {
      return null;
    }

    List<String> checkedExceptions = method.getExceptionSummary().flowOf(value).checkedExceptions();
    return checkedExceptions.isEmpty()
      ? value
      : createCheckedInlineInitializer(wrapper, method, fieldType, value, checkedExceptions);
  }

  private static boolean containsOwnStaticFieldAssignment(Exprent exprent, StructClass cl) {
    for (Exprent nested : exprent.getAllExprents(true, true)) {
      if (nested instanceof AssignmentExprent assignment
        && assignment.getLeft() instanceof FieldExprent field
        && isOwnStaticField(field, cl)) {
        return true;
      }
    }
    return false;
  }

  private static LeadingInterfaceFieldAssignment findLeadingInterfaceFieldAssignment(
    Exprent exprent,
    FunctionExprent parent,
    StructClass cl
  ) {
    if (exprent instanceof AssignmentExprent assignment
      && assignment.getCondType() == null
      && assignment.getLeft() instanceof FieldExprent field
      && isOwnStaticField(field, cl)) {
      return new LeadingInterfaceFieldAssignment(assignment, parent);
    }

    if (exprent instanceof FunctionExprent function && !function.getLstOperands().isEmpty()) {
      return findLeadingInterfaceFieldAssignment(function.getLstOperands().get(0), function, cl);
    }
    return null;
  }

  private record LeadingInterfaceFieldAssignment(AssignmentExprent assignment, FunctionExprent parent) {
    private void replaceWith(FieldExprent replacement, AssignmentExprent outerAssignment) {
      if (parent == null) {
        outerAssignment.setRight(replacement);
      }
      else {
        parent.getLstOperands().set(0, replacement);
      }
    }
  }

  private static boolean isOwnStaticField(FieldExprent field, StructClass cl) {
    return field.isStatic()
      && field.getClassname().equals(cl.qualifiedName)
      && cl.hasField(field.getName(), field.getDescriptor().descriptorString);
  }

  private static InvocationExprent createResidualInterfaceInitializer(
    ClassWrapper wrapper,
    MethodWrapper owner,
    VarType returnType,
    List<Exprent> prefix,
    Exprent value
  ) {
    List<VarExprent> promotedDefinitions = findResidualInterfaceLocalDefinitions(owner, prefix, value);
    if (promotedDefinitions == null) {
      return null;
    }

    HelperBody body = new HelperBody(Collections.emptyList(), prefix);
    List<ClassWrapper.SourceOnlyParameter> parameters = collectSourceOnlyParameters(owner, body, value, promotedDefinitions);
    if (parameters == null || !parameters.isEmpty()) {
      return null;
    }

    // Validation above is deliberately side-effect free: rejected extraction must not alter the
    // variable declarations in the <clinit> that remains in place.
    promotedDefinitions.forEach(variable -> variable.setDefinition(true));

    return createInlineInitializerHelper(
      wrapper,
      owner,
      "VFInterfaceInitializer",
      returnType,
      prefix,
      value,
      checkedExceptionsOf(owner, body, value)
    );
  }

  private static List<VarExprent> findResidualInterfaceLocalDefinitions(
    MethodWrapper owner,
    List<Exprent> prefix,
    Exprent value
  ) {
    Set<VarVersionPair> available = new HashSet<>(owner.varproc.getExternalVars());
    List<VarExprent> promotedDefinitions = new ArrayList<>();
    for (Exprent exprent : prefix) {
      if (exprent instanceof AssignmentExprent assignment && assignment.getLeft() instanceof VarExprent left) {
        if (!available.containsAll(varUse(assignment.getRight()).reads)) {
          return null;
        }

        VarVersionPair pair = new VarVersionPair(left);
        if (assignment.getCondType() != null && !available.contains(pair)) {
          return null;
        }
        if (!available.contains(pair)) {
          // Splitting one <clinit> local scope across helper methods requires a fresh declaration in
          // every slice whose first use overwrites the old value before reading it.
          if (!left.isDefinition()) {
            promotedDefinitions.add(left);
          }
        }
        available.add(pair);
      }
      else {
        VarUse use = varUse(exprent);
        Set<VarVersionPair> assignedWithoutDefinition = new HashSet<>(use.lefts);
        assignedWithoutDefinition.removeAll(use.definitions);
        if (!available.containsAll(use.reads) || !available.containsAll(assignedWithoutDefinition)) {
          return null;
        }
        available.addAll(use.definitions);
      }
    }

    VarUse valueUse = varUse(value);
    return available.containsAll(valueUse.reads) && available.containsAll(valueUse.lefts)
      ? promotedDefinitions
      : null;
  }

  private static InvocationExprent createCheckedInlineInitializer(
    ClassWrapper wrapper,
    MethodWrapper owner,
    VarType returnType,
    Exprent value,
    List<String> checkedExceptions
  ) {
    return createInlineInitializerHelper(
      wrapper,
      owner,
      "VFCheckedInitializer",
      returnType,
      Collections.emptyList(),
      value,
      checkedExceptions
    );
  }

  private static InvocationExprent createInlineInitializerHelper(
    ClassWrapper wrapper,
    MethodWrapper owner,
    String holderName,
    VarType returnType,
    List<Exprent> prefix,
    Exprent value,
    List<String> checkedExceptions
  ) {
    DecompilerContext.resetMethod(owner);
    ClassWrapper.SourceOnlyClass holder =
      wrapper.getOrCreateSourceOnlyClass(
        holderName,
        CodeConstants.ACC_STATIC | CodeConstants.ACC_FINAL
      );
    String methodName = "$VF$init" + holder.methods().size();

    BasicBlockStatement returnBlock = BasicBlockStatement.create();
    List<Exprent> bodyExprents = new ArrayList<>(prefix);
    bodyExprents.add(new ExitExprent(
      ExitExprent.Type.RETURN,
      value,
      returnType,
      value.bytecode,
      null
    ));
    returnBlock.setExprents(bodyExprents);
    Statement body = checkedExceptions.isEmpty()
      ? returnBlock
      : CheckedExceptionRepairProcessor.createRuntimeWrapper(returnBlock, owner, checkedExceptions);
    ClassWrapper.SourceOnlyMethod helper = new ClassWrapper.SourceOnlyMethod(
      methodName,
      returnType,
      List.of(),
      List.of(),
      List.of(body),
      owner
    );
    holder.addMethod(helper);

    String descriptor = helper.descriptorString();
    InvocationExprent invocation = new InvocationExprent();
    invocation.setName(methodName);
    // The holder is emitted lexically inside the owning class. Its simple name avoids an invalid
    // same-class import when that owner is in the default package.
    invocation.setClassname(holder.name());
    invocation.setStringDescriptor(descriptor);
    invocation.setDescriptor(MethodDescriptor.parseDescriptor(descriptor));
    invocation.setFunctype(InvocationExprent.Type.GENERAL);
    invocation.setStatic(true);
    invocation.setLstParameters(List.of());
    return invocation;
  }

  private static void extractDynamicInitializers(ClassWrapper wrapper) {
    StructClass cl = wrapper.getClassStruct();

    boolean isAnonymous = DecompilerContext.getClassProcessor().getMapRootClasses().get(cl.qualifiedName).type == ClassNode.Type.ANONYMOUS;

    List<List<Exprent>> lstFirst = new ArrayList<>();
    List<MethodWrapper> lstMethodWrappers = new ArrayList<>();

    for (MethodWrapper method : wrapper.getMethods()) {
      if (CodeConstants.INIT_NAME.equals(method.methodStruct.getName()) && method.root != null) { // successfully decompiled constructor
        Statement firstData = Statements.findFirstData(method.root);
        if (firstData == null || firstData.getExprents().isEmpty()) {
          continue;
        }

        Exprent exprent = firstData.getExprents().get(0);
        if (!isAnonymous) { // FIXME: doesn't make sense
          if (!(exprent instanceof InvocationExprent) ||
              !Statements.isInvocationInitConstructor((InvocationExprent)exprent, method, wrapper, false)) {
            continue;
          }
        }
        lstFirst.add(firstData.getExprents());
        lstMethodWrappers.add(method);
      }
    }

    if (lstFirst.isEmpty()) {
      return;
    }

    Set<String> whitelist = new HashSet<String>(wrapper.getStaticFieldInitializers().getLstKeys());
    int prev_fidx = 0;

    while (true) {
      String fieldWithDescr = null;
      Exprent value = null;

      for (int i = 0; i < lstFirst.size(); i++) {
        List<Exprent> lst = lstFirst.get(i);

        if (lst.size() < (isAnonymous ? 1 : 2)) {
          return;
        }

        Exprent exprent = lst.get(isAnonymous ? 0 : 1);

        boolean found = false;

        if (exprent instanceof AssignmentExprent) {
          AssignmentExprent assignExpr = (AssignmentExprent)exprent;
          if (assignExpr.getLeft() instanceof FieldExprent) {
            FieldExprent fExpr = (FieldExprent)assignExpr.getLeft();
            if (!fExpr.isStatic() && fExpr.getClassname().equals(cl.qualifiedName) &&
                cl.hasField(fExpr.getName(), fExpr.getDescriptor().descriptorString)) { // check for the physical existence of the field. Could be defined in a superclass.

              String fieldKey = InterpreterUtil.makeUniqueKey(fExpr.getName(), fExpr.getDescriptor().descriptorString);
              int fidx = cl.getFields().getIndexByKey(fieldKey);
              if (prev_fidx <= fidx && isExprentIndependent(fExpr, assignExpr.getRight(), lstMethodWrappers.get(i), cl, whitelist, Collections.emptySet() /* TODO */, new ArrayList<>(),  fidx, false)) {
                prev_fidx = fidx;
                if (fieldWithDescr == null) {
                  fieldWithDescr = fieldKey;
                  value = assignExpr.getRight();
                }
                else {
                  if (!fieldWithDescr.equals(fieldKey) ||
                      !value.equals(assignExpr.getRight())) {
                    return;
                  }
                }
                found = true;
              }
            }
          }
        }

        if (!found) {
          return;
        }
      }

      if (!wrapper.getDynamicFieldInitializers().containsKey(fieldWithDescr)) {
        // Some very last minute things to catch bugs with initializing and inlining
        value = processDynamicInitializer(value);
        wrapper.getDynamicFieldInitializers().addWithKey(value, fieldWithDescr);
        whitelist.add(fieldWithDescr);

        for (List<Exprent> lst : lstFirst) {
          lst.remove(isAnonymous ? 0 : 1);
        }
      }
      else {
        return;
      }
    }
  }

  private static Exprent processDynamicInitializer(Exprent expr) {

    if (expr instanceof FunctionExprent) {
      Exprent temp = expr;
      // Find function inside casts
      while (temp instanceof FunctionExprent && (((FunctionExprent) temp).getFuncType().castType != null || ((FunctionExprent) temp).getFuncType() == FunctionType.CAST)) {
        temp = ((FunctionExprent) temp).getLstOperands().get(0);
      }

      if (temp instanceof FunctionExprent) {
        FunctionExprent func = (FunctionExprent) temp;

        if (ExprProcessor.shouldDecompileAutoboxing()) {
          // Force unwrap boxing in function
          func.unwrapBox();
        }

        expr = func;
      }
    } else {
      // boolean b = obj; -> boolean b = (Boolean)obj;
      expr = processBoxingCast(expr);
    }

    return expr;
  }

  private static Exprent processBoxingCast(Exprent expr) {
    if (expr instanceof InvocationExprent) {
      if (ExprProcessor.shouldDecompileAutoboxing() && ((InvocationExprent) expr).isUnboxingCall()) {
        Exprent inner = ((InvocationExprent) expr).getInstance();
        if (inner instanceof FunctionExprent && ((FunctionExprent)inner).getFuncType() == FunctionType.CAST) {
          inner.addBytecodeOffsets(expr.bytecode);
          expr = inner;
        }
      }
    }

    return expr;
  }

  private static boolean isExprentIndependent(FieldExprent field, Exprent exprent, MethodWrapper method, StructClass cl, Set<String> whitelist, Set<String> multiAssign, List<FieldExprent> notInlined, int fidx, boolean isStatic) {
    String keyField = InterpreterUtil.makeUniqueKey(field.getName(), field.getDescriptor().descriptorString);
    List<Exprent> lst = exprent.getAllExprents(true, true);

    for (Exprent expr : lst) {
      switch (expr.type) {
        case VAR:
          VarVersionPair varPair = new VarVersionPair((VarExprent)expr);
          if (!method.varproc.getExternalVars().contains(varPair)) {
            String varName = method.varproc.getVarName(varPair);
            if (varName == null || !varName.equals("this") && !varName.endsWith(".this")) { // FIXME: remove direct comparison with strings
              return false;
            }
          }
          break;
        case FIELD:
          FieldExprent fexpr = (FieldExprent)expr;
          if (notInlined.contains(fexpr)) {
            return false;
          }

          if (cl.hasField(fexpr.getName(), fexpr.getDescriptor().descriptorString)) {
            String key = InterpreterUtil.makeUniqueKey(fexpr.getName(), fexpr.getDescriptor().descriptorString);
            if (isStatic) {
              // If this field has been assigned to more than once, we can't assume it's safe to inline
              if (multiAssign.contains(key)) {
                return false;
              }

              if (!fexpr.isStatic()) {
                return false;
              } else if (cl.getFields().getIndexByKey(key) >= fidx) {
                fexpr.forceQualified(true);
              }
            } else {
              if (!whitelist.contains(key)) {
                return false;
              } else if (cl.getFields().getIndexByKey(key) > fidx) {
                return false;
              }
            }
          }
          else if (!fexpr.isStatic() && fexpr.getInstance() == null) {
            return false;
          }
          break;
        case NEW:
          qualifyFieldReferences((NewExprent)expr, cl, fidx);
          break;
      }
    }

    return true;
  }

  // Qualifies field references to future static fields in lambdas
  private static void qualifyFieldReferences(NewExprent nexpr, StructClass cl, int fidx) {
    boolean isStatic = cl.getFields().get(fidx).hasModifier(CodeConstants.ACC_STATIC);
    if (isStatic && nexpr.isLambda() && !nexpr.isMethodReference()) {
      ClassNode child = DecompilerContext.getClassProcessor().getMapRootClasses().get(nexpr.getNewType().value);
      MethodWrapper wrapper = child.parent.getWrapper().getMethods().getWithKey(child.lambdaInformation.content_method_key);

      Set<Exprent> s = new HashSet<>();
      wrapper.getOrBuildGraph().iterateExprentsDeep(e -> {
        if (e instanceof FieldExprent || e instanceof NewExprent)
          s.add(e);
        return 0;
      });
      for (Exprent e : s) {
        switch (e.type) {
          case FIELD:
            FieldExprent fe = (FieldExprent)e;
            if (cl.qualifiedName.equals(fe.getClassname()) && fe.isStatic() && cl.hasField(fe.getName(), fe.getDescriptor().descriptorString)) {
              String key = InterpreterUtil.makeUniqueKey(fe.getName(), fe.getDescriptor().descriptorString);
              if (fe.getInstance() == null && cl.getFields().getIndexByKey(key) > fidx) {
                fe.forceQualified(true);
              }
            }
            break;
          case NEW:
            qualifyFieldReferences((NewExprent)e, cl, fidx);
            break;
        }
      }
    }

  }
}
