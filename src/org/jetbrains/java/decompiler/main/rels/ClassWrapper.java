// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.main.rels;

import org.jetbrains.java.decompiler.api.plugin.LanguageSpec;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.cfg.ControlFlowGraph;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.collectors.CounterContainer;
import org.jetbrains.java.decompiler.main.collectors.VarNamesCollector;
import org.jetbrains.java.decompiler.main.decompiler.CancelationManager;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.modules.decompiler.flow.FlattenStatementsHelper;
import org.jetbrains.java.decompiler.modules.decompiler.stats.RootStatement;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarProcessor;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.attr.StructLocalVariableTableAttribute;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DotExporter;
import org.jetbrains.java.decompiler.util.InterpreterUtil;
import org.jetbrains.java.decompiler.util.collections.VBStyleCollection;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ForkJoinPool;
import java.util.concurrent.ForkJoinTask;
import java.util.concurrent.TimeoutException;

public class ClassWrapper {
  // Sometimes when debugging you want to be able to only analyze a specific method.
  // When not null, this skips processing of every method except the one with the name specified.
  private static final String DEBUG_METHOD_FILTER = null;
  // Method-level tasks only pay for themselves once a class is much larger than
  // the normal class-level scheduling granularity.
  private static final int MIN_PARALLEL_CODE_METHODS = 500;
  private final StructClass classStruct;
  private final Set<String> hiddenMembers = ConcurrentHashMap.newKeySet();
  private final VBStyleCollection<Exprent, String> staticFieldInitializers = new VBStyleCollection<>();
  private final VBStyleCollection<Exprent, String> dynamicFieldInitializers = new VBStyleCollection<>();
  private final VBStyleCollection<MethodWrapper, String> methods = new VBStyleCollection<>();
  private final List<SourceOnlyMethod> sourceOnlyMethods = new ArrayList<>();
  private final Map<String, SourceOnlyMethod> sourceOnlyMethodsByKey = new HashMap<>();
  private final List<SourceOnlyClass> sourceOnlyClasses = new ArrayList<>();
  private final List<MissingAbstractMethod> missingAbstractMethods = new ArrayList<>();
  private final Set<String> missingAbstractMethodKeys = new HashSet<>();
  private final Set<String> requiredSourceMethodKeys = new HashSet<>();
  private final Set<String> abstractMethodFallbackKeys = new HashSet<>();
  private int sourceOnlyMethodCounter;

  public ClassWrapper(StructClass classStruct) {
    this.classStruct = classStruct;
  }

  public void init(LanguageSpec spec) {
    DecompilerContext.setProperty(DecompilerContext.CURRENT_CLASS, classStruct);
    DecompilerContext.setProperty(DecompilerContext.CURRENT_CLASS_WRAPPER, this);
    DecompilerContext.getLogger().startClass(classStruct.qualifiedName);

    try {
      int maxSec = Integer.parseInt(DecompilerContext.getProperty(IFernflowerPreferences.MAX_PROCESSING_METHOD).toString());
      boolean testMode = DecompilerContext.getOption(IFernflowerPreferences.UNIT_TEST_MODE);
      VBStyleCollection<StructMethod, String> classMethods = classStruct.getMethods();

      if (shouldProcessMethodsInParallel(classMethods, maxSec, testMode)) {
        processMethodsInParallel(classMethods, spec, maxSec, testMode);
      }
      else {
        for (StructMethod mt : classMethods) {
          addMethod(processMethod(mt, spec, maxSec, testMode));
        }
      }
    }
    finally {
      DecompilerContext.getLogger().endClass();
    }
  }

  private static boolean shouldProcessMethodsInParallel(List<StructMethod> classMethods, int maxSec, boolean testMode) {
    ForkJoinPool pool = ForkJoinTask.getPool();
    if (!DecompilerContext.getOption(IFernflowerPreferences.PARALLEL_METHODS) ||
        DEBUG_METHOD_FILTER != null ||
        pool == null ||
        pool.getParallelism() <= 1) {
      return false;
    }

    // The timeout path already delegates each method to a dedicated thread so
    // that it can be stopped. Combining it with method work stealing would
    // oversubscribe the configured decompiler pool and make timings erratic.
    if (maxSec != 0 && !testMode) {
      return false;
    }

    int codeMethods = 0;
    for (StructMethod method : classMethods) {
      if (method.containsCode() && ++codeMethods >= MIN_PARALLEL_CODE_METHODS) {
        return true;
      }
    }

    return false;
  }

