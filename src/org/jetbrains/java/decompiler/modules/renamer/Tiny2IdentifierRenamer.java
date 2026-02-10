package org.jetbrains.java.decompiler.modules.renamer;

import org.jetbrains.annotations.NotNull;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.extern.IIdentifierRenamer;
import org.jetbrains.java.decompiler.main.extern.IVariableNameProvider;
import org.jetbrains.java.decompiler.main.extern.IVariableNamingFactory;
import org.jetbrains.java.decompiler.modules.decompiler.vars.VarVersionPair;
import org.jetbrains.java.decompiler.struct.StructMethod;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.NewClassNameBuilder;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.Pair;
import org.jetbrains.java.decompiler.util.TextUtil;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class Tiny2IdentifierRenamer implements IIdentifierRenamer {
  private final Map<String, String> classRenames;
  private final Map<MemberKey, String> fieldRenames;
  private final Map<MemberKey, String> methodRenames;
  private final Map<MemberKey, Map<Integer, String>> parameterRenames;
  private final int parameterEntryCount;

  private final Map<String, Integer> classRenameAttempts = new HashMap<>();
  private final Map<MemberKey, Integer> fieldRenameAttempts = new HashMap<>();
  private final Map<MemberKey, Integer> methodRenameAttempts = new HashMap<>();

  private Tiny2IdentifierRenamer(
    Map<String, String> classRenames,
    Map<MemberKey, String> fieldRenames,
    Map<MemberKey, String> methodRenames,
    Map<MemberKey, Map<Integer, String>> parameterRenames,
    int parameterEntryCount
  ) {
    this.classRenames = classRenames;
    this.fieldRenames = fieldRenames;
    this.methodRenames = methodRenames;
    this.parameterRenames = parameterRenames;
    this.parameterEntryCount = parameterEntryCount;
  }

  public static Tiny2IdentifierRenamer fromFile(Path mappingPath, String sourceNamespace, String targetNamespace) throws IOException {
    List<String> lines = Files.readAllLines(mappingPath, StandardCharsets.UTF_8);
    if (lines.isEmpty()) {
      throw new IOException("Tiny mapping file is empty: " + mappingPath);
    }

    Header header = parseHeader(lines.get(0), mappingPath);
    int sourceNamespaceIndex = findNamespaceIndex(header.namespaces(), sourceNamespace, 0, "source", mappingPath);
    int targetNamespaceIndex = findNamespaceIndex(header.namespaces(), targetNamespace, 1, "target", mappingPath);
    if (sourceNamespaceIndex == targetNamespaceIndex) {
      throw new IOException("Tiny mapping source and target namespaces resolve to the same index (" + sourceNamespaceIndex + ") in " + mappingPath);
    }

    boolean escapedNames = hasEscapedNamesProperty(lines);

    Map<String, String> classRenames = collectClassRenames(lines, mappingPath, header, sourceNamespaceIndex, targetNamespaceIndex, escapedNames);
    ParsedMembers parsed = parseMembers(lines, mappingPath, header, sourceNamespaceIndex, targetNamespaceIndex, escapedNames, classRenames);

    return new Tiny2IdentifierRenamer(
      Collections.unmodifiableMap(classRenames),
      Collections.unmodifiableMap(parsed.fieldRenames()),
      Collections.unmodifiableMap(parsed.methodRenames()),
      freezeParameterMap(parsed.parameterRenames()),
      parsed.parameterEntryCount()
    );
  }

  public int classRenameCount() {
    return classRenames.size();
  }

  public int fieldRenameCount() {
    return fieldRenames.size();
  }

  public int methodRenameCount() {
    return methodRenames.size();
  }

  public int parameterRenameCount() {
    return parameterEntryCount;
  }

  public String getParameterRename(String owner, String methodName, String descriptor, int localVariableIndex) {
    Map<Integer, String> methodParams = parameterRenames.get(new MemberKey(owner, methodName, descriptor));
    if (methodParams == null) {
      return null;
    }
    return methodParams.get(localVariableIndex);
  }

  public IVariableNamingFactory createVariableNamingFactory(IVariableNamingFactory delegateFactory) {
    return new Tiny2ParameterNameFactory(parameterRenames, delegateFactory);
  }

  @Override
  public boolean toBeRenamed(Type elementType, String className, String element, String descriptor) {
    if (className == null) {
      return false;
    }

    return switch (elementType) {
      case ELEMENT_CLASS -> classRenames.containsKey(className);
      case ELEMENT_FIELD -> element != null && descriptor != null && fieldRenames.containsKey(new MemberKey(className, element, descriptor));
      case ELEMENT_METHOD -> element != null
        && descriptor != null
        && !CodeConstants.INIT_NAME.equals(element)
        && !CodeConstants.CLINIT_NAME.equals(element)
        && methodRenames.containsKey(new MemberKey(className, element, descriptor));
    };
  }

  @Override
  public String getNextClassName(String fullName, String shortName) {
    String mapped = classRenames.get(fullName);
    if (mapped == null) {
      return shortName != null ? shortName : fullName;
    }
    return withRetrySuffix(mapped, classRenameAttempts.merge(fullName, 1, Integer::sum));
  }

  @Override
  public String getNextFieldName(String className, String field, String descriptor) {
    MemberKey key = new MemberKey(className, field, descriptor);
    String mapped = fieldRenames.get(key);
    if (mapped == null) {
      return field;
    }
    return withRetrySuffix(mapped, fieldRenameAttempts.merge(key, 1, Integer::sum));
  }

  @Override
  public String getNextMethodName(String className, String method, String descriptor) {
    MemberKey key = new MemberKey(className, method, descriptor);
    String mapped = methodRenames.get(key);
    if (mapped == null) {
      return method;
    }
    return withRetrySuffix(mapped, methodRenameAttempts.merge(key, 1, Integer::sum));
  }

  private static String withRetrySuffix(String base, int attempt) {
    if (attempt <= 1) {
      return base;
    }
    return base + "_" + (attempt - 1);
  }

  private static Map<String, String> collectClassRenames(
    List<String> lines,
    Path mappingPath,
    Header header,
    int sourceNamespaceIndex,
    int targetNamespaceIndex,
    boolean escapedNames
  ) throws IOException {
    Map<String, String> classRenames = new LinkedHashMap<>();

    for (int lineNo = 2; lineNo <= lines.size(); lineNo++) {
      String line = lines.get(lineNo - 1);
      int indent = countLeadingTabs(line);
      String body = line.substring(indent);
      if (indent != 0 || body.isBlank() || body.startsWith("#")) {
        continue;
      }

      String[] columns = body.split("\t", -1);
      if (!"c".equals(columns[0])) {
        continue;
      }

      ensureColumns(columns, 1 + header.namespaces().size(), mappingPath, lineNo, "class");
      String fromName = decodeTinyString(columns[1 + sourceNamespaceIndex], escapedNames, mappingPath, lineNo, "class source name");
      String toName = decodeTinyString(columns[1 + targetNamespaceIndex], escapedNames, mappingPath, lineNo, "class target name");
      addRename(classRenames, fromName, fromName, toName, mappingPath, lineNo, "class");
    }

    return classRenames;
  }

  private static ParsedMembers parseMembers(
    List<String> lines,
    Path mappingPath,
    Header header,
    int sourceNamespaceIndex,
    int targetNamespaceIndex,
    boolean escapedNames,
    Map<String, String> classRenames
  ) throws IOException {
    Map<MemberKey, String> fieldRenames = new LinkedHashMap<>();
    Map<MemberKey, String> methodRenames = new LinkedHashMap<>();
    Map<MemberKey, Map<Integer, String>> parameterRenames = new LinkedHashMap<>();
    int parameterEntryCount = 0;

    String currentClass = null;
    MemberKey currentMethodSource = null;
    List<MemberKey> currentMethodPhaseKeys = Collections.emptyList();

    for (int lineNo = 2; lineNo <= lines.size(); lineNo++) {
      String line = lines.get(lineNo - 1);
      int indent = countLeadingTabs(line);
      String body = line.substring(indent);
      if (body.isBlank() || body.startsWith("#")) {
        continue;
      }

      String[] columns = body.split("\t", -1);
      String kind = columns[0];

      if (indent == 0) {
        currentMethodSource = null;
        currentMethodPhaseKeys = Collections.emptyList();

        if (!"c".equals(kind)) {
          currentClass = null;
          continue;
        }

        ensureColumns(columns, 1 + header.namespaces().size(), mappingPath, lineNo, "class");
        String fromName = decodeTinyString(columns[1 + sourceNamespaceIndex], escapedNames, mappingPath, lineNo, "class source name");
        currentClass = fromName.isEmpty() ? null : fromName;
        continue;
      }

      if (indent == 1) {
        currentMethodSource = null;
        currentMethodPhaseKeys = Collections.emptyList();

        if (currentClass == null) {
          continue;
        }

        if ("f".equals(kind)) {
          ensureColumns(columns, 2 + header.namespaces().size(), mappingPath, lineNo, "field");
          String descriptor = decodeTinyString(columns[1], escapedNames, mappingPath, lineNo, "field descriptor");
          String fromName = decodeTinyString(columns[2 + sourceNamespaceIndex], escapedNames, mappingPath, lineNo, "field source name");
          String toName = decodeTinyString(columns[2 + targetNamespaceIndex], escapedNames, mappingPath, lineNo, "field target name");
          addRename(fieldRenames, new MemberKey(currentClass, fromName, descriptor), fromName, toName, mappingPath, lineNo, "field");
        }
        else if ("m".equals(kind)) {
          ensureColumns(columns, 2 + header.namespaces().size(), mappingPath, lineNo, "method");
          String descriptor = decodeTinyString(columns[1], escapedNames, mappingPath, lineNo, "method descriptor");
          String fromName = decodeTinyString(columns[2 + sourceNamespaceIndex], escapedNames, mappingPath, lineNo, "method source name");
          String toName = decodeTinyString(columns[2 + targetNamespaceIndex], escapedNames, mappingPath, lineNo, "method target name");

          currentMethodSource = new MemberKey(currentClass, fromName, descriptor);
          MemberKey currentMethodTarget = toTargetMethodKey(currentMethodSource, toName, classRenames);
          currentMethodPhaseKeys = buildMethodPhaseKeys(currentMethodSource, currentMethodTarget);

          addRename(methodRenames, currentMethodSource, fromName, toName, mappingPath, lineNo, "method");
        }

        continue;
      }

      if (indent == 2 && "p".equals(kind) && currentMethodSource != null) {
        ensureColumns(columns, 2 + header.namespaces().size(), mappingPath, lineNo, "parameter");

        int lvIndex = parseLocalVariableIndex(columns[1], mappingPath, lineNo);
        String fromName = decodeTinyString(columns[2 + sourceNamespaceIndex], escapedNames, mappingPath, lineNo, "parameter source name");
        String toName = decodeTinyString(columns[2 + targetNamespaceIndex], escapedNames, mappingPath, lineNo, "parameter target name");

        boolean added = addParameterRename(parameterRenames, currentMethodSource, lvIndex, fromName, toName, mappingPath, lineNo);
        for (MemberKey phaseKey : currentMethodPhaseKeys) {
          if (!phaseKey.equals(currentMethodSource)) {
            addParameterRename(parameterRenames, phaseKey, lvIndex, fromName, toName, mappingPath, lineNo);
          }
        }

        if (added) {
          parameterEntryCount++;
        }
      }
    }

    return new ParsedMembers(fieldRenames, methodRenames, parameterRenames, parameterEntryCount);
  }

  private static int parseLocalVariableIndex(String rawValue, Path mappingPath, int lineNo) throws IOException {
    final int lvIndex;
    try {
      lvIndex = Integer.parseInt(rawValue);
    }
    catch (NumberFormatException ex) {
      throw new IOException("Invalid Tiny parameter local variable index at " + mappingPath + ":" + lineNo + ": '" + rawValue + "'", ex);
    }

    if (lvIndex < 0) {
      throw new IOException("Invalid Tiny parameter local variable index at " + mappingPath + ":" + lineNo + ": must be non-negative");
    }

    return lvIndex;
  }

  private static MemberKey toTargetMethodKey(MemberKey source, String targetName, Map<String, String> classRenames) {
    String mappedOwner = classRenames.getOrDefault(source.owner(), source.owner());
    String mappedDescriptor = remapMethodDescriptor(source.descriptor(), classRenames);
    return new MemberKey(mappedOwner, targetName, mappedDescriptor);
  }

  private static List<MemberKey> buildMethodPhaseKeys(MemberKey source, MemberKey target) {
    LinkedHashSet<MemberKey> keys = new LinkedHashSet<>();

    String[] owners = {source.owner(), target.owner()};
    String[] names = {source.name(), target.name()};
    String[] descriptors = {source.descriptor(), target.descriptor()};

    for (String owner : owners) {
      for (String name : names) {
        for (String descriptor : descriptors) {
          keys.add(new MemberKey(owner, name, descriptor));
        }
      }
    }

    return List.copyOf(keys);
  }

  private static Header parseHeader(String line, Path mappingPath) throws IOException {
    String normalized = line.stripLeading();
    if (!normalized.isEmpty() && normalized.charAt(0) == '\uFEFF') {
      normalized = normalized.substring(1);
    }

    String[] columns = normalized.split("\t", -1);
    if (columns.length < 5 || !"tiny".equals(columns[0]) || !"2".equals(columns[1])) {
      throw new IOException("Unsupported Tiny header in " + mappingPath + ": expected Tiny v2");
    }

    String[] namespaces = Arrays.copyOfRange(columns, 3, columns.length);
    if (namespaces.length < 2) {
      throw new IOException("Tiny mapping requires at least two namespaces: " + mappingPath);
    }

    return new Header(Arrays.asList(namespaces));
  }

  private static boolean hasEscapedNamesProperty(List<String> lines) {
    for (int lineNo = 2; lineNo <= lines.size(); lineNo++) {
      String line = lines.get(lineNo - 1);
      int indent = countLeadingTabs(line);
      String body = line.substring(indent);

      if (body.isBlank() || body.startsWith("#")) {
        continue;
      }

      if (indent == 0) {
        break;
      }

      if (indent == 1) {
        String[] columns = body.split("\t", -1);
        if (columns.length >= 1 && "escaped-names".equals(columns[0])) {
          return true;
        }
      }
    }

    return false;
  }

  private static int findNamespaceIndex(List<String> namespaces, String requestedNamespace, int fallback, String role, Path mappingPath) throws IOException {
    String requested = requestedNamespace == null ? "" : requestedNamespace.trim();
    if (!requested.isEmpty()) {
      int index = namespaces.indexOf(requested);
      if (index < 0) {
        throw new IOException("Unknown Tiny " + role + " namespace '" + requested + "' in " + mappingPath + ". Available: " + namespaces);
      }
      return index;
    }

    if (fallback >= namespaces.size()) {
      throw new IOException("Tiny mapping in " + mappingPath + " does not provide enough namespaces for the default " + role + " namespace");
    }

    return fallback;
  }

  private static int countLeadingTabs(String line) {
    int tabs = 0;
    while (tabs < line.length() && line.charAt(tabs) == '\t') {
      tabs++;
    }
    return tabs;
  }

  private static void ensureColumns(String[] columns, int minimum, Path mappingPath, int lineNo, String itemType) throws IOException {
    if (columns.length < minimum) {
      throw new IOException("Invalid Tiny " + itemType + " entry at " + mappingPath + ":" + lineNo + ": expected at least " + minimum + " columns");
    }
  }

  private static <K> void addRename(
    Map<K, String> map,
    K key,
    String sourceName,
    String targetName,
    Path mappingPath,
    int lineNo,
    String itemType
  ) throws IOException {
    if (targetName == null || targetName.isEmpty() || key == null || sourceName == null || sourceName.isEmpty() || sourceName.equals(targetName)) {
      return;
    }

    String existing = map.putIfAbsent(key, targetName);
    if (existing != null && !existing.equals(targetName)) {
      throw new IOException(
        "Conflicting Tiny " + itemType + " rename for " + key + " at " + mappingPath + ":" + lineNo
          + ": existing '" + existing + "', new '" + targetName + "'"
      );
    }
  }

  private static boolean addParameterRename(
    Map<MemberKey, Map<Integer, String>> map,
    MemberKey methodKey,
    int localVariableIndex,
    String sourceName,
    String targetName,
    Path mappingPath,
    int lineNo
  ) throws IOException {
    if (methodKey == null || targetName == null || targetName.isEmpty() || sourceName == null || sourceName.equals(targetName)) {
      return false;
    }

    Map<Integer, String> namesByIndex = map.computeIfAbsent(methodKey, k -> new LinkedHashMap<>());
    String existing = namesByIndex.putIfAbsent(localVariableIndex, targetName);
    if (existing != null && !existing.equals(targetName)) {
      throw new IOException(
        "Conflicting Tiny parameter rename for " + methodKey + " lv=" + localVariableIndex + " at " + mappingPath + ":" + lineNo
          + ": existing '" + existing + "', new '" + targetName + "'"
      );
    }
    return existing == null;
  }

  private static Map<MemberKey, Map<Integer, String>> freezeParameterMap(Map<MemberKey, Map<Integer, String>> map) {
    Map<MemberKey, Map<Integer, String>> frozen = new LinkedHashMap<>();
    for (Map.Entry<MemberKey, Map<Integer, String>> entry : map.entrySet()) {
      frozen.put(entry.getKey(), Collections.unmodifiableMap(new LinkedHashMap<>(entry.getValue())));
    }
    return Collections.unmodifiableMap(frozen);
  }

  private static String remapMethodDescriptor(String descriptor, Map<String, String> classMap) {
    try {
      MethodDescriptor parsed = MethodDescriptor.parseDescriptor(descriptor);
      NewClassNameBuilder builder = classMap::get;
      String rebuilt = parsed.buildNewDescriptor(builder);
      return rebuilt != null ? rebuilt : descriptor;
    }
    catch (RuntimeException ex) {
      return descriptor;
    }
  }

  private static String decodeTinyString(String value, boolean escapedNames, Path mappingPath, int lineNo, String context) throws IOException {
    if (!escapedNames || value.indexOf('\\') < 0) {
      return value;
    }

    StringBuilder sb = new StringBuilder(value.length());
    boolean escaped = false;

    for (int i = 0; i < value.length(); i++) {
      char c = value.charAt(i);
      if (!escaped) {
        if (c == '\\') {
          escaped = true;
        }
        else {
          sb.append(c);
        }
        continue;
      }

      switch (c) {
        case 'n' -> sb.append('\n');
        case 'r' -> sb.append('\r');
        case 't' -> sb.append('\t');
        case '0' -> sb.append('\0');
        case '\\' -> sb.append('\\');
        default -> throw new IOException(
          "Invalid Tiny escape sequence '\\" + c + "' in " + context + " at " + mappingPath + ":" + lineNo
        );
      }

      escaped = false;
    }

    if (escaped) {
      throw new IOException("Dangling Tiny escape in " + context + " at " + mappingPath + ":" + lineNo);
    }

    return sb.toString();
  }

  private static String uniqueName(String base, Set<String> used) {
    if (used.add(base)) {
      return base;
    }

    int suffix = 1;
    while (true) {
      String candidate = base + "_" + suffix;
      if (used.add(candidate)) {
        return candidate;
      }
      suffix++;
    }
  }

  static String resolveMappedParameterName(int flags, String currentName, String mappedName) {
    if (mappedName == null) {
      return null;
    }

    // Abstract/native methods do not have method bodies, so there is no
    // var-collision pass to preserve and we should always apply the mapping.
    if ((flags & (CodeConstants.ACC_ABSTRACT | CodeConstants.ACC_NATIVE)) != 0) {
      return mappedName;
    }

    // For concrete methods, ClassWriter passes the already-resolved local
    // name. If it is a collision-adjusted form of the mapped name (e.g.
    // "pausedx"), preserve it so declarations match body usages.
    if (isCollisionAdjustedVariant(currentName, mappedName)) {
      return currentName;
    }

    return mappedName;
  }

  static boolean isCollisionAdjustedVariant(String currentName, String mappedName) {
    if (currentName == null || mappedName == null) {
      return false;
    }
    if (currentName.equals(mappedName)) {
      return true;
    }
    if (!currentName.startsWith(mappedName)) {
      return false;
    }

    String suffix = currentName.substring(mappedName.length());
    if (suffix.isEmpty()) {
      return true;
    }
    if (suffix.chars().allMatch(ch -> ch == 'x')) {
      return true;
    }
    if (suffix.length() > 1 && suffix.charAt(0) == '_') {
      for (int i = 1; i < suffix.length(); i++) {
        if (!Character.isDigit(suffix.charAt(i))) {
          return false;
        }
      }
      return true;
    }

    return false;
  }

  private record Header(List<String> namespaces) { }

  private record MemberKey(String owner, String name, String descriptor) {
    @Override
    public String toString() {
      return owner + "." + name + descriptor;
    }
  }

  private record ParsedMembers(
    Map<MemberKey, String> fieldRenames,
    Map<MemberKey, String> methodRenames,
    Map<MemberKey, Map<Integer, String>> parameterRenames,
    int parameterEntryCount
  ) { }

  private static final class Tiny2ParameterNameFactory implements IVariableNamingFactory {
    private final Map<MemberKey, Map<Integer, String>> parameterRenames;
    private final IVariableNamingFactory delegateFactory;

    private Tiny2ParameterNameFactory(Map<MemberKey, Map<Integer, String>> parameterRenames, IVariableNamingFactory delegateFactory) {
      this.parameterRenames = parameterRenames;
      this.delegateFactory = delegateFactory;
    }

    @Override
    public @NotNull IVariableNameProvider createFactory(StructMethod structMethod) {
      MemberKey methodKey = new MemberKey(structMethod.getClassQualifiedName(), structMethod.getName(), structMethod.getDescriptor());
      Map<Integer, String> names = parameterRenames.getOrDefault(methodKey, Collections.emptyMap());
      IVariableNameProvider delegate = delegateFactory != null ? delegateFactory.createFactory(structMethod) : null;
      return new Tiny2ParameterNameProvider(structMethod, names, delegate);
    }
  }

  private static final class Tiny2ParameterNameProvider implements IVariableNameProvider {
    private final StructMethod method;
    private final Map<Integer, String> parameterNames;
    private final IVariableNameProvider delegate;

    private Tiny2ParameterNameProvider(StructMethod method, Map<Integer, String> parameterNames, IVariableNameProvider delegate) {
      this.method = method;
      this.parameterNames = normalizeParameterNames(parameterNames);
      this.delegate = delegate;
    }

    @Override
    public Map<VarVersionPair, String> rename(Map<VarVersionPair, Pair<VarType, String>> variables) {
      Map<VarVersionPair, String> delegateRenames = delegate != null ? delegate.rename(variables) : null;
      if (parameterNames.isEmpty()) {
        return delegateRenames;
      }

      Map<VarVersionPair, String> result = new LinkedHashMap<>();
      if (delegateRenames != null) {
        result.putAll(delegateRenames);
      }

      Set<String> used = new HashSet<>(result.values());
      Map<Integer, String> assigned = new HashMap<>();
      List<VarVersionPair> keys = new ArrayList<>(variables.keySet());
      keys.sort(Comparator.comparingInt((VarVersionPair v) -> v.var).thenComparingInt(v -> v.version));

      for (VarVersionPair key : keys) {
        String base = parameterNames.get(key.var);
        if (base == null) {
          continue;
        }

        String name = assigned.get(key.var);
        if (name == null) {
          name = uniqueName(base, used);
          assigned.put(key.var, name);
        }

        result.put(key, name);
      }

      return result;
    }

    @Override
    public String renameAbstractParameter(String name, int index) {
      String resolved = resolveParameterName(CodeConstants.ACC_ABSTRACT, name, index);
      if (resolved != null) {
        return resolved;
      }
      if (delegate != null) {
        return delegate.renameAbstractParameter(name, index);
      }
      return IVariableNameProvider.super.renameAbstractParameter(name, index);
    }

    @Override
    public String renameParameter(int flags, VarType type, String name, int index) {
      String resolved = resolveParameterName(flags, name, index);
      if (resolved != null) {
        return resolved;
      }
      if (delegate != null) {
        return delegate.renameParameter(flags, type, name, index);
      }
      return IVariableNameProvider.super.renameParameter(flags, type, name, index);
    }

    @Override
    public void addParentContext(IVariableNameProvider renamer) {
      if (delegate == null) {
        return;
      }

      if (renamer instanceof Tiny2ParameterNameProvider tinyParent && tinyParent.delegate != null) {
        delegate.addParentContext(tinyParent.delegate);
      }
      else {
        delegate.addParentContext(renamer);
      }
    }

    private String resolveParameterName(int flags, String currentName, int index) {
      String mapped = parameterNames.get(index);
      if (mapped == null) {
        return null;
      }
      return resolveMappedParameterName(flags, currentName, mapped);
    }

    private Map<Integer, String> normalizeParameterNames(Map<Integer, String> rawNames) {
      if (rawNames.isEmpty()) {
        return Collections.emptyMap();
      }

      Map<Integer, String> normalized = new LinkedHashMap<>();
      Set<String> used = new HashSet<>();
      List<Integer> indices = new ArrayList<>(rawNames.keySet());
      Collections.sort(indices);

      for (Integer index : indices) {
        String name = sanitizeIdentifier(rawNames.get(index), index);
        name = uniqueName(name, used);
        normalized.put(index, name);
      }

      return Collections.unmodifiableMap(normalized);
    }

    private String sanitizeIdentifier(String rawName, int index) {
      if (rawName == null || rawName.isEmpty()) {
        return "param" + index;
      }

      StringBuilder sb = new StringBuilder(rawName.length() + 1);
      char first = rawName.charAt(0);
      if (!Character.isJavaIdentifierStart(first)) {
        sb.append('_');
      }

      for (int i = 0; i < rawName.length(); i++) {
        char c = rawName.charAt(i);
        sb.append(Character.isJavaIdentifierPart(c) ? c : '_');
      }

      String candidate = sb.toString();
      if (candidate.isEmpty()) {
        candidate = "param" + index;
      }

      if (!TextUtil.isValidIdentifier(candidate, method.getBytecodeVersion(), method)) {
        if (TextUtil.isKeyword(candidate, method.getBytecodeVersion(), method)) {
          candidate = candidate + "_";
        }
        if (!TextUtil.isValidIdentifier(candidate, method.getBytecodeVersion(), method)) {
          candidate = "param" + index;
        }
      }

      return candidate;
    }
  }
}
