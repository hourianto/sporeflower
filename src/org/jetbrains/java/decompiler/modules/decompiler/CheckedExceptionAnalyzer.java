// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.rels.ClassWrapper;
import org.jetbrains.java.decompiler.main.rels.MethodWrapper;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.modules.decompiler.exps.ExitExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.Exprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.InvocationExprent;
import org.jetbrains.java.decompiler.modules.decompiler.exps.NewExprent;
import org.jetbrains.java.decompiler.modules.decompiler.stats.Statement;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.InterpreterUtil;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class CheckedExceptionAnalyzer {
  private static final ThreadLocal<CheckedExceptionAnalyzer> ACTIVE = new ThreadLocal<>();
  private static final String[] FALLBACK_CATCH_TYPES = {
    "java/lang/RuntimeException",
    "java/lang/Error",
    "java/lang/Exception",
    "java/lang/Throwable"
  };

  private final Map<String, List<String>> inferredCheckedExceptions = new HashMap<>();
  private final Set<String> inferenceStack = new HashSet<>();
  private final CheckedInvocationResolver invocationResolver = new CheckedInvocationResolver(this::inferMissingCheckedExceptions);

  public static @Nullable CheckedExceptionAnalyzer active() {
    return ACTIVE.get();
  }

  public static Scope activate(CheckedExceptionAnalyzer analyzer) {
    return new Scope(analyzer);
  }

  public CatchRewrite rewriteCatchTypes(
    Statement tryBody,
    List<String> exceptionTypes,
    List<String> previousCatchTypes,
    List<String> followingCatchTypes
  ) {
    List<String> renderedTypes = new ArrayList<>();
    List<String> removedCheckedTypes = new ArrayList<>();

    for (String exceptionType : exceptionTypes) {
      if (CheckedExceptionSupport.isCheckedExceptionType(exceptionType) && !canStatementThrow(tryBody, exceptionType)) {
        removedCheckedTypes.add(exceptionType);
      } else {
        renderedTypes.add(exceptionType);
      }
    }

    if (removedCheckedTypes.isEmpty()) {
      return new CatchRewrite(new ArrayList<>(exceptionTypes), Collections.emptyList(), false, false);
    }

    if (!renderedTypes.isEmpty()) {
      return new CatchRewrite(renderedTypes, removedCheckedTypes, true, false);
    }

    String fallback = selectFallbackCatchType(previousCatchTypes, followingCatchTypes);
    if (fallback != null) {
      return new CatchRewrite(Collections.singletonList(fallback), removedCheckedTypes, true, true);
    }

    // No safe fallback left (all would be shadowed). Keep original to avoid introducing an unreachable duplicate catch.
    return new CatchRewrite(new ArrayList<>(exceptionTypes), removedCheckedTypes, false, false);
  }

  public CatchRewrite rewriteCatchTypes(Statement tryBody, List<String> exceptionTypes, List<String> previousCatchTypes) {
    return rewriteCatchTypes(tryBody, exceptionTypes, previousCatchTypes, Collections.emptyList());
  }

  public List<String> inferMissingCheckedExceptions(StructClass ownerClass, ClassWrapper ownerWrapper, StructMethod method, MethodWrapper methodWrapper) {
    String methodKey = buildMethodKey(ownerClass, method);
    List<String> cached = inferredCheckedExceptions.get(methodKey);
    if (cached != null) {
      return cached;
    }
    if (!inferenceStack.add(methodKey)) {
      return Collections.emptyList();
    }

    try {
      if (methodWrapper == null || methodWrapper.root == null) {
        inferredCheckedExceptions.put(methodKey, Collections.emptyList());
        return Collections.emptyList();
      }

      LinkedHashSet<String> escaping = new LinkedHashSet<>();
      LinkedHashSet<String> callsiteCaughtTypes = collectSameClassCallsiteCaughtTypes(ownerClass, ownerWrapper, method);
      collectEscapingCheckedExceptions(methodWrapper.root.getFirst(), ownerClass, ownerWrapper, Collections.emptyList(), escaping);
      augmentInferredExceptionsFromSameClassCallSites(method, methodWrapper, callsiteCaughtTypes, escaping);
      augmentInferredExceptionsFromOverriddenDeclarationsByCallsiteCatches(
        ownerClass,
        ownerWrapper,
        method,
        callsiteCaughtTypes,
        escaping
      );
      List<String> inferred = new ArrayList<>(escaping);
      inferred = filterInferredExceptionsByOverrideCompatibility(ownerClass, ownerWrapper, method, inferred);
      inferredCheckedExceptions.put(methodKey, inferred);
      return inferred;
    }
    finally {
      inferenceStack.remove(methodKey);
    }
  }

  public boolean canStatementThrow(Statement statement, String exceptionType) {
    return CheckedStatementWalker.walk(
      statement,
      Collections.emptyList(),
      CheckedStatementWalker.CatchAllPolicy.CATCH_ALL_CATCHES_THROWABLE,
      (exprents, activeCatchTypes) -> canExprentsThrow(exprents, exceptionType, activeCatchTypes)
    );
  }

  private boolean canExprentsThrow(List<Exprent> exprents, String exceptionType, List<String> activeCatchTypes) {
    if (exprents == null || exprents.isEmpty()) {
      return false;
    }

    StructClass currentClass = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS);
    ClassWrapper currentWrapper = DecompilerContext.getContextProperty(DecompilerContext.CURRENT_CLASS_WRAPPER);

    for (Exprent exprent : exprents) {
      for (Exprent nested : exprent.getAllExprents(true, true)) {
        if (nested instanceof InvocationExprent invocation) {
          if (invocationResolver.invocationThrows(invocation, exceptionType, currentClass, currentWrapper)
            && !CheckedExceptionSupport.isCaughtByActiveCatches(exceptionType, activeCatchTypes)) {
            return true;
          }
        }
        else if (nested instanceof NewExprent newExprent && newExprent.getConstructor() != null) {
          // NewExprent does not expose its constructor invocation from getAllExprents(),
          // so account for checked constructor throws explicitly.
          InvocationExprent ctor = newExprent.getConstructor();
          if (invocationResolver.invocationThrows(ctor, exceptionType, currentClass, currentWrapper)
            && !CheckedExceptionSupport.isCaughtByActiveCatches(exceptionType, activeCatchTypes)) {
            return true;
          }
        }
        else if (nested instanceof ExitExprent exit && exit.getExitType() == ExitExprent.Type.THROW) {
          Exprent value = exit.getValue();
          if (value != null) {
            VarType thrownType = value.getExprType();
            if (thrownType.type == CodeType.OBJECT
              && thrownType.value != null
              && CheckedExceptionSupport.isSubtypeOf(thrownType.value, exceptionType)) {
              if (!CheckedExceptionSupport.isCaughtByActiveCatches(thrownType.value, activeCatchTypes)) {
                return true;
              }
            }
          }
        }
      }
    }
    return false;
  }

  private void collectEscapingCheckedExceptions(
    Statement statement,
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    List<String> activeCatchTypes,
    LinkedHashSet<String> escapingExceptions
  ) {
    CheckedStatementWalker.walk(
      statement,
      activeCatchTypes,
      CheckedStatementWalker.CatchAllPolicy.CATCH_ALL_CATCHES_THROWABLE,
      (exprents, currentCatchTypes) -> {
        collectEscapingFromExprents(exprents, ownerClass, ownerWrapper, currentCatchTypes, escapingExceptions);
        return false;
      }
    );
  }

  private void collectEscapingFromExprents(
    List<Exprent> exprents,
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    List<String> activeCatchTypes,
    LinkedHashSet<String> escapingExceptions
  ) {
    if (exprents == null || exprents.isEmpty()) {
      return;
    }

    for (Exprent exprent : exprents) {
      List<Exprent> nestedExprents = exprent.getAllExprents(true, true);
      for (Exprent nested : nestedExprents) {
        if (nested instanceof InvocationExprent invocation) {
          for (String thrownException : invocationResolver.getInvocationCheckedExceptions(invocation, ownerClass, ownerWrapper)) {
            if (!CheckedExceptionSupport.isCaughtByActiveCatches(thrownException, activeCatchTypes)) {
              escapingExceptions.add(thrownException);
            }
          }
        }
        else if (nested instanceof NewExprent newExprent && newExprent.getConstructor() != null) {
          for (String thrownException : invocationResolver.getInvocationCheckedExceptions(newExprent.getConstructor(), ownerClass, ownerWrapper)) {
            if (!CheckedExceptionSupport.isCaughtByActiveCatches(thrownException, activeCatchTypes)) {
              escapingExceptions.add(thrownException);
            }
          }
        }
        else if (nested instanceof ExitExprent exitExprent && exitExprent.getExitType() == ExitExprent.Type.THROW) {
          Exprent throwValue = exitExprent.getValue();
          if (throwValue != null) {
            VarType thrownType = throwValue.getExprType();
            if (thrownType.type == CodeType.OBJECT
              && thrownType.value != null
              && CheckedExceptionSupport.isCheckedExceptionType(thrownType.value)) {
              if (!CheckedExceptionSupport.isCaughtByActiveCatches(thrownType.value, activeCatchTypes)) {
                escapingExceptions.add(thrownType.value);
              }
            }
          }
        }
      }
    }
  }

  private void augmentInferredExceptionsFromSameClassCallSites(
    StructMethod targetMethod,
    MethodWrapper targetWrapper,
    Set<String> callsiteCaughtTypes,
    LinkedHashSet<String> escapingExceptions
  ) {
    // Restrict callsite-based augmentation to private methods to avoid widening
    // externally visible or overriding signatures.
    if (!targetMethod.hasModifier(CodeConstants.ACC_PRIVATE)) {
      return;
    }

    if (targetWrapper.root == null || callsiteCaughtTypes.isEmpty()) {
      return;
    }

    for (String catchType : callsiteCaughtTypes) {
      if (!CheckedExceptionSupport.isCheckedExceptionType(catchType)) {
        continue;
      }
      // Broad catch-all types are too imprecise for signature synthesis and tend to
      // create cascading false positives on unresolved/library invocations.
      if (isBroadCheckedCatchType(catchType)) {
        continue;
      }
      if (canStatementThrow(targetWrapper.root.getFirst(), catchType)) {
        escapingExceptions.add(catchType);
      }
    }
  }

  private static boolean isBroadCheckedCatchType(String catchType) {
    return "java/lang/Exception".equals(catchType) || "java/lang/Throwable".equals(catchType);
  }

  private void augmentInferredExceptionsFromOverriddenDeclarationsByCallsiteCatches(
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    StructMethod method,
    Set<String> callsiteCaughtTypes,
    LinkedHashSet<String> escapingExceptions
  ) {
    if (CodeConstants.INIT_NAME.equals(method.getName())
      || method.hasModifier(CodeConstants.ACC_PRIVATE)
      || method.hasModifier(CodeConstants.ACC_STATIC)) {
      return;
    }

    if (callsiteCaughtTypes.isEmpty()) {
      return;
    }

    LinkedHashSet<String> declaredByOverrides = new LinkedHashSet<>();
    for (List<String> declared : collectOverriddenMethodCheckedThrows(ownerClass, ownerWrapper, method)) {
      declaredByOverrides.addAll(declared);
    }
    if (declaredByOverrides.isEmpty()) {
      return;
    }

    // Only seed inherited declarations that are actually observed in precise
    // same-class checked catches around invocations of this method.
    for (String catchType : callsiteCaughtTypes) {
      if (!CheckedExceptionSupport.isCheckedExceptionType(catchType) || isBroadCheckedCatchType(catchType)) {
        continue;
      }
      for (String declared : declaredByOverrides) {
        if (CheckedExceptionSupport.isSubtypeOf(declared, catchType)) {
          escapingExceptions.add(declared);
        }
      }
    }
  }

  private LinkedHashSet<String> collectSameClassCallsiteCaughtTypes(
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    StructMethod targetMethod
  ) {
    LinkedHashSet<String> callsiteCaughtTypes = new LinkedHashSet<>();
    for (MethodWrapper callerWrapper : ownerWrapper.getMethods()) {
      if (callerWrapper.root == null) {
        continue;
      }
      CheckedStatementWalker.walk(
        callerWrapper.root.getFirst(),
        Collections.emptyList(),
        CheckedStatementWalker.CatchAllPolicy.CATCH_ALL_IGNORED,
        (exprents, activeCatchTypes) -> {
          collectCallsiteCaughtTypesFromExprents(exprents, ownerClass, targetMethod, activeCatchTypes, callsiteCaughtTypes);
          return false;
        }
      );
    }
    return callsiteCaughtTypes;
  }

  private static void collectCallsiteCaughtTypesFromExprents(
    List<Exprent> exprents,
    StructClass ownerClass,
    StructMethod targetMethod,
    List<String> activeCatchTypes,
    LinkedHashSet<String> caughtTypes
  ) {
    if (exprents == null || exprents.isEmpty()) {
      return;
    }

    for (Exprent exprent : exprents) {
      for (Exprent nested : exprent.getAllExprents(true, true)) {
        if (nested instanceof InvocationExprent invocation && isSameClassInvocation(invocation, ownerClass, targetMethod)) {
          for (String catchType : activeCatchTypes) {
            caughtTypes.add(catchType);
          }
        }
        else if (nested instanceof NewExprent newExprent
          && newExprent.getConstructor() != null
          && isSameClassInvocation(newExprent.getConstructor(), ownerClass, targetMethod)) {
          for (String catchType : activeCatchTypes) {
            caughtTypes.add(catchType);
          }
        }
      }
    }
  }

  private static boolean isSameClassInvocation(InvocationExprent invocation, StructClass ownerClass, StructMethod targetMethod) {
    if (!targetMethod.getName().equals(invocation.getName())
      || !targetMethod.getDescriptor().equals(invocation.getStringDescriptor())) {
      return false;
    }

    String invocationClass = invocation.getClassname();
    return invocationClass != null && invocationClass.equals(ownerClass.qualifiedName);
  }

  private static String selectFallbackCatchType(List<String> previousCatchTypes, List<String> followingCatchTypes) {
    for (String candidate : FALLBACK_CATCH_TYPES) {
      // Keep fallback catches compilable in both directions:
      // avoid types shadowed by earlier handlers and avoid types shadowing later handlers.
      if (!isShadowedByPreviousCatch(candidate, previousCatchTypes)
        && !shadowsFollowingCatch(candidate, followingCatchTypes)) {
        return candidate;
      }
    }
    return null;
  }

  private static boolean isShadowedByPreviousCatch(String candidateType, List<String> previousCatchTypes) {
    for (String previousType : previousCatchTypes) {
      if (candidateType.equals(previousType) || CheckedExceptionSupport.isSubtypeOf(candidateType, previousType)) {
        return true;
      }
    }
    return false;
  }

  private static boolean shadowsFollowingCatch(String candidateType, List<String> followingCatchTypes) {
    for (String followingType : followingCatchTypes) {
      if (candidateType.equals(followingType) || CheckedExceptionSupport.isSubtypeOf(followingType, candidateType)) {
        return true;
      }
    }
    return false;
  }

  private List<String> filterInferredExceptionsByOverrideCompatibility(
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    StructMethod method,
    List<String> inferredExceptions
  ) {
    // Never synthesize throws that would make an override signature illegal to compile.
    // If a super declaration has no checked throws, inferred checked throws are dropped.
    if (inferredExceptions.isEmpty()) {
      return inferredExceptions;
    }
    if (CodeConstants.INIT_NAME.equals(method.getName())
      || method.hasModifier(CodeConstants.ACC_PRIVATE)
      || method.hasModifier(CodeConstants.ACC_STATIC)) {
      return inferredExceptions;
    }

    List<List<String>> overriddenThrows = collectOverriddenMethodCheckedThrows(ownerClass, ownerWrapper, method);
    if (overriddenThrows.isEmpty()) {
      return inferredExceptions;
    }

    List<String> compatible = new ArrayList<>();
    for (String inferred : inferredExceptions) {
      boolean allowed = true;
      for (List<String> declared : overriddenThrows) {
        if (declared.isEmpty()) {
          allowed = false;
          break;
        }

        boolean coveredByDeclaration = false;
        for (String declaredType : declared) {
          if (CheckedExceptionSupport.isSubtypeOf(inferred, declaredType)) {
            coveredByDeclaration = true;
            break;
          }
        }
        if (!coveredByDeclaration) {
          allowed = false;
          break;
        }
      }

      if (allowed) {
        compatible.add(inferred);
      }
    }

    return compatible;
  }

  private List<List<String>> collectOverriddenMethodCheckedThrows(StructClass ownerClass, ClassWrapper ownerWrapper, StructMethod method) {
    List<List<String>> declarations = new ArrayList<>();
    String originalName = getOriginalMethodName(ownerClass, method);
    String descriptor = method.getDescriptor();
    Set<String> visited = new HashSet<>();

    if (ownerClass.superClass != null) {
      collectOverriddenMethodCheckedThrows(
        ownerClass.superClass.getString(),
        ownerClass,
        ownerWrapper,
        originalName,
        descriptor,
        visited,
        declarations
      );
    }
    for (String ifaceName : ownerClass.getInterfaceNames()) {
      collectOverriddenMethodCheckedThrows(
        ifaceName,
        ownerClass,
        ownerWrapper,
        originalName,
        descriptor,
        visited,
        declarations
      );
    }

    return declarations;
  }

  private void collectOverriddenMethodCheckedThrows(
    String className,
    StructClass ownerClass,
    ClassWrapper ownerWrapper,
    String originalMethodName,
    String descriptor,
    Set<String> visited,
    List<List<String>> declarations
  ) {
    if (className == null || !visited.add(className)) {
      return;
    }

    StructClass cls = DecompilerContext.getStructContext().getClass(className);
    if (cls == null) {
      if (className.startsWith("java/")) {
        List<String> reflected = CheckedInvocationResolver.reflectionMethodCheckedExceptions(className, originalMethodName, descriptor);
        if (reflected != null) {
          declarations.add(reflected);
        }
      }
      return;
    }

    StructMethod declaredMethod = findMethodByOriginalName(cls, originalMethodName, descriptor);
    if (declaredMethod != null) {
      List<String> declared = CheckedInvocationResolver.getDeclaredCheckedExceptions(cls, declaredMethod);
      if (declared.isEmpty() && cls.isOwn() && declaredMethod.containsCode()) {
        ClassWrapper resolvedWrapper = invocationResolver.resolveClassWrapper(cls, ownerClass, ownerWrapper);
        MethodWrapper wrapper = resolvedWrapper == null
          ? null
          : resolvedWrapper.getMethodWrapper(declaredMethod.getName(), declaredMethod.getDescriptor());
        if (wrapper != null) {
          declared = inferMissingCheckedExceptions(cls, resolvedWrapper, declaredMethod, wrapper);
        }
      }
      declarations.add(declared);
    }

    if (cls.superClass != null) {
      collectOverriddenMethodCheckedThrows(
        cls.superClass.getString(),
        ownerClass,
        ownerWrapper,
        originalMethodName,
        descriptor,
        visited,
        declarations
      );
    }
    for (String ifaceName : cls.getInterfaceNames()) {
      collectOverriddenMethodCheckedThrows(
        ifaceName,
        ownerClass,
        ownerWrapper,
        originalMethodName,
        descriptor,
        visited,
        declarations
      );
    }
  }

  private static @Nullable StructMethod findMethodByOriginalName(StructClass ownerClass, String originalMethodName, String descriptor) {
    for (StructMethod method : ownerClass.getMethods()) {
      if (!descriptor.equals(method.getDescriptor())) {
        continue;
      }
      if (originalMethodName.equals(getOriginalMethodName(ownerClass, method))) {
        return method;
      }
    }
    return null;
  }

  private static String getOriginalMethodName(StructClass ownerClass, StructMethod method) {
    PoolInterceptor interceptor = DecompilerContext.getPoolInterceptor();
    if (interceptor == null) {
      return method.getName();
    }

    String oldName = interceptor.getOldName(ownerClass.qualifiedName + " " + method.getName() + " " + method.getDescriptor());
    if (oldName == null) {
      return method.getName();
    }

    int split = oldName.indexOf(' ');
    return split <= 0 ? oldName : oldName.substring(0, split);
  }

  private static String buildMethodKey(StructClass ownerClass, StructMethod method) {
    return ownerClass.qualifiedName + " " + InterpreterUtil.makeUniqueKey(method.getName(), method.getDescriptor());
  }

  public static final class CatchRewrite {
    private final List<String> renderedTypes;
    private final List<String> removedCheckedTypes;
    private final boolean rewritten;
    private final boolean fallbackUsed;

    private CatchRewrite(List<String> renderedTypes, List<String> removedCheckedTypes, boolean rewritten, boolean fallbackUsed) {
      this.renderedTypes = renderedTypes;
      this.removedCheckedTypes = removedCheckedTypes;
      this.rewritten = rewritten;
      this.fallbackUsed = fallbackUsed;
    }

    public List<String> getRenderedTypes() {
      return renderedTypes;
    }

    public List<String> getRemovedCheckedTypes() {
      return removedCheckedTypes;
    }

    public boolean isRewritten() {
      return rewritten;
    }

    public boolean isFallbackUsed() {
      return fallbackUsed;
    }
  }

  public static final class Scope implements AutoCloseable {
    private final @Nullable CheckedExceptionAnalyzer previous;
    private boolean closed;

    private Scope(CheckedExceptionAnalyzer analyzer) {
      this.previous = ACTIVE.get();
      ACTIVE.set(analyzer);
    }

    @Override
    public void close() {
      if (closed) {
        return;
      }
      closed = true;
      if (previous == null) {
        ACTIVE.remove();
      }
      else {
        ACTIVE.set(previous);
      }
    }
  }
}