  private void processMethodsInParallel(List<StructMethod> classMethods, LanguageSpec spec, int maxSec, boolean testMode) {
    DecompilerContext parentContext = DecompilerContext.getCurrentContext();
    MethodWrapper[] results = new MethodWrapper[classMethods.size()];
    List<ForkJoinTask<?>> tasks = new ArrayList<>(classMethods.size());

    try {
      for (int i = 0; i < classMethods.size(); i++) {
        StructMethod mt = classMethods.get(i);
        if (mt.containsCode()) {
          tasks.add(forkMethodProcessing(parentContext, results, i, mt, spec, maxSec, testMode));
        }
      }

      for (int i = 0; i < classMethods.size(); i++) {
        StructMethod mt = classMethods.get(i);
        if (!mt.containsCode()) {
          results[i] = processMethod(mt, spec, maxSec, testMode);
        }
      }

      for (ForkJoinTask<?> task : tasks) {
        task.join();
      }
    }
    catch (Throwable failure) {
      cancelAndDrainTasks(tasks);
      throw asRuntimeException(failure);
    }

    for (MethodWrapper result : results) {
      addMethod(result);
    }
  }

  private ForkJoinTask<?> forkMethodProcessing(
    DecompilerContext parentContext,
    MethodWrapper[] results,
    int index,
    StructMethod mt,
    LanguageSpec spec,
    int maxSec,
    boolean testMode
  ) {
    DecompilerContext methodContext = parentContext.copyForMethodProcessing();
    ForkJoinTask<?> task = ForkJoinTask.adapt(() -> {
      DecompilerContext previousContext = DecompilerContext.getCurrentContext();
      DecompilerContext.setCurrentContext(methodContext);
      try {
        DecompilerContext.setProperty(DecompilerContext.CURRENT_CLASS, classStruct);
        DecompilerContext.setProperty(DecompilerContext.CURRENT_CLASS_WRAPPER, this);
        results[index] = processMethod(mt, spec, maxSec, testMode);
      }
      finally {
        DecompilerContext.setCurrentContext(previousContext);
      }
    });
    task.fork();
    return task;
  }

  private static void cancelAndDrainTasks(List<? extends ForkJoinTask<?>> tasks) {
    for (ForkJoinTask<?> task : tasks) {
      if (!task.isDone()) {
        task.cancel(true);
      }
    }
    for (ForkJoinTask<?> task : tasks) {
      task.quietlyJoin();
    }
  }

  private static RuntimeException asRuntimeException(Throwable failure) {
    if (failure instanceof RuntimeException) {
      return (RuntimeException)failure;
    }
    if (failure instanceof Error) {
      throw (Error)failure;
    }
    return new RuntimeException(failure);
  }

  private void addMethod(MethodWrapper methodWrapper) {
    methods.addWithKey(methodWrapper, InterpreterUtil.makeUniqueKey(methodWrapper.methodStruct.getName(), methodWrapper.methodStruct.getDescriptor()));
  }

