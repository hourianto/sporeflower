// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.extern.IFernflowerPreferences;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.main.rels.SourceMethodSemantics;
import org.jetbrains.java.decompiler.main.rels.SourceFieldScope;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructField;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.consts.ConstantPool;
import org.jetbrains.java.decompiler.struct.consts.LinkConstant;
import org.jetbrains.java.decompiler.struct.consts.PooledConstant;
import org.jetbrains.java.decompiler.struct.consts.PrimitiveConstant;
import org.jetbrains.java.decompiler.struct.gen.CodeType;
import org.jetbrains.java.decompiler.struct.gen.FieldDescriptor;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.NewClassNameBuilder;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.collections.VBStyleCollection;

import java.io.IOException;
import java.util.*;
import java.util.stream.Collectors;

public class IdentifierConverter implements NewClassNameBuilder {
  private final StructContext context;
  private final IIdentifierRenamer helper;
  private final ConverterHelper conflictFallbackRenamer = new ConverterHelper();
  private final PoolInterceptor interceptor;
  private List<ClassWrapperNode> rootClasses = new ArrayList<>();
  private List<ClassWrapperNode> rootInterfaces = new ArrayList<>();
  private Map<String, String> overrideMethodRenameHints = new LinkedHashMap<>();
  private final Map<StructMethod, StructMethod> covariantBridges = new LinkedHashMap<>();
  private final Map<StructMethod, StructMethod> bridgesByTarget = new IdentityHashMap<>();
  private Set<String> reservedClassNames = Set.of();
  private static final String MULTI_PACKAGE_DEFAULT_RELOCATION = "decompiled/defaultpkg";
  private final Map<String, String> innerNames = new LinkedHashMap<>();
  private final Map<String, String> forcedPackageRelocations = new HashMap<>();

  public IdentifierConverter(StructContext context, IIdentifierRenamer helper, PoolInterceptor interceptor) {
    this.context = context;
    this.helper = helper;
    this.interceptor = interceptor;
  }

  public NamingPlan rename(NamingPlan prepared) {
    try {
      reservedClassNames = new HashSet<>(Arrays.asList(((String)DecompilerContext.getProperty(
        IFernflowerPreferences.RESERVED_CLASS_NAMES)).split(",")));
      reservedClassNames = reservedClassNames.stream().map(String::trim).filter(name -> !name.isEmpty())
        .collect(Collectors.toUnmodifiableSet());
      interceptor.bindMemberReferences(context);
      if (prepared != null) {
        if (!(helper instanceof Tiny2IdentifierRenamer tiny)) {
          throw new IllegalArgumentException("Prepared names require a complete Tiny mapping");
        }
        NamingPlan plan = prepared;
        NamingPlanApplication.apply(plan, context, interceptor);
        tiny.bindParameterNames(context, interceptor);
        retainRenamedBridges();
        context.reloadContext();
        return plan;
      }
      buildInheritanceTree();
      collectForcedPackageRelocations();
      renameAllClasses();
      renameMemberClasses();
      collectOverrideMethodRenameHints();
      renameMembers(rootInterfaces);
      renameMembers(rootClasses);
      resolveFieldNameConflicts();
      resolveQualifierConflicts();
      if (helper instanceof Tiny2IdentifierRenamer tinyRenamer) {
        tinyRenamer.bindParameterNames(context, interceptor);
      }
      NamingPlan inventory = NamingPlanApplication.capture(context, interceptor);
      NamingPlan plan = new NamingPlan(inventory.classes(), inventory.fields(), inventory.methods(),
        helper instanceof Tiny2IdentifierRenamer tiny ? tiny.declarationNames().parameters() : Map.of(), innerNames);
      // Qualifier repairs can move classes after member names were assigned.
      NamingPlanApplication.applyMemberNames(plan, interceptor);
      retainRenamedBridges();
      context.reloadContext();
      return plan;
    }
    catch (IOException ex) {
      throw new RuntimeException("Renaming failed with exception!", ex);
    }
  }

  private void renameMemberClasses() {
    if (!DecompilerContext.getOption(IFernflowerPreferences.DECOMPILE_INNER)) return;
    Map<String, ClassNode> members = new HashMap<>();
    for (ClassNode node : DecompilerContext.getClassProcessor().getMapRootClasses().values()) {
      if (node.parent != null && node.classStruct.isOwn()) members.put(node.classStruct.qualifiedName, node);
    }
    Set<String> done = new HashSet<>();
    for (String name : new TreeSet<>(members.keySet())) renameMemberClass(name, members, done, new HashSet<>());
  }

  private void renameMemberClass(String name, Map<String, ClassNode> members,
                                 Set<String> done, Set<String> visiting) {
    if (done.contains(name) || !members.containsKey(name)) return;
    if (!visiting.add(name)) throw new IllegalArgumentException("Circular member-class ownership: " + name);
    ClassNode node = members.get(name);
    String parent = node.parent.classStruct.qualifiedName;
    renameMemberClass(parent, members, done, visiting);
    String outer = interceptor.getName(parent);
    if (outer == null) outer = parent;
    String mapped = interceptor.getName(name);
    String simple = innerNames.getOrDefault(name, node.simpleName);
    if (node.type == ClassNode.Type.MEMBER && mapped != null && !innerNames.containsKey(name)
        && !ConverterHelper.getSimpleClassName(mapped).equals(ConverterHelper.getSimpleClassName(name))) {
      // '$' is legal inside a simple name; strip only a known enclosing-class prefix.
      simple = mapped.startsWith(outer + "$") ? mapped.substring(outer.length() + 1) : ConverterHelper.getSimpleClassName(mapped);
    }
    if (simple != null && ConverterHelper.mustRenameForJava(IIdentifierRenamer.Type.ELEMENT_CLASS, simple)) {
      simple = conflictFallbackRenamer.getNextClassName(name, simple);
    }
    String target;
    if (node.type == ClassNode.Type.MEMBER) {
      target = outer + "$" + simple;
    } else if (name.startsWith(parent + "$")) {
      target = outer + name.substring(parent.length());
      if (simple != null && !Objects.equals(simple, node.simpleName)) {
        target = outer + "$1" + simple;
      }
    } else {
      target = mapped == null ? name : mapped;
    }
    String base = target;
    for (int suffix = 1; isClassNameOccupied(target, name); suffix++) target = base + "_" + suffix;
    if (node.type == ClassNode.Type.MEMBER) simple = target.substring(outer.length() + 1);
    if (simple != null) {
      innerNames.put(name, simple);
      interceptor.setInnerName(target, simple);
    }
    if (!target.equals(name)) interceptor.addName(name, target);
    visiting.remove(name);
    done.add(name);
  }

