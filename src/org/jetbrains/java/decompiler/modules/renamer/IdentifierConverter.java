// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructField;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.consts.ConstantPool;
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

public class IdentifierConverter implements NewClassNameBuilder {
  private final StructContext context;
  private final IIdentifierRenamer helper;
  private final ConverterHelper conflictFallbackRenamer = new ConverterHelper();
  private final PoolInterceptor interceptor;
  private List<ClassWrapperNode> rootClasses = new ArrayList<>();
  private List<ClassWrapperNode> rootInterfaces = new ArrayList<>();
  private Map<String, Map<String, String>> interfaceNameMaps = new LinkedHashMap<>();
  private Map<String, String> overrideMethodRenameHints = new LinkedHashMap<>();
  private static final String MULTI_PACKAGE_DEFAULT_RELOCATION = "decompiled/defaultpkg";
  private final Map<String, String> forcedPackageRelocations = new HashMap<>();

  public IdentifierConverter(StructContext context, IIdentifierRenamer helper, PoolInterceptor interceptor) {
    this.context = context;
    this.helper = helper;
    this.interceptor = interceptor;
  }

  public void rename() {
    try {
      buildInheritanceTree();
      collectForcedPackageRelocations();
      renameAllClasses();
      collectOverrideMethodRenameHints();
      renameInterfaces();
      renameClasses();
      resolveFieldNameConflicts();
      context.reloadContext();
    }
    catch (IOException ex) {
      throw new RuntimeException("Renaming failed with exception!", ex);
    }
  }

  private void renameClasses() {
    List<ClassWrapperNode> lstClasses = getReversePostOrderListIterative(rootClasses);
    Map<String, Map<String, String>> classNameMaps = new LinkedHashMap<>();

    for (ClassWrapperNode node : lstClasses) {
      StructClass cl = node.getClassStruct();
      Map<String, String> names = new LinkedHashMap<>();

      // merge information on super class
      if (cl.superClass != null) {
        Map<String, String> mapClass = classNameMaps.get(cl.superClass.getString());
        if (mapClass != null) {
          names.putAll(mapClass);
        }
      }

      // merge information on interfaces
      for (String ifName : cl.getInterfaceNames()) {
        Map<String, String> mapInt = interfaceNameMaps.get(ifName);
        if (mapInt != null) {
          names.putAll(mapInt);
        }
        else {
          StructClass clintr = context.getClass(ifName);
          if (clintr != null) {
            names.putAll(processExternalInterface(clintr));
          }
        }
      }

      renameClassIdentifiers(cl, names);

      if (!node.getSubclasses().isEmpty()) {
        classNameMaps.put(cl.qualifiedName, names);
      }
    }
  }

  private Map<String, String> processExternalInterface(StructClass cl) {
    Map<String, String> names = new LinkedHashMap<>();

    for (String ifName : cl.getInterfaceNames()) {
      Map<String, String> mapInt = interfaceNameMaps.get(ifName);
      if (mapInt != null) {
        names.putAll(mapInt);
      }
      else {
        StructClass clintr = context.getClass(ifName);
        if (clintr != null) {
          names.putAll(processExternalInterface(clintr));
        }
      }
    }

    renameClassIdentifiers(cl, names);

    return names;
  }