  private MethodWrapper processMethod(StructMethod mt, LanguageSpec spec, int maxSec, boolean testMode) {
    DecompilerContext.getLogger().startMethod(mt.getName() + " " + mt.getDescriptor());

    try {
      MethodDescriptor md = MethodDescriptor.parseDescriptor(mt, null);
      VarProcessor varProc = new VarProcessor(mt, md);
      DecompilerContext.startMethod(varProc);

      VarNamesCollector vc = varProc.getVarNamesCollector();
      CounterContainer counter = DecompilerContext.getCounterContainer();

      RootStatement root = null;

      Throwable error = null;

      if (DEBUG_METHOD_FILTER != null && !DEBUG_METHOD_FILTER.equals(mt.getName())) {
        return new MethodWrapper(null, varProc, mt, classStruct, counter);
      }

      try {
        if (mt.containsCode()) {
          if (maxSec == 0 || testMode) {
            root = MethodProcessor.codeToJava(classStruct, mt, md, varProc, spec);
          }
          else {
            MethodProcessor mtProc = new MethodProcessor(classStruct, mt, md, varProc, spec, DecompilerContext.getCurrentContext());

            Thread mtThread = new Thread(mtProc, "Java decompiler");
            long stopAt = System.currentTimeMillis() + maxSec * 1000L;

            mtThread.start();

            while (!mtProc.isFinished()) {
              try {
                synchronized (mtProc.lock) {
                  mtProc.lock.wait(200);
                }
              }
              catch (InterruptedException e) {
                killThread(mtThread);
                throw e;
              }

              if (System.currentTimeMillis() >= stopAt) {
                String message = "Processing time limit exceeded for method " + mt.getName() + ", execution interrupted.";
                DecompilerContext.getLogger().writeMessage(message, IFernflowerLogger.Severity.ERROR);
                killThread(mtThread);
                error = new TimeoutException();
                break;
              }
            }

            if (error == null) {
              root = mtProc.getResult();
            }
          }
        }
        else {
          boolean thisVar = !mt.hasModifier(CodeConstants.ACC_STATIC);

          int paramCount = 0;
          if (thisVar) {
            varProc.getThisVars().put(new VarVersionPair(0, 0), classStruct.qualifiedName);
            paramCount = 1;
          }
          paramCount += md.params.length;

          int varIndex = 0;
          for (int i = 0; i < paramCount; i++) {
            varProc.setVarName(new VarVersionPair(varIndex, 0), vc.getFreeName(varIndex));
            varProc.markParam(new VarVersionPair(varIndex, 0));

            if (thisVar) {
              if (i == 0) {
                varIndex++;
              }
              else {
                varIndex += md.params[i - 1].stackSize;
              }
            }
            else {
              varIndex += md.params[i].stackSize;
            }
          }
        }
      }
      catch (CancelationManager.CanceledException e) {
        throw e;
      }
      catch (Throwable t) {
        String message = "Method " + mt.getName() + " " + mt.getDescriptor() + " in class " + classStruct.qualifiedName + " couldn't be decompiled.";
        DecompilerContext.getLogger().writeMessage(message, IFernflowerLogger.Severity.WARN, t);
        error = t;
        RootStatement rootStat = MethodProcessor.debugCurrentlyDecompiling.get();
        if (rootStat != null) {
          DotExporter.errorToDotFile(rootStat, mt, "fail");

          try {
            DotExporter.errorToDotFile(new FlattenStatementsHelper().buildDirectGraph(rootStat), mt, "failDGraph");
          } catch (Exception ignored) {
          }
        }

        ControlFlowGraph graph = MethodProcessor.debugCurrentCFG.get();
        if (graph != null) {
          DotExporter.errorToDotFile(graph, mt, "failCFG");
        }

        DecompileRecord decompileRecord = MethodProcessor.debugCurrentDecompileRecord.get();
        if (decompileRecord != null) {
          DotExporter.toDotFile(decompileRecord, mt, "failRecord", true);
        }
      }

      MethodWrapper methodWrapper = new MethodWrapper(root, varProc, mt, classStruct, counter);
      methodWrapper.decompileError = error;

      if (error == null) {
        // if debug information present and should be used
        if (DecompilerContext.getOption(IFernflowerPreferences.USE_DEBUG_VAR_NAMES)) {
          StructLocalVariableTableAttribute attr = mt.getLocalVariableAttr();
          if (attr != null) {
            // only param names here
            varProc.setDebugVarNames(root, attr.getMapNames());

            /*
            // the rest is here
            methodWrapper.getOrBuildGraph().iterateExprents(exprent -> {
              List<Exprent> lst = exprent.getAllExprents(true);
              lst.add(exprent);
              lst.stream()
                .filter(e -> e instanceof VarExprent)
                .forEach(e -> {
                  VarExprent varExprent = (VarExprent)e;
                  String name = varExprent.getDebugName(mt);
                  if (name != null) {
                    varProc.setVarName(varExprent.getVarVersionPair(), name);
                  }
                });
              return 0;
            });
            */
          }
        }
      }

      return methodWrapper;
    }
    finally {
      DecompilerContext.getLogger().endMethod();
    }
  }

  @SuppressWarnings("deprecation")
  private static void killThread(Thread thread) {
    thread.stop();
  }

  public MethodWrapper getMethodWrapper(String name, String descriptor) {
    return methods.getWithKey(InterpreterUtil.makeUniqueKey(name, descriptor));
  }

  public MethodWrapper getMethodWrapper(int index) {
    return methods.get(index);
  }

  public StructClass getClassStruct() {
    return classStruct;
  }

  public VBStyleCollection<MethodWrapper, String> getMethods() {
    return methods;
  }

  public Set<String> getHiddenMembers() {
    return hiddenMembers;
  }

  public void hideMember(String key) {
    hiddenMembers.add(key);
  }

  public List<SourceOnlyMethod> getSourceOnlyMethods() {
    return sourceOnlyMethods;
  }

  public void addSourceOnlyMethod(SourceOnlyMethod method) {
    String key = InterpreterUtil.makeUniqueKey(method.name(), method.descriptorString());
    if (sourceOnlyMethodsByKey.putIfAbsent(key, method) != null) {
      throw new IllegalStateException("Duplicate source-only method: " + key);
    }
    sourceOnlyMethods.add(method);
  }

  public SourceOnlyMethod getSourceOnlyMethod(String name, String descriptor) {
    return sourceOnlyMethodsByKey.get(InterpreterUtil.makeUniqueKey(name, descriptor));
  }