  private void renameMembers(List<ClassWrapperNode> roots) {
    for (ClassWrapperNode node : getReversePostOrderListIterative(roots)) {
      if (node.getClassStruct().isOwn()) {
        renameClassIdentifiers(node.getClassStruct());
      }
    }
  }

  private void renameAllClasses() {
    // order not important
    List<ClassWrapperNode> lstAllClasses = new ArrayList<>(getReversePostOrderListIterative(rootInterfaces));
    lstAllClasses.addAll(getReversePostOrderListIterative(rootClasses));

    // rename all interfaces and classes
    for (ClassWrapperNode node : lstAllClasses) {
      renameClass(node.getClassStruct());
    }
  }

  private void renameClass(StructClass cl) {
    if (!cl.isOwn()) {
      return;
    }

    String classOldFullName = cl.qualifiedName;
    String clSimpleName = ConverterHelper.getSimpleClassName(classOldFullName);
    boolean renameByPolicy = helper instanceof Tiny2IdentifierRenamer
      ? helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_CLASS, classOldFullName, null, null)
      : helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_CLASS, clSimpleName, null, null);
    String targetPackage = forcedPackageRelocations.get(classOldFullName);
    boolean reserved = reservedClassNames.contains(classOldFullName);
    if (!renameByPolicy && !reserved && targetPackage == null) {
      return;
    }

    String classNewFullName;
    if (renameByPolicy || reserved) {
      do {
        String classname = renameByPolicy ? helper.getNextClassName(classOldFullName, clSimpleName)
          : conflictFallbackRenamer.getNextClassName(classOldFullName, clSimpleName);
        classNewFullName = classname.indexOf('/') >= 0
          ? classname
          : ConverterHelper.replaceSimpleClassName(classOldFullName, classname);
        classNewFullName = applyPackageRelocation(targetPackage, classNewFullName);
      }
      while (isClassNameOccupied(classNewFullName, classOldFullName));
    }
    else {
      classNewFullName = applyPackageRelocation(targetPackage, classOldFullName);
      if (isClassNameOccupied(classNewFullName, classOldFullName)) {
        int counter = 1;
        String candidate;
        do {
          if (targetPackage.isEmpty()) {
            candidate = clSimpleName + "_" + counter++;
          }
          else {
            candidate = targetPackage + "/" + clSimpleName + "_" + counter++;
          }
        }
        while (isClassNameOccupied(candidate, classOldFullName));
        classNewFullName = candidate;
      }
    }

    if (!classOldFullName.equals(classNewFullName)) {
      interceptor.addName(classOldFullName, classNewFullName);
    }
  }

  private void collectForcedPackageRelocations() {
    forcedPackageRelocations.clear();

    List<String> defaultOwnClasses = new ArrayList<>();
    Set<String> referencingPackages = new TreeSet<>();

    for (StructClass ownClass : context.getOwnClasses()) {
      String className = ownClass.qualifiedName;
      String ownerPackage = packageName(className);
      if (ownerPackage.isEmpty()) {
        defaultOwnClasses.add(className);
        continue;
      }

      if (!collectDefaultPackageOwnReferences(ownClass).isEmpty()) {
        referencingPackages.add(ownerPackage);
      }
    }

    String configured = (String)DecompilerContext.getProperty(IFernflowerPreferences.DEFAULT_PACKAGE);
    if (defaultOwnClasses.isEmpty() || referencingPackages.isEmpty() && configured.isEmpty()) {
      return;
    }

    String base = configured.isEmpty() ? MULTI_PACKAGE_DEFAULT_RELOCATION : configured;
    boolean occupied = context.getOwnClasses().stream().anyMatch(cl -> packageName(cl.qualifiedName).equals(base));
    String targetPackage = configured.isEmpty() || occupied ? chooseRelocationPackage(base) : configured;
    for (String defaultClass : defaultOwnClasses) {
      forcedPackageRelocations.put(defaultClass, targetPackage);
    }
  }

  private Set<String> collectDefaultPackageOwnReferences(StructClass owner) {
    Set<String> references = new LinkedHashSet<>();

    if (owner.superClass != null) {
      addDefaultPackageOwnReferenceIfPresent(references, owner.superClass.getString());
    }

    for (String interfaceName : owner.getInterfaceNames()) {
      addDefaultPackageOwnReferenceIfPresent(references, interfaceName);
    }

    for (StructField field : owner.getFields()) {
      VarType type = FieldDescriptor.parseDescriptor(field.getDescriptor()).type;
      if (isDefaultPackageOwnType(type)) {
        references.add(type.value);
      }
    }

    for (StructMethod method : owner.getMethods()) {
      MethodDescriptor descriptor = MethodDescriptor.parseDescriptor(method.getDescriptor());
      for (VarType param : descriptor.params) {
        if (isDefaultPackageOwnType(param)) {
          references.add(param.value);
        }
      }

      if (isDefaultPackageOwnType(descriptor.ret)) {
        references.add(descriptor.ret.value);
      }
    }

    ConstantPool pool = owner.getPool();
    if (pool != null) {
      for (PooledConstant pooled : pool.getPool()) {
        if (pooled instanceof PrimitiveConstant primitive && primitive.type == CodeConstants.CONSTANT_Class) {
          addDefaultPackageOwnReferenceIfPresent(references, primitive.getString());
        }
      }
    }

    return references;
  }

  private void addDefaultPackageOwnReferenceIfPresent(Set<String> references, String className) {
    if (isDefaultPackageOwnClass(className)) {
      references.add(className);
    }
  }

  private String chooseRelocationPackage(String base) {
    Set<String> occupied = new HashSet<>();
    for (String name : reservedClassNames) occupied.add(packageName(name));
    for (StructClass cl : context.getOwnClasses()) {
      occupied.add(packageName(cl.qualifiedName));
      if (helper instanceof Tiny2IdentifierRenamer tiny) {
        String mapped = tiny.getMappedClassName(cl.qualifiedName);
        if (mapped != null) occupied.add(packageName(mapped));
      }
    }
    String target = base;
    for (int suffix = 1; occupied.contains(target); suffix++) target = base + suffix;
    return target;
  }

  private static String packageName(String internalClassName) {
    return SourceMethodSemantics.packageName(internalClassName);
  }

  private boolean isDefaultPackageOwnType(VarType type) {
    if (type.value != null && (type.type == CodeType.OBJECT || type.arrayDim > 0)) {
      return isDefaultPackageOwnClass(type.value);
    }
    return false;
  }

  private boolean isDefaultPackageOwnClass(String referencedClass) {
    if (referencedClass == null || referencedClass.isEmpty() || referencedClass.charAt(0) == '[' || referencedClass.indexOf('/') >= 0) {
      return false;
    }

    StructClass referenced = context.getClass(referencedClass);
    return referenced != null && referenced.isOwn() && referenced.qualifiedName.indexOf('/') < 0;
  }

  private static String applyPackageRelocation(String targetPackage, String className) {
    if (targetPackage == null) {
      return className;
    }

    if (targetPackage.isEmpty()) {
      return ConverterHelper.getSimpleClassName(className);
    }

    if (className.indexOf('/') < 0) {
      return targetPackage + "/" + className;
    }

    return className;
  }

  private boolean isClassNameOccupied(String className, String oldName) {
    if (reservedClassNames.contains(className)) return true;
    if (className.equals(oldName)) {
      return false;
    }
    String allocated = interceptor.getOldName(className);
    return context.hasClass(className) || allocated != null && !allocated.equals(oldName);
  }

  private void retainRenamedBridges() {
    for (StructClass owner : context.getOwnClasses()) {
      for (StructMethod bridge : owner.getMethods()) {
        StructMethod target = SourceMethodSemantics.forwardingBridgeTarget(context, owner, bridge);
        if (target == null) continue;
        String bridgeName = currentMethodName(owner, bridge);
        if (!bridgeName.equals(currentMethodName(owner, target))) {
          String emittedOwner = interceptor.getName(owner.qualifiedName);
          interceptor.retainBridge(emittedOwner == null ? owner.qualifiedName : emittedOwner,
            bridgeName, buildNewDescriptor(false, bridge.getDescriptor()));
        }
      }
    }
  }

  private String currentMethodName(StructClass owner, StructMethod method) {
    String renamed = interceptor.getName(buildMethodKey(owner.qualifiedName, method.getName(), method.getDescriptor()));
    return renamed == null ? method.getName() : renamed.split(" ")[1];
  }

  private void renameClassIdentifiers(StructClass cl) {
    // all classes are already renamed
    String classOldFullName = cl.qualifiedName;
    String classNewFullName = interceptor.getName(classOldFullName);

    if (classNewFullName == null) {
      classNewFullName = classOldFullName;
    }

    Map<String, String> inheritedNames = new LinkedHashMap<>();
    Set<String> inheritedMethodSignatures = new HashSet<>();
    collectAncestorMethodNames(cl, cl, new HashSet<>(), inheritedNames, inheritedMethodSignatures);

    // methods
    VBStyleCollection<StructMethod, String> methods = cl.getMethods();
    Set<String> assignedMethodSignatures = new HashSet<>();
    for (int index : buildMethodProcessingOrder(methods, inheritedNames)) {
      StructMethod mt = methods.get(index);
      String key = methods.getKey(index);
      boolean isPrivate = mt.hasModifier(CodeConstants.ACC_PRIVATE);
      boolean isStatic = mt.hasModifier(CodeConstants.ACC_STATIC);
      String methodDescriptor = buildNewDescriptor(false, mt.getDescriptor());

      String oldName = mt.getName();
      if (CodeConstants.INIT_NAME.equals(oldName) || CodeConstants.CLINIT_NAME.equals(oldName)) {
        assignedMethodSignatures.add(methodSignature(oldName, methodDescriptor));
        continue;
      }

      if (mt.hasModifier(CodeConstants.ACC_NATIVE)) {
        // Native entry points must retain their names.
        assignedMethodSignatures.add(methodSignature(oldName, methodDescriptor));
        continue;
      }

      String inheritedName = isPrivate || isStatic ? null : inheritedNames.get(key);
      StructMethod bridge = bridgesByTarget.get(mt);
      if (bridge != null) inheritedName = currentMethodName(cl, bridge);
      String overrideHint = overrideMethodRenameHints.get(buildMethodKey(classOldFullName, oldName, mt.getDescriptor()));
      String inheritedSignature = inheritedName == null ? null : methodSignature(inheritedName, methodDescriptor);
      boolean renameByPolicy = inheritedName == null
        && overrideHint == null
        && helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_METHOD, classOldFullName, oldName, mt.getDescriptor());
      // Hints are computed before source-signature conflicts are resolved.
      // Once a superclass has a realized inherited name, subclasses must keep it.
      String newName = inheritedName != null ? inheritedName : overrideHint != null ? overrideHint : oldName;

      while (renameByPolicy || hasMethodNameConflict(newName, methodDescriptor, assignedMethodSignatures, inheritedMethodSignatures, inheritedSignature)) {
        newName =
          nextMethodName(
            classOldFullName,
            mt,
            methodDescriptor,
            renameByPolicy,
            assignedMethodSignatures,
            inheritedMethodSignatures,
            inheritedSignature
          );
        renameByPolicy = false;
      }

      if (!covariantBridges.containsKey(mt)) assignedMethodSignatures.add(methodSignature(newName, methodDescriptor));

      if (!newName.equals(oldName)) {
        interceptor.addName(classOldFullName + " " + oldName + " " + mt.getDescriptor(),
                            classNewFullName + " " + newName + " " + buildNewDescriptor(false, mt.getDescriptor()));
      }
    }

    // fields
    HashSet<String> occupiedFieldNames = new HashSet<>();
    for (StructField fd : cl.getFields()) {
      String oldName = fd.getName();
      boolean renameByPolicy = helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_FIELD, classOldFullName, oldName, fd.getDescriptor());
      String newName = oldName;

      while (renameByPolicy || occupiedFieldNames.contains(newName)) {
        newName = nextFieldName(classOldFullName, fd, renameByPolicy, occupiedFieldNames);
        renameByPolicy = false;
      }

      occupiedFieldNames.add(newName);

      if (!newName.equals(oldName)) {
        interceptor.addName(classOldFullName + " " + oldName + " " + fd.getDescriptor(),
                            classNewFullName + " " + newName + " " + buildNewDescriptor(true, fd.getDescriptor()));
      }
    }
  }

  @Override
  public String buildNewClassname(String className) {
    return interceptor.getName(className);
  }

  private String buildNewDescriptor(boolean isField, String descriptor) {
    String newDescriptor;
    if (isField) {
      newDescriptor = FieldDescriptor.parseDescriptor(descriptor).buildNewDescriptor(this);
    }
    else {
      newDescriptor = MethodDescriptor.parseDescriptor(descriptor).buildNewDescriptor(this);
    }
    return newDescriptor != null ? newDescriptor : descriptor;
  }

  private static List<Integer> buildMethodProcessingOrder(VBStyleCollection<StructMethod, String> methods, Map<String, String> inheritedNames) {
    List<Integer> inherited = new ArrayList<>();
    List<Integer> own = new ArrayList<>();

    for (int i = 0; i < methods.size(); i++) {
      StructMethod method = methods.get(i);
      boolean hasInheritedName = !method.hasModifier(CodeConstants.ACC_PRIVATE)
                                 && !method.hasModifier(CodeConstants.ACC_STATIC)
                                 && inheritedNames.containsKey(methods.getKey(i));
      if (hasInheritedName) {
        inherited.add(i);
      }
      else {
        own.add(i);
      }
    }

    inherited.addAll(own);
    return inherited;
  }

  private void collectAncestorMethodNames(
    StructClass target,
    StructClass current,
    Set<String> visited,
    Map<String, String> inheritedNames,
    Set<String> renderedSignatures
  ) {
    if (!visited.add(current.qualifiedName)) {
      return;
    }
    if (current != target) {
      for (StructMethod method : current.getMethods()) {
        if (!canParticipateInSourceDeclaration(method)) {
          continue;
        }
        String mapped = interceptor.getName(buildMethodKey(current.qualifiedName, method.getName(), method.getDescriptor()));
        String name = mapped == null ? method.getName() : mapped.split(" ", 3)[1];
        // Use original access for name propagation, but rendered access for
        // collisions: relocation can make formerly independent methods overlap.
        if (SourceMethodSemantics.isOverridableFrom(method, current, target)) {
          inheritedNames.putIfAbsent(method.getName() + " " + method.getDescriptor(), name);
        }
        if (isVisibleInOutput(method, current, target)) {
          renderedSignatures.add(methodSignature(name, buildNewDescriptor(false, method.getDescriptor())));
        }
      }
    }
    if (current.superClass != null) {
      StructClass parent = context.getClass(current.superClass.getString());
      if (parent != null) {
        collectAncestorMethodNames(target, parent, visited, inheritedNames, renderedSignatures);
      }
    }
    for (String interfaceName : current.getInterfaceNames()) {
      StructClass parent = context.getClass(interfaceName);
      if (parent != null) {
        collectAncestorMethodNames(target, parent, visited, inheritedNames, renderedSignatures);
      }
    }
  }

  private boolean isVisibleInOutput(StructMethod method, StructClass declaringClass, StructClass target) {
    return !(declaringClass.hasModifier(CodeConstants.ACC_INTERFACE) && method.hasModifier(CodeConstants.ACC_STATIC))
      && SourceMethodSemantics.isAccessibleFrom(method, renderedClassName(declaringClass), renderedClassName(target));
  }

  private String renderedClassName(StructClass owner) {
    String renamed = interceptor.getName(owner.qualifiedName);
    return renamed == null ? owner.qualifiedName : renamed;
  }

  private String nextMethodName(
    String className,
    StructMethod method,
    String methodDescriptor,
    boolean useHelper,
    Set<String> assignedMethodSignatures,
    Set<String> inheritedMethodSignatures,
    String inheritedSignature
  ) {
    Set<String> attempts = new HashSet<>();
    boolean usePolicyRenamer = useHelper;

    while (true) {
      String candidate = generateMethodNameCandidate(className, method, usePolicyRenamer);
      if (!hasMethodNameConflict(candidate, methodDescriptor, assignedMethodSignatures, inheritedMethodSignatures, inheritedSignature)) {
        return candidate;
      }

      if (!attempts.add((usePolicyRenamer ? "helper:" : "fallback:") + candidate)) {
        usePolicyRenamer = false;
      }
    }
  }

  private String nextFieldName(String className, StructField field, boolean useHelper, Set<String> occupiedFieldNames) {
    Set<String> attempts = new HashSet<>();
    boolean usePolicyRenamer = useHelper;

    while (true) {
      String candidate = generateFieldNameCandidate(className, field, usePolicyRenamer);
      if (!occupiedFieldNames.contains(candidate)) {
        return candidate;
      }

      if (!attempts.add((usePolicyRenamer ? "helper:" : "fallback:") + candidate)) {
        usePolicyRenamer = false;
      }
    }
  }

  private String generateMethodNameCandidate(String className, StructMethod method, boolean useHelper) {
    String candidate = useHelper
      ? helper.getNextMethodName(className, method.getName(), method.getDescriptor())
      : conflictFallbackRenamer.getNextMethodName(className, method.getName(), method.getDescriptor());
    if (candidate == null || candidate.isEmpty()) {
      return conflictFallbackRenamer.getNextMethodName(className, method.getName(), method.getDescriptor());
    }
    return candidate;
  }

  private String generateFieldNameCandidate(String className, StructField field, boolean useHelper) {
    String candidate = useHelper
      ? helper.getNextFieldName(className, field.getName(), field.getDescriptor())
      : conflictFallbackRenamer.getNextFieldName(className, field.getName(), field.getDescriptor());
    if (candidate == null || candidate.isEmpty()) {
      return conflictFallbackRenamer.getNextFieldName(className, field.getName(), field.getDescriptor());
    }
    return candidate;
  }

  private static boolean hasMethodNameConflict(
    String candidateName,
    String methodDescriptor,
    Set<String> assignedMethodSignatures,
    Set<String> inheritedMethodSignatures,
    String inheritedSignature
  ) {
    String candidateSignature = methodSignature(candidateName, methodDescriptor);
    if (assignedMethodSignatures.contains(candidateSignature)) {
      return true;
    }

    if (!inheritedMethodSignatures.contains(candidateSignature)) {
      return false;
    }

    if (inheritedSignature == null) {
      return true;
    }

    return !candidateSignature.equals(inheritedSignature);
  }

  private static String methodSignature(String name, String descriptor) {
    return SourceMethodSemantics.sourceSignature(name, descriptor);
  }

  private void collectOverrideMethodRenameHints() {
    overrideMethodRenameHints = new LinkedHashMap<>();
    List<MethodReference> methods = collectSourceVisibleMethodCandidates();
    if (methods.isEmpty()) {
      return;
    }

    // First keep true override families source-consistent. Static and return-only
    // collisions remain separate components, because Java source cannot express
    // them with the same name even though the JVM can.
    int[] components = buildNamingComponents(methods);
    for (int i = 0; i < components.length; i++) {
      components[i] = findComponent(components, i);
    }

    Map<Integer, List<MethodReference>> methodsByComponent = new LinkedHashMap<>();
    for (int i = 0; i < methods.size(); i++) {
      methodsByComponent.computeIfAbsent(components[i], key -> new ArrayList<>()).add(methods.get(i));
    }

    collectMappedOverrideMethodRenameHints(methodsByComponent);
    collectSourceSignatureConflictRenameHints(methods, components, methodsByComponent);
  }

  private void collectMappedOverrideMethodRenameHints(Map<Integer, List<MethodReference>> methodsByComponent) {
    if (!(helper instanceof Tiny2IdentifierRenamer tinyRenamer)) {
      return;
    }

    List<MethodReference> methods = new ArrayList<>();
    for (List<MethodReference> component : methodsByComponent.values()) {
      methods.addAll(component);
    }

    for (MethodReference method : methods) {
      String mappedName = tinyRenamer.getMappedMethodName(
        method.owner.qualifiedName,
        method.method.getName(),
        method.method.getDescriptor()
      );
      if (mappedName == null || mappedName.isEmpty()) {
        continue;
      }
      method.mappedName = mappedName;
    }

    for (List<MethodReference> component : methodsByComponent.values()) {
      MethodReference namingMethod = component.stream()
        .filter(method -> method.mappedName != null)
        .min(Comparator
          .comparingInt(IdentifierConverter::overrideMappingPriority)
          .thenComparingInt(method -> method.order))
        .orElse(null);
      if (namingMethod == null) {
        continue;
      }

      // Java source cannot rename one member of an override family independently:
      // a partial Tiny mapping for a concrete method must also name its abstract
      // declaration, sibling overrides, and interface declarations satisfied by
      // an inherited superclass method. When mappings disagree, a concrete
      // implementation carries the most source-level signal.
      for (MethodReference method : component) {
        overrideMethodRenameHints.put(
          buildMethodKey(method.owner.qualifiedName, method.method.getName(), method.method.getDescriptor()),
          namingMethod.mappedName
        );
      }
    }
  }

  private void collectSourceSignatureConflictRenameHints(
    List<MethodReference> methods,
    int[] components,
    Map<Integer, List<MethodReference>> methodsByComponent
  ) {
    Map<String, MethodReference> methodsByKey = new HashMap<>();
    Map<String, Integer> componentByMethodKey = new HashMap<>();
    for (int i = 0; i < methods.size(); i++) {
      MethodReference method = methods.get(i);
      String key = buildMethodKey(method.owner.qualifiedName, method.method.getName(), method.method.getDescriptor());
      methodsByKey.put(key, method);
      componentByMethodKey.put(key, components[i]);
    }

    for (StructClass cl : context.getOwnClasses()) {
      LinkedHashMap<Integer, MethodReference> visibleComponents = new LinkedHashMap<>();
      collectVisibleSourceConflictMethods(cl, cl.qualifiedName, new HashSet<>(), methodsByKey, componentByMethodKey, visibleComponents);

      Map<String, LinkedHashMap<Integer, MethodReference>> visibleByRenderedSignature = groupByRenderedSourceSignature(
        visibleComponents,
        methodsByComponent
      );
      Set<String> usedNames = collectRenderedMethodNames(visibleComponents, methodsByComponent);

      // Java source identity is rendered name + parameters. True override
      // components have already been merged; any remaining rendered collision
      // is a distinct bytecode method identity and needs a distinct source name.
      for (LinkedHashMap<Integer, MethodReference> renderedComponents : visibleByRenderedSignature.values()) {
        if (renderedComponents.size() < 2) {
          continue;
        }

        int keeper = chooseMethodConflictKeeper(renderedComponents.keySet(), methodsByComponent);
        for (int component : renderedComponents.keySet()) {
          if (component == keeper || !canRenameMethodComponent(methodsByComponent.get(component))) {
            continue;
          }

          String newName = nextMethodConflictName(methodsByComponent.get(component), usedNames);
          usedNames.add(newName);
          addComponentRenameHint(methodsByComponent.get(component), newName);
        }
      }
    }
  }

  private Map<String, LinkedHashMap<Integer, MethodReference>> groupByRenderedSourceSignature(
    LinkedHashMap<Integer, MethodReference> visibleComponents,
    Map<Integer, List<MethodReference>> methodsByComponent
  ) {
    Map<String, LinkedHashMap<Integer, MethodReference>> visibleByRenderedSignature = new LinkedHashMap<>();
    for (Map.Entry<Integer, MethodReference> visible : visibleComponents.entrySet()) {
      int component = visible.getKey();
      MethodReference ref = visible.getValue();
      String hint = getComponentRenameHint(methodsByComponent.get(component));
      String renderedName = hint != null ? hint : ref.method.getName();
      String renderedSignature = methodSignature(renderedName, buildNewDescriptor(false, ref.method.getDescriptor()));
      visibleByRenderedSignature.computeIfAbsent(renderedSignature, unused -> new LinkedHashMap<>()).put(component, ref);
    }
    return visibleByRenderedSignature;
  }

  private Set<String> collectRenderedMethodNames(
    LinkedHashMap<Integer, MethodReference> visibleComponents,
    Map<Integer, List<MethodReference>> methodsByComponent
  ) {
    Set<String> usedNames = new HashSet<>();
    for (Map.Entry<Integer, MethodReference> visible : visibleComponents.entrySet()) {
      String hint = getComponentRenameHint(methodsByComponent.get(visible.getKey()));
      usedNames.add(hint != null ? hint : visible.getValue().method.getName());
    }
    return usedNames;
  }

  private void collectVisibleSourceConflictMethods(
    StructClass target,
    String className,
    Set<String> visited,
    Map<String, MethodReference> methodsByKey,
    Map<String, Integer> componentByMethodKey,
    LinkedHashMap<Integer, MethodReference> visibleComponents
  ) {
    if (!visited.add(className)) {
      return;
    }

    StructClass cl = context.getClass(className);
    if (cl == null) {
      return;
    }

    for (StructMethod method : cl.getMethods()) {
      if (!canParticipateInSourceDeclaration(method)
          || (cl != target && !isVisibleInOutput(method, cl, target))) {
        continue;
      }

      String key = buildMethodKey(cl.qualifiedName, method.getName(), method.getDescriptor());
      Integer component = componentByMethodKey.get(key);
      MethodReference ref = methodsByKey.get(key);
      if (component != null && ref != null) {
        visibleComponents.putIfAbsent(component, ref);
      }
    }

    if (cl.superClass != null) {
      collectVisibleSourceConflictMethods(target, cl.superClass.getString(), visited, methodsByKey, componentByMethodKey, visibleComponents);
    }

    for (String ifName : cl.getInterfaceNames()) {
      collectVisibleSourceConflictMethods(target, ifName, visited, methodsByKey, componentByMethodKey, visibleComponents);
    }
  }

  private int chooseMethodConflictKeeper(Collection<Integer> components, Map<Integer, List<MethodReference>> methodsByComponent) {
    return components.stream()
      .min(Comparator
        .comparingInt((Integer component) -> methodConflictKeeperRank(methodsByComponent.get(component)))
        .thenComparingInt(component -> methodsByComponent.get(component).stream().mapToInt(method -> method.order).min().orElse(Integer.MAX_VALUE)))
      .orElseThrow();
  }

  private static int methodConflictKeeperRank(List<MethodReference> component) {
    if (!canRenameMethodComponent(component)) {
      return 0;
    }

    boolean hasClassMethod = false;
    boolean hasConcreteClassMethod = false;

    for (MethodReference method : component) {
      if (method.owner.hasModifier(CodeConstants.ACC_INTERFACE)) {
        continue;
      }

      hasClassMethod = true;
      if (method.method.hasModifier(CodeConstants.ACC_FINAL)) {
        return 1;
      }
      if (!method.method.hasModifier(CodeConstants.ACC_ABSTRACT)) {
        hasConcreteClassMethod = true;
      }
    }

    if (hasConcreteClassMethod) {
      return 2;
    }
    if (hasClassMethod) {
      return 3;
    }
    return 4;
  }

  private static boolean canRenameMethodComponent(List<MethodReference> component) {
    for (MethodReference method : component) {
      if (!method.owner.isOwn()) {
        return false;
      }
    }
    return true;
  }

  private String nextMethodConflictName(List<MethodReference> component, Set<String> usedNames) {
    MethodReference method = component.get(0);
    while (true) {
      String candidate = generateMethodNameCandidate(method.owner.qualifiedName, method.method, false);
      if (!usedNames.contains(candidate)) {
        return candidate;
      }
    }
  }

  private String getComponentRenameHint(List<MethodReference> component) {
    for (MethodReference method : component) {
      String hint = overrideMethodRenameHints.get(buildMethodKey(method.owner.qualifiedName, method.method.getName(), method.method.getDescriptor()));
      if (hint != null) {
        return hint;
      }
    }
    return null;
  }

  private void addComponentRenameHint(List<MethodReference> component, String newName) {
    for (MethodReference method : component) {
      overrideMethodRenameHints.put(
        buildMethodKey(method.owner.qualifiedName, method.method.getName(), method.method.getDescriptor()),
        newName
      );
    }
  }

  private List<MethodReference> collectSourceVisibleMethodCandidates() {
    List<MethodReference> methods = new ArrayList<>();
    List<ClassWrapperNode> ordered = new ArrayList<>(getReversePostOrderListIterative(rootInterfaces));
    ordered.addAll(getReversePostOrderListIterative(rootClasses));

    for (ClassWrapperNode node : ordered) {
      StructClass owner = node.getClassStruct();
      for (StructMethod method : owner.getMethods()) {
        if (canParticipateInSourceDeclaration(method)
            && (owner.isOwn() || isExternalStaticConflictBlocker(method))) {
          methods.add(new MethodReference(owner, method, methods.size()));
        }
      }
    }

    return methods;
  }

  private int[] buildNamingComponents(List<MethodReference> methods) {
    int[] components = new int[methods.size()];
    for (int i = 0; i < components.length; i++) {
      components[i] = i;
    }

    // Unrelated signatures cannot share an override family. Keep input order
    // within each bucket so conflict resolution remains deterministic.
    Map<String, List<MethodReference>> bySignature = new LinkedHashMap<>();
    for (MethodReference method : methods) {
      if (SourceMethodSemantics.canParticipateInOverride(method.method)) {
        bySignature.computeIfAbsent(method.sourceSignature, unused -> new ArrayList<>()).add(method);
      }
    }
    for (List<MethodReference> bucket : bySignature.values()) {
      for (int i = 0; i < bucket.size(); i++) {
        MethodReference first = bucket.get(i);
        for (int j = i + 1; j < bucket.size(); j++) {
          MethodReference second = bucket.get(j);
          if (first.method.getDescriptor().equals(second.method.getDescriptor())
              && SourceMethodSemantics.areOverrideRelated(context, first.owner, first.method, second.owner, second.method)) {
            unionComponents(components, first.order, second.order);
          }
        }
      }
    }

    Map<StructMethod, MethodReference> references = new IdentityHashMap<>();
    for (MethodReference method : methods) references.put(method.method, method);
    for (MethodReference method : methods) {
      if (DecompilerContext.shouldUseLegacySourceCompatibility(method.owner, BytecodeVersion.MAJOR_5)) continue;
      StructMethod target = SourceMethodSemantics.covariantBridgeTarget(context, method.owner, method.method);
      MethodReference targetReference = references.get(target);
      // A modern compiler regenerates this proven forwarding bridge. Sharing
      // its source name does not merge unrelated return-only implementations.
      if (targetReference != null) {
        covariantBridges.put(method.method, target);
        bridgesByTarget.putIfAbsent(target, method.method);
        unionComponents(components, method.order, targetReference.order);
      }
    }
    return components;
  }

  private static int findComponent(int[] components, int index) {
    int parent = components[index];
    if (parent != index) {
      parent = findComponent(components, parent);
      components[index] = parent;
    }
    return parent;
  }

  private static void unionComponents(int[] components, int first, int second) {
    int firstRoot = findComponent(components, first);
    int secondRoot = findComponent(components, second);
    if (firstRoot != secondRoot) {
      components[secondRoot] = firstRoot;
    }
  }

  private static boolean canParticipateInSourceDeclaration(StructMethod method) {
    return !CodeConstants.INIT_NAME.equals(method.getName())
           && !CodeConstants.CLINIT_NAME.equals(method.getName());
  }

  private static boolean isExternalStaticConflictBlocker(StructMethod method) {
    return method.hasModifier(CodeConstants.ACC_STATIC)
           && !method.hasModifier(CodeConstants.ACC_PRIVATE);
  }

  private static int overrideMappingPriority(MethodReference method) {
    if (!method.owner.hasModifier(CodeConstants.ACC_INTERFACE) && !method.method.hasModifier(CodeConstants.ACC_ABSTRACT)) {
      return 0;
    }
    if (!method.owner.hasModifier(CodeConstants.ACC_INTERFACE)) {
      return 1;
    }
    return 2;
  }

  private static String buildMethodKey(String owner, String name, String descriptor) {
    return owner + " " + name + " " + descriptor;
  }

  private void resolveFieldNameConflicts() {
    Map<String, Set<String>> ownerOccupiedFieldNames = new HashMap<>();
    for (StructClass cl : context.getOwnClasses()) {
      resolveVisibleFieldConflicts(cl, ownerOccupiedFieldNames);
    }
  }

  private void resolveQualifierConflicts() {
    Map<String, Set<String>> occupied = new HashMap<>();
    for (StructClass owner : context.getOwnClasses()) {
      Map<String, List<FieldReference>> fields = new LinkedHashMap<>();
      Set<String> visited = new HashSet<>();
      collectVisibleFields(owner, visited, fields);
      ClassNode node = DecompilerContext.getClassProcessor().getMapRootClasses().get(owner.qualifiedName);
      for (ClassNode parent = node == null ? null : node.parent; parent != null; parent = parent.parent) {
        collectVisibleFields(parent.classStruct, visited, fields);
      }
      for (PooledConstant constant : owner.getPool().getPool()) {
        if (!(constant instanceof LinkConstant link)) continue;
        boolean isField = link.type == CodeConstants.CONSTANT_Fieldref;
        if (!isField && link.type != CodeConstants.CONSTANT_Methodref && link.type != CodeConstants.CONSTANT_InterfaceMethodref) continue;
        String declaration = interceptor.originalMemberOwner(link, isField);
        StructClass declaringClass = declaration == null ? null : context.getClass(declaration);
        if (declaringClass == null) continue;
        var member = isField ? declaringClass.getField(link.elementname, link.descriptor) : declaringClass.getMethod(link.elementname, link.descriptor);
        if (member == null || !member.hasModifier(CodeConstants.ACC_STATIC)) continue;
        ClassNode qualifier = DecompilerContext.getClassProcessor().getMapRootClasses().get(link.classname);
        while (qualifier != null && qualifier.type == ClassNode.Type.MEMBER && qualifier.parent != null) qualifier = qualifier.parent;
        String qualifierOwner = qualifier == null ? link.classname : qualifier.classStruct.qualifiedName;
        String target = interceptor.getName(qualifierOwner);
        if (target == null) target = qualifierOwner;
        String simple = ConverterHelper.getSimpleClassName(target);
        List<FieldReference> shadowing = fields.get(simple);
        if (shadowing == null) continue;
        int slash = target.indexOf('/');
        if (slash >= 0 && !fields.containsKey(target.substring(0, slash))) continue;
        // Neither a simple nor a package-qualified expression can name this type.
        Set<String> blocked = new HashSet<>(fields.keySet());
        for (FieldReference field : shadowing) {
          if (!field.currentName.equals(simple)) continue;
          if (!field.ownerClass.isOwn()) {
            StructClass targetClass = context.getClass(qualifierOwner);
            if (targetClass == null || !targetClass.isOwn()) {
              throw new IllegalArgumentException("Cannot qualify external class " + target + " in " + owner.qualifiedName
                + ": inherited field " + simple + " also obscures its package");
            }
            String replacement;
            do {
              String next = conflictFallbackRenamer.getNextClassName(qualifierOwner, simple);
              replacement = ConverterHelper.replaceSimpleClassName(target, next);
            } while (blocked.contains(ConverterHelper.getSimpleClassName(replacement)) || isClassNameOccupied(replacement, qualifierOwner));
            interceptor.addName(qualifierOwner, replacement);
            renameMemberClasses();
            break;
          }
          renameFieldReference(field, blocked, occupied);
        }
      }
    }
  }

  private void resolveVisibleFieldConflicts(StructClass cl, Map<String, Set<String>> ownerOccupiedFieldNames) {
    Map<String, List<FieldReference>> fieldsByName = new LinkedHashMap<>();
    collectVisibleFields(cl, new HashSet<>(), fieldsByName);

    for (List<FieldReference> conflictingFields : fieldsByName.values()) {
      if (conflictingFields.size() < 2) {
        continue;
      }

      FieldReference keeper = chooseFieldConflictKeeper(conflictingFields);
      Set<String> disallowedNames = new HashSet<>();
      for (FieldReference ref : conflictingFields) {
        disallowedNames.add(ref.currentName);
      }

      for (FieldReference ref : conflictingFields) {
        if (ref == keeper || !ref.ownerClass.isOwn()) {
          continue;
        }
        renameFieldReference(ref, disallowedNames, ownerOccupiedFieldNames);
      }
    }
  }

  private void collectVisibleFields(StructClass cl, Set<String> visited, Map<String, List<FieldReference>> fieldsByName) {
    if (!visited.add(cl.qualifiedName)) return;
    SourceFieldScope.visit(context, cl, name -> {
      String renamed = interceptor.getName(name);
      return renamed == null ? name : renamed;
    }, (owner, field) -> {
      String name = resolveCurrentFieldName(owner.qualifiedName, field);
      List<FieldReference> references = fieldsByName.computeIfAbsent(name, key -> new ArrayList<>());
      if (references.stream().noneMatch(reference -> reference.field == field)) references.add(new FieldReference(owner, field, name));
    });
  }

  private static FieldReference chooseFieldConflictKeeper(List<FieldReference> conflictingFields) {
    for (FieldReference ref : conflictingFields) {
      if (!ref.ownerClass.hasModifier(CodeConstants.ACC_INTERFACE)) {
        return ref;
      }
    }
    return conflictingFields.get(0);
  }

  private void renameFieldReference(
    FieldReference ref,
    Set<String> disallowedNames,
    Map<String, Set<String>> ownerOccupiedFieldNames
  ) {
    String owner = ref.ownerClass.qualifiedName;
    Set<String> occupiedNames = ownerOccupiedFieldNames.computeIfAbsent(owner, key -> getOwnerOccupiedFieldNames(ref.ownerClass));

    boolean renameByPolicy = helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_FIELD, owner, ref.field.getName(), ref.field.getDescriptor());
    String newName = ref.currentName;
    while (renameByPolicy || occupiedNames.contains(newName) || disallowedNames.contains(newName)) {
      Set<String> blockedNames = new HashSet<>(occupiedNames);
      blockedNames.addAll(disallowedNames);
      newName = nextFieldName(owner, ref.field, renameByPolicy, blockedNames);
      renameByPolicy = false;
    }

    if (newName.equals(ref.currentName)) {
      return;
    }

    occupiedNames.add(newName);
    disallowedNames.add(newName);
    ref.currentName = newName;

    String ownerNew = interceptor.getName(owner);
    if (ownerNew == null) {
      ownerNew = owner;
    }

    interceptor.addName(
      buildFieldKey(owner, ref.field.getName(), ref.field.getDescriptor()),
      ownerNew + " " + newName + " " + buildNewDescriptor(true, ref.field.getDescriptor())
    );
  }

  private Set<String> getOwnerOccupiedFieldNames(StructClass ownerClass) {
    Set<String> occupiedNames = new HashSet<>();
    for (StructField field : ownerClass.getFields()) {
      occupiedNames.add(resolveCurrentFieldName(ownerClass.qualifiedName, field));
    }
    return occupiedNames;
  }

  private String resolveCurrentFieldName(String owner, StructField field) {
    String mapped = interceptor.getName(buildFieldKey(owner, field.getName(), field.getDescriptor()));
    if (mapped == null) {
      return field.getName();
    }

    String[] parts = mapped.split(" ", 3);
    if (parts.length >= 2 && !parts[1].isEmpty()) {
      return parts[1];
    }

    return field.getName();
  }

  private static String buildFieldKey(String owner, String name, String descriptor) {
    return owner + " " + name + " " + descriptor;
  }

  private static final class FieldReference {
    private final StructClass ownerClass;
    private final StructField field;
    private String currentName;

    private FieldReference(StructClass ownerClass, StructField field, String currentName) {
      this.ownerClass = ownerClass;
      this.field = field;
      this.currentName = currentName;
    }
  }

  private static final class MethodReference {
    private final StructClass owner;
    private final StructMethod method;
    private final String sourceSignature;
    private final int order;
    private String mappedName;

    private MethodReference(StructClass owner, StructMethod method, int order) {
      this.owner = owner;
      this.method = method;
      this.sourceSignature = SourceMethodSemantics.sourceSignature(method);
      this.order = order;
    }
  }

  private static List<ClassWrapperNode> getReversePostOrderListIterative(List<ClassWrapperNode> roots) {
    List<ClassWrapperNode> res = new ArrayList<>();

    LinkedList<ClassWrapperNode> stackNode = new LinkedList<>();
    LinkedList<Integer> stackIndex = new LinkedList<>();

    Set<ClassWrapperNode> setVisited = new HashSet<>();

    for (ClassWrapperNode root : roots) {
      stackNode.add(root);
      stackIndex.add(0);
    }

    while (!stackNode.isEmpty()) {
      ClassWrapperNode node = stackNode.getLast();
      int index = stackIndex.removeLast();

      setVisited.add(node);

      List<ClassWrapperNode> lstSubs = node.getSubclasses();

      for (; index < lstSubs.size(); index++) {
        ClassWrapperNode sub = lstSubs.get(index);
        if (!setVisited.contains(sub)) {
          stackIndex.add(index + 1);
          stackNode.add(sub);
          stackIndex.add(0);
          break;
        }
      }

      if (index == lstSubs.size()) {
        res.add(0, node);
        stackNode.removeLast();
      }
    }

    return res;
  }

  private void buildInheritanceTree() {
    Map<String, ClassWrapperNode> nodes = new LinkedHashMap<>();
    List<StructClass> classes = context.getOwnClasses();

    List<ClassWrapperNode> rootClasses = new ArrayList<>();
    List<ClassWrapperNode> rootInterfaces = new ArrayList<>();

    for (StructClass cl : classes) {
      LinkedList<StructClass> stack = new LinkedList<>();
      LinkedList<ClassWrapperNode> stackSubNodes = new LinkedList<>();

      stack.add(cl);
      stackSubNodes.add(null);

      while (!stack.isEmpty()) {
        StructClass clStr = stack.removeFirst();
        ClassWrapperNode child = stackSubNodes.removeFirst();

        ClassWrapperNode node = nodes.get(clStr.qualifiedName);
        boolean isNewNode = (node == null);

        if (isNewNode) {
          nodes.put(clStr.qualifiedName, node = new ClassWrapperNode(clStr));
        }

        if (child != null) {
          node.addSubclass(child);
        }

        if (!isNewNode) {
          break;
        }
        else {
          boolean isInterface = clStr.hasModifier(CodeConstants.ACC_INTERFACE);
          boolean found_parent = false;

          if (isInterface) {
            for (String ifName : clStr.getInterfaceNames()) {
              StructClass clParent = context.getClass(ifName);
              if (clParent != null) {
                stack.add(clParent);
                stackSubNodes.add(node);
                found_parent = true;
              }
            }
          }
          else if (clStr.superClass != null) { // null iff java/lang/Object
            StructClass clParent = context.getClass(clStr.superClass.getString());
            if (clParent != null) {
              stack.add(clParent);
              stackSubNodes.add(node);
              found_parent = true;
            }
          }

          if (!found_parent) { // no super class or interface
            (isInterface ? rootInterfaces : rootClasses).add(node);
          }
        }
      }
    }

    this.rootClasses = rootClasses;
    this.rootInterfaces = rootInterfaces;
  }
}