  private void renameInterfaces() {
    List<ClassWrapperNode> lstInterfaces = getReversePostOrderListIterative(rootInterfaces);
    Map<String, Map<String, String>> interfaceNameMaps = new LinkedHashMap<>();

    // rename methods and fields
    for (ClassWrapperNode node : lstInterfaces) {

      StructClass cl = node.getClassStruct();
      Map<String, String> names = new LinkedHashMap<>();

      // merge information on super interfaces
      for (String ifName : cl.getInterfaceNames()) {
        Map<String, String> mapInt = interfaceNameMaps.get(ifName);
        if (mapInt != null) {
          names.putAll(mapInt);
        }
      }

      renameClassIdentifiers(cl, names);

      interfaceNameMaps.put(cl.qualifiedName, names);
    }

    this.interfaceNameMaps = interfaceNameMaps;
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
    if (!renameByPolicy && targetPackage == null) {
      return;
    }

    String classNewFullName;
    if (renameByPolicy) {
      do {
        String classname = helper.getNextClassName(classOldFullName, clSimpleName);
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

    if (defaultOwnClasses.isEmpty() || referencingPackages.isEmpty()) {
      return;
    }

    String targetPackage = chooseRelocationPackage(referencingPackages);
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

  private static String chooseRelocationPackage(Set<String> referencingPackages) {
    if (referencingPackages.size() == 1) {
      return referencingPackages.iterator().next();
    }
    return MULTI_PACKAGE_DEFAULT_RELOCATION;
  }

  private static String packageName(String internalClassName) {
    int idx = internalClassName.lastIndexOf('/');
    return idx < 0 ? "" : internalClassName.substring(0, idx);
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
    if (className.equals(oldName)) {
      return false;
    }
    return context.hasClass(className) || interceptor.getOldName(className) != null;
  }

  private void renameClassIdentifiers(StructClass cl, Map<String, String> names) {
    // all classes are already renamed
    String classOldFullName = cl.qualifiedName;
    String classNewFullName = interceptor.getName(classOldFullName);

    if (classNewFullName == null) {
      classNewFullName = classOldFullName;
    }

    Map<String, String> inheritedNames = new LinkedHashMap<>(names);
    Set<String> inheritedMethodSignatures = collectInheritedMethodSignatures(inheritedNames);

    // methods
    VBStyleCollection<StructMethod, String> methods = cl.getMethods();
    Set<String> assignedMethodSignatures = new HashSet<>();
    for (int index : buildMethodProcessingOrder(methods, inheritedNames)) {
      StructMethod mt = methods.get(index);
      String key = methods.getKey(index);
      boolean isPrivate = mt.hasModifier(CodeConstants.ACC_PRIVATE);
      String methodDescriptor = buildNewDescriptor(false, mt.getDescriptor());

      String oldName = mt.getName();
      if (CodeConstants.INIT_NAME.equals(oldName) || CodeConstants.CLINIT_NAME.equals(oldName)) {
        if (!isPrivate) {
          names.put(key, oldName);
        }
        assignedMethodSignatures.add(methodSignature(oldName, methodDescriptor));
        continue;
      }

      if (!cl.isOwn() || mt.hasModifier(CodeConstants.ACC_NATIVE)) {
        // external and native methods must not be renamed
        if (!isPrivate) {
          names.put(key, oldName);
        }
        assignedMethodSignatures.add(methodSignature(oldName, methodDescriptor));
        continue;
      }

      String inheritedName = isPrivate ? null : inheritedNames.get(key);
      String overrideHint = overrideMethodRenameHints.get(buildMethodKey(classOldFullName, oldName, mt.getDescriptor()));
      String inheritedSignature = inheritedName == null ? null : methodSignature(inheritedName, methodDescriptor);
      boolean renameByPolicy = inheritedName == null
        && overrideHint == null
        && helper.toBeRenamed(IIdentifierRenamer.Type.ELEMENT_METHOD, classOldFullName, oldName, mt.getDescriptor());
      String newName = overrideHint != null ? overrideHint : inheritedName != null ? inheritedName : oldName;

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

      assignedMethodSignatures.add(methodSignature(newName, methodDescriptor));

      if (!isPrivate) {
        names.put(key, newName);
      }

      if (!newName.equals(oldName)) {
        interceptor.addName(classOldFullName + " " + oldName + " " + mt.getDescriptor(),
                            classNewFullName + " " + newName + " " + buildNewDescriptor(false, mt.getDescriptor()));
      }
    }

    // external fields are not being renamed
    if (!cl.isOwn()) {
      return;
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
      boolean hasInheritedName = !method.hasModifier(CodeConstants.ACC_PRIVATE) && inheritedNames.containsKey(methods.getKey(i));
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

  private Set<String> collectInheritedMethodSignatures(Map<String, String> inheritedNames) {
    Set<String> signatures = new HashSet<>();
    for (Map.Entry<String, String> entry : inheritedNames.entrySet()) {
      String descriptor = descriptorFromMethodKey(entry.getKey());
      if (descriptor == null) {
        continue;
      }
      signatures.add(methodSignature(entry.getValue(), buildNewDescriptor(false, descriptor)));
    }
    return signatures;
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

  private static String descriptorFromMethodKey(String key) {
    int split = key.indexOf(' ');
    if (split < 0 || split + 1 >= key.length()) {
      return null;
    }
    return key.substring(split + 1);
  }

  private static String methodSignature(String name, String descriptor) {
    return name + " " + parameterDescriptor(descriptor);
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
    int[] components = buildOverrideComponents(methods);
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
      Map<String, LinkedHashMap<Integer, MethodReference>> visibleBySignature = new LinkedHashMap<>();
      collectVisibleSourceConflictMethods(cl.qualifiedName, new HashSet<>(), methodsByKey, componentByMethodKey, visibleBySignature);

      // Java method identity is name + parameters. If two independent bytecode
      // components with that source signature are visible from the same class,
      // one of the components must be renamed.
      for (Map.Entry<String, LinkedHashMap<Integer, MethodReference>> entry : visibleBySignature.entrySet()) {
        LinkedHashMap<Integer, MethodReference> visibleComponents = entry.getValue();
        if (visibleComponents.size() < 2) {
          continue;
        }

        int keeper = chooseMethodConflictKeeper(visibleComponents.keySet(), methodsByComponent);
        Set<String> usedNames = new HashSet<>();
        for (int component : visibleComponents.keySet()) {
          String hint = getComponentRenameHint(methodsByComponent.get(component));
          usedNames.add(hint != null ? hint : visibleComponents.get(component).method.getName());
        }

        for (int component : visibleComponents.keySet()) {
          if (component == keeper || getComponentRenameHint(methodsByComponent.get(component)) != null) {
            continue;
          }

          String newName = nextMethodConflictName(methodsByComponent.get(component), usedNames);
          usedNames.add(newName);
          addComponentRenameHint(methodsByComponent.get(component), newName);
        }
      }
    }
  }

  private void collectVisibleSourceConflictMethods(
    String className,
    Set<String> visited,
    Map<String, MethodReference> methodsByKey,
    Map<String, Integer> componentByMethodKey,
    Map<String, LinkedHashMap<Integer, MethodReference>> visibleBySignature
  ) {
    if (!visited.add(className)) {
      return;
    }

    StructClass cl = context.getClass(className);
    if (cl == null) {
      return;
    }

    for (StructMethod method : cl.getMethods()) {
      if (!canParticipateInSourceVisibleConflict(method)) {
        continue;
      }

      String key = buildMethodKey(cl.qualifiedName, method.getName(), method.getDescriptor());
      Integer component = componentByMethodKey.get(key);
      MethodReference ref = methodsByKey.get(key);
      if (component != null && ref != null) {
        visibleBySignature.computeIfAbsent(ref.sourceSignature, unused -> new LinkedHashMap<>()).putIfAbsent(component, ref);
      }
    }

    if (cl.superClass != null) {
      collectVisibleSourceConflictMethods(cl.superClass.getString(), visited, methodsByKey, componentByMethodKey, visibleBySignature);
    }

    for (String ifName : cl.getInterfaceNames()) {
      collectVisibleSourceConflictMethods(ifName, visited, methodsByKey, componentByMethodKey, visibleBySignature);
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
    boolean hasClassMethod = false;
    boolean hasConcreteClassMethod = false;

    for (MethodReference method : component) {
      if (method.owner.hasModifier(CodeConstants.ACC_INTERFACE)) {
        continue;
      }

      hasClassMethod = true;
      if (method.method.hasModifier(CodeConstants.ACC_FINAL)) {
        return 0;
      }
      if (!method.method.hasModifier(CodeConstants.ACC_ABSTRACT)) {
        hasConcreteClassMethod = true;
      }
    }

    if (hasConcreteClassMethod) {
      return 1;
    }
    if (hasClassMethod) {
      return 2;
    }
    return 3;
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
      if (!owner.isOwn()) {
        continue;
      }

      for (StructMethod method : owner.getMethods()) {
        if (canParticipateInSourceVisibleConflict(method)) {
          methods.add(new MethodReference(owner, method, methods.size()));
        }
      }
    }

    return methods;
  }

  private int[] buildOverrideComponents(List<MethodReference> methods) {
    int[] components = new int[methods.size()];
    for (int i = 0; i < components.length; i++) {
      components[i] = i;
    }

    for (int i = 0; i < methods.size(); i++) {
      for (int j = i + 1; j < methods.size(); j++) {
        if (areOverrideRelated(methods.get(i), methods.get(j))) {
          unionComponents(components, i, j);
        }
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

  private boolean areOverrideRelated(MethodReference first, MethodReference second) {
    if (!first.sourceSignature.equals(second.sourceSignature)) {
      return false;
    }

    if (!canParticipateInOverride(first.method) || !canParticipateInOverride(second.method)) {
      return false;
    }

    if (first.owner.qualifiedName.equals(second.owner.qualifiedName)) {
      return first.method.getName().equals(second.method.getName())
             && first.method.getDescriptor().equals(second.method.getDescriptor());
    }

    if (isSubtype(first.owner.qualifiedName, second.owner.qualifiedName)) {
      return isReturnOverrideCompatible(first.descriptor.ret, second.descriptor.ret);
    }

    if (isSubtype(second.owner.qualifiedName, first.owner.qualifiedName)) {
      return isReturnOverrideCompatible(second.descriptor.ret, first.descriptor.ret);
    }

    return isInheritedInterfaceImplementation(first, second);
  }

  private boolean isInheritedInterfaceImplementation(MethodReference first, MethodReference second) {
    boolean firstInterface = first.owner.hasModifier(CodeConstants.ACC_INTERFACE);
    boolean secondInterface = second.owner.hasModifier(CodeConstants.ACC_INTERFACE);
    if (firstInterface == secondInterface) {
      return false;
    }

    MethodReference intf = firstInterface ? first : second;
    MethodReference impl = firstInterface ? second : first;
    if (!impl.method.hasModifier(CodeConstants.ACC_PUBLIC) || !isReturnOverrideCompatible(impl.descriptor.ret, intf.descriptor.ret)) {
      return false;
    }

    for (StructClass cls : context.getOwnClasses()) {
      if (!cls.hasModifier(CodeConstants.ACC_INTERFACE)
          && isSubtype(cls.qualifiedName, impl.owner.qualifiedName)
          && isSubtype(cls.qualifiedName, intf.owner.qualifiedName)) {
        return true;
      }
    }

    return false;
  }

  private boolean isSubtype(String child, String parent) {
    return isSubtype(child, parent, new HashSet<>());
  }

  private boolean isSubtype(String child, String parent, Set<String> visited) {
    if (child == null || parent == null) {
      return false;
    }

    if (child.equals(parent)) {
      return true;
    }

    if (!visited.add(child)) {
      return false;
    }

    StructClass cls = context.getClass(child);
    if (cls != null) {
      if (cls.superClass != null && isSubtype(cls.superClass.getString(), parent, visited)) {
        return true;
      }

      for (String interfaceName : cls.getInterfaceNames()) {
        if (isSubtype(interfaceName, parent, visited)) {
          return true;
        }
      }
    }

    return context.instanceOf(child, parent);
  }

  private boolean isReturnOverrideCompatible(VarType childReturn, VarType parentReturn) {
    if (childReturn.equals(parentReturn)) {
      return true;
    }

    if (childReturn.type == CodeType.OBJECT
        && parentReturn.type == CodeType.OBJECT
        && childReturn.arrayDim == 0
        && parentReturn.arrayDim == 0) {
      return isSubtype(childReturn.value, parentReturn.value);
    }

    return false;
  }

  private static boolean canParticipateInOverride(StructMethod method) {
    return canParticipateInSourceVisibleConflict(method) && !method.hasModifier(CodeConstants.ACC_STATIC);
  }

  private static boolean canParticipateInSourceVisibleConflict(StructMethod method) {
    int flags = method.getAccessFlags();
    return !CodeConstants.INIT_NAME.equals(method.getName())
           && !CodeConstants.CLINIT_NAME.equals(method.getName())
           && (flags & CodeConstants.ACC_PRIVATE) == 0;
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

  private static String sourceMethodSignature(StructMethod method) {
    return method.getName() + " " + parameterDescriptor(method.getDescriptor());
  }

  private static String parameterDescriptor(String descriptor) {
    int end = descriptor.indexOf(')');
    return end >= 0 ? descriptor.substring(0, end + 1) : descriptor;
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
    if (!visited.add(cl.qualifiedName)) {
      return;
    }

    for (StructField field : cl.getFields()) {
      String currentName = resolveCurrentFieldName(cl.qualifiedName, field);
      fieldsByName.computeIfAbsent(currentName, key -> new ArrayList<>()).add(new FieldReference(cl, field, currentName));
    }

    if (cl.superClass != null) {
      StructClass parent = context.getClass(cl.superClass.getString());
      if (parent != null) {
        collectVisibleFields(parent, visited, fieldsByName);
      }
    }

    for (String ifName : cl.getInterfaceNames()) {
      StructClass parent = context.getClass(ifName);
      if (parent != null) {
        collectVisibleFields(parent, visited, fieldsByName);
      }
    }
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
    private final MethodDescriptor descriptor;
    private final String sourceSignature;
    private final int order;
    private String mappedName;

    private MethodReference(StructClass owner, StructMethod method, int order) {
      this.owner = owner;
      this.method = method;
      this.descriptor = MethodDescriptor.parseDescriptor(method.getDescriptor());
      this.sourceSignature = sourceMethodSignature(method);
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