  public List<SourceOnlyClass> getSourceOnlyClasses() {
    return Collections.unmodifiableList(sourceOnlyClasses);
  }

  public SourceOnlyClass getOrCreateSourceOnlyClass(String namePrefix, int accessFlags) {
    for (SourceOnlyClass sourceOnlyClass : sourceOnlyClasses) {
      if (sourceOnlyClass.name().startsWith(namePrefix)) {
        if (sourceOnlyClass.accessFlags() != accessFlags) {
          throw new IllegalStateException("Source-only class requested with inconsistent access flags: " + namePrefix);
        }
        return sourceOnlyClass;
      }
    }

    String name = namePrefix;
    int suffix = 0;
    while (DecompilerContext.getStructContext().getClass(classStruct.qualifiedName + "$" + name) != null
      || hasSourceOnlyClass(name)) {
      name = namePrefix + ++suffix;
    }
    SourceOnlyClass created = new SourceOnlyClass(name, accessFlags);
    sourceOnlyClasses.add(created);
    return created;
  }

  private boolean hasSourceOnlyClass(String name) {
    for (SourceOnlyClass sourceOnlyClass : sourceOnlyClasses) {
      if (sourceOnlyClass.name().equals(name)) {
        return true;
      }
    }
    return false;
  }

  public List<MissingAbstractMethod> getMissingAbstractMethods() {
    return missingAbstractMethods;
  }

  public void addMissingAbstractMethod(MissingAbstractMethod method) {
    if (missingAbstractMethodKeys.add(InterpreterUtil.makeUniqueKey(method.name(), method.descriptorString()))) {
      missingAbstractMethods.add(method);
    }
  }

  public Set<String> getRequiredSourceMethodKeys() {
    return requiredSourceMethodKeys;
  }

  public void requireMethodInSource(String methodKey) {
    requiredSourceMethodKeys.add(methodKey);
  }

  public Set<String> getAbstractMethodFallbackKeys() {
    return abstractMethodFallbackKeys;
  }

  public void addAbstractMethodFallback(String methodKey) {
    abstractMethodFallbackKeys.add(methodKey);
    requiredSourceMethodKeys.add(methodKey);
  }

  public String nextSourceOnlyMethodName(String prefix) {
    Set<String> usedNames = new HashSet<>();
    for (StructMethod method : classStruct.getMethods()) {
      usedNames.add(method.getName());
    }
    for (SourceOnlyMethod method : sourceOnlyMethods) {
      usedNames.add(method.name());
    }

    String name;
    do {
      name = prefix + sourceOnlyMethodCounter++;
    }
    while (usedNames.contains(name));

    return name;
  }

  public VBStyleCollection<Exprent, String> getStaticFieldInitializers() {
    return staticFieldInitializers;
  }

  public VBStyleCollection<Exprent, String> getDynamicFieldInitializers() {
    return dynamicFieldInitializers;
  }

  @Override
  public String toString() {
    return classStruct.qualifiedName;
  }

  public record SourceOnlyMethod(
    String name,
    VarType returnType,
    List<SourceOnlyParameter> parameters,
    List<String> thrownExceptions,
    List<Statement> bodyStatements,
    MethodWrapper owner
  ) {
    public SourceOnlyMethod {
      parameters = Collections.unmodifiableList(new ArrayList<>(parameters));
      thrownExceptions = List.copyOf(thrownExceptions);
      bodyStatements = Collections.unmodifiableList(new ArrayList<>(bodyStatements));
    }

    public String descriptorString() {
      StringBuilder descriptor = new StringBuilder("(");
      for (SourceOnlyParameter parameter : parameters) {
        descriptor.append(parameter.type());
      }
      return descriptor.append(')').append(returnType).toString();
    }
  }

  public static final class SourceOnlyClass {
    private final String name;
    private final int accessFlags;
    private final List<SourceOnlyMethod> methods = new ArrayList<>();

    private SourceOnlyClass(String name, int accessFlags) {
      this.name = name;
      this.accessFlags = accessFlags;
    }

    public String name() {
      return name;
    }

    public int accessFlags() {
      return accessFlags;
    }

    public List<SourceOnlyMethod> methods() {
      return Collections.unmodifiableList(methods);
    }

    public void addMethod(SourceOnlyMethod method) {
      methods.add(method);
    }
  }

  public record MissingAbstractMethod(
    String name,
    String descriptorString,
    int accessFlags,
    VarType returnType,
    List<VarType> parameterTypes
  ) {
    public MissingAbstractMethod {
      parameterTypes = Collections.unmodifiableList(new ArrayList<>(parameterTypes));
    }
  }

  public record SourceOnlyParameter(VarType type, String name, VarExprent exprent) {}

}
