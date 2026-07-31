// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IResultSaver;
import org.jetbrains.java.decompiler.struct.StructClass;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class SemanticMappings {
  public record MemberKey(String owner, String name, String desc) {}
  public record ArraySemantics(Map<Integer, String> indexDomains, String slotDomain) {
    public ArraySemantics {
      indexDomains = Map.copyOf(indexDomains);
    }

    public boolean isEmpty() {
      return indexDomains.isEmpty() && slotDomain == null;
    }

    public ArraySemantics element() {
      Map<Integer, String> shifted = new LinkedHashMap<>();
      indexDomains.forEach((dimension, domain) -> {
        if (dimension > 0) shifted.put(dimension - 1, domain);
      });
      return new ArraySemantics(shifted, null);
    }
  }
  private record IndexKey(MemberKey field, int dimension) {}
  public record Value(String domain, long value, String owner, String name, String desc, int access,
                      boolean synthetic, String elementDomain) {}

  private final Map<String, String> domainKinds;
  private final Map<String, Map<Long, Value>> values;
  private final Map<MemberKey, String> fieldDomains;
  private final Map<MemberKey, String> returnDomains;
  private final Map<MemberKey, Map<Integer, String>> parameterDomains;
  private final Map<IndexKey, String> indexDomains;
  private final Map<MemberKey, Map<Integer, String>> returnIndexDomains;
  private final Map<MemberKey, Map<Integer, Map<Integer, String>>> parameterIndexDomains;
  private final Map<MemberKey, String> slotDomains;

  private SemanticMappings(
    Map<String, String> domainKinds,
    Map<String, Map<Long, Value>> values,
    Map<MemberKey, String> fieldDomains,
    Map<MemberKey, String> returnDomains,
    Map<MemberKey, Map<Integer, String>> parameterDomains,
    Map<IndexKey, String> indexDomains,
    Map<MemberKey, Map<Integer, String>> returnIndexDomains,
    Map<MemberKey, Map<Integer, Map<Integer, String>>> parameterIndexDomains,
    Map<MemberKey, String> slotDomains
  ) {
    this.domainKinds = domainKinds;
    this.values = values;
    this.fieldDomains = fieldDomains;
    this.returnDomains = returnDomains;
    this.parameterDomains = parameterDomains;
    this.indexDomains = indexDomains;
    this.returnIndexDomains = returnIndexDomains;
    this.parameterIndexDomains = parameterIndexDomains;
    this.slotDomains = slotDomains;
  }

  public static SemanticMappings load(Path path) throws IOException {
    Object rootValue = new JsonParser(Files.readString(path)).parse();
    Map<String, Object> root = object(rootValue, "semantic map root");
    if (number(root.get("version"), "version") != 1L) {
      throw new IOException("Unsupported semantic map version in " + path);
    }
    if (!"named".equals(string(root.get("namespace"), "namespace"))) {
      throw new IOException("Semantic map must use the named namespace: " + path);
    }

    Map<String, String> domainKinds = new LinkedHashMap<>();
    for (Object item : array(root.get("domains"), "domains")) {
      Map<String, Object> entry = object(item, "domain");
      String id = string(entry.get("id"), "domain id");
      String kind = string(entry.get("kind"), "domain kind");
      if (!Set.of("value", "flags", "slots").contains(kind)) {
        throw new IOException("Unsupported semantic domain kind '" + kind + "' for " + id);
      }
      if (domainKinds.putIfAbsent(id, kind) != null) {
        throw new IOException("Duplicate semantic domain: " + id);
      }
    }

    Map<String, Map<Long, Value>> values = new LinkedHashMap<>();
    for (Object item : array(root.get("values"), "values")) {
      Map<String, Object> entry = object(item, "value");
      String domain = string(entry.get("domain"), "value domain");
      requireDomain(domainKinds, domain);
      String elementDomain = nullableString(entry.get("element_domain"));
      if (elementDomain != null) requireDomain(domainKinds, elementDomain);
      Value value = new Value(
        domain,
        number(entry.get("value"), "value"),
        string(entry.get("owner"), "value owner"),
        string(entry.get("name"), "value name"),
        string(entry.get("desc"), "value descriptor"),
        (int)number(entry.get("access"), "value access"),
        bool(entry.get("synthetic"), "value synthetic"),
        elementDomain
      );
      Value previous = values.computeIfAbsent(domain, ignored -> new LinkedHashMap<>()).put(value.value(), value);
      if (previous != null) {
        throw new IOException("Duplicate semantic value " + value.value() + " in domain " + domain);
      }
    }

    Map<MemberKey, String> fields = readFieldBindings(root.get("field_bindings"), "field_bindings", domainKinds);
    Map<MemberKey, String> returns = readFieldBindings(root.get("return_bindings"), "return_bindings", domainKinds);
    Map<MemberKey, String> slots = readFieldBindings(root.get("slot_bindings"), "slot_bindings", domainKinds);

    Map<MemberKey, Map<Integer, String>> parameters = new LinkedHashMap<>();
    for (Object item : array(root.get("parameter_bindings"), "parameter_bindings")) {
      Map<String, Object> entry = object(item, "parameter binding");
      MemberKey method = member(entry);
      int index = (int)number(entry.get("index"), "parameter index");
      String domain = string(entry.get("domain"), "parameter domain");
      requireDomain(domainKinds, domain);
      if (parameters.computeIfAbsent(method, ignored -> new LinkedHashMap<>()).putIfAbsent(index, domain) != null) {
        throw new IOException("Duplicate semantic parameter binding: " + method + " parameter " + index);
      }
    }

    Map<IndexKey, String> indexes = new LinkedHashMap<>();
    for (Object item : array(root.get("index_bindings"), "index_bindings")) {
      Map<String, Object> entry = object(item, "index binding");
      IndexKey key = new IndexKey(member(entry), (int)number(entry.get("dimension"), "array dimension"));
      String domain = string(entry.get("domain"), "index domain");
      requireDomain(domainKinds, domain);
      if (indexes.putIfAbsent(key, domain) != null) {
        throw new IOException("Duplicate semantic index binding: " + key);
      }
    }

    Map<MemberKey, Map<Integer, String>> returnIndexes = new LinkedHashMap<>();
    for (Object item : optionalArray(root.get("return_index_bindings"), "return_index_bindings")) {
      Map<String, Object> entry = object(item, "return index binding");
      MemberKey method = member(entry);
      int dimension = (int)number(entry.get("dimension"), "array dimension");
      String domain = string(entry.get("domain"), "return index domain");
      requireDomain(domainKinds, domain);
      if (returnIndexes.computeIfAbsent(method, ignored -> new LinkedHashMap<>()).putIfAbsent(dimension, domain) != null) {
        throw new IOException("Duplicate semantic return index binding: " + method + " dimension " + dimension);
      }
    }

    Map<MemberKey, Map<Integer, Map<Integer, String>>> parameterIndexes = new LinkedHashMap<>();
    for (Object item : optionalArray(root.get("parameter_index_bindings"), "parameter_index_bindings")) {
      Map<String, Object> entry = object(item, "parameter index binding");
      MemberKey method = member(entry);
      int parameter = (int)number(entry.get("index"), "parameter index");
      int dimension = (int)number(entry.get("dimension"), "array dimension");
      String domain = string(entry.get("domain"), "parameter index domain");
      requireDomain(domainKinds, domain);
      Map<Integer, String> dimensions = parameterIndexes.computeIfAbsent(method, ignored -> new LinkedHashMap<>())
        .computeIfAbsent(parameter, ignored -> new LinkedHashMap<>());
      if (dimensions.putIfAbsent(dimension, domain) != null) {
        throw new IOException("Duplicate semantic parameter index binding: " + method + " parameter " + parameter + " dimension " + dimension);
      }
    }

    return new SemanticMappings(domainKinds, values, fields, returns, parameters, indexes, returnIndexes, parameterIndexes, slots);
  }

  public String fieldDomain(MemberKey field) {
    return inheritedBinding(fieldDomains, field);
  }

  public String returnDomain(MemberKey method) {
    return inheritedBinding(returnDomains, method);
  }

  public String parameterDomain(MemberKey method, int index) {
    MemberKey declaration = inheritedKey(parameterDomains, method);
    return declaration == null ? null : parameterDomains.get(declaration).get(index);
  }

  public ArraySemantics fieldArraySemantics(MemberKey field) {
    MemberKey declaration = inheritedFieldKey(field);
    if (declaration == null) return null;
    Map<Integer, String> dimensions = new LinkedHashMap<>();
    indexDomains.forEach((key, domain) -> {
      if (key.field().equals(declaration)) dimensions.put(key.dimension(), domain);
    });
    ArraySemantics result = new ArraySemantics(dimensions, slotDomains.get(declaration));
    return result.isEmpty() ? null : result;
  }

  public ArraySemantics returnArraySemantics(MemberKey method) {
    MemberKey declaration = inheritedKey(returnIndexDomains, method);
    if (declaration == null) return null;
    ArraySemantics result = new ArraySemantics(returnIndexDomains.get(declaration), null);
    return result.isEmpty() ? null : result;
  }

  public ArraySemantics parameterArraySemantics(MemberKey method, int parameter) {
    MemberKey declaration = inheritedKey(parameterIndexDomains, method);
    if (declaration == null) return null;
    Map<Integer, String> dimensions = parameterIndexDomains.get(declaration).get(parameter);
    if (dimensions == null) return null;
    ArraySemantics result = new ArraySemantics(dimensions, null);
    return result.isEmpty() ? null : result;
  }

  public String domainKind(String domain) {
    return domainKinds.get(domain);
  }

  public Value value(String domain, long literal, String currentOwner) {
    Value value = values.getOrDefault(domain, Map.of()).get(literal);
    return value != null && isAccessible(value, currentOwner) ? value : null;
  }

  public List<Value> expressionValues(String domain, long literal, String currentOwner) {
    Value exact = value(domain, literal, currentOwner);
    if (exact != null) return List.of(exact);
    if (!"flags".equals(domainKind(domain)) || literal == 0) return List.of();

    List<Value> candidates = values.getOrDefault(domain, Map.of()).values().stream()
      .filter(value -> value.value() != 0 && (value.value() & literal) == value.value())
      .filter(value -> isAccessible(value, currentOwner))
      .sorted((left, right) -> {
        int bits = Integer.compare(Long.bitCount(right.value()), Long.bitCount(left.value()));
        return bits != 0 ? bits : Long.compareUnsigned(left.value(), right.value());
      })
      .toList();
    List<Value> result = new ArrayList<>();
    long covered = 0;
    for (Value candidate : candidates) {
      if ((covered | candidate.value()) == covered) continue;
      result.add(candidate);
      covered |= candidate.value();
      if (covered == literal) return result;
    }
    return List.of();
  }

  public void writeSyntheticSources(IResultSaver saver) {
    for (Map.Entry<String, Map<Long, Value>> entry : values.entrySet()) {
      List<Value> synthetic = entry.getValue().values().stream().filter(Value::synthetic).toList();
      if (synthetic.isEmpty()) {
        continue;
      }
      String owner = synthetic.get(0).owner();
      int slash = owner.lastIndexOf('/');
      String packageName = slash < 0 ? "" : owner.substring(0, slash).replace('/', '.');
      String simpleName = slash < 0 ? owner : owner.substring(slash + 1);
      StringBuilder source = new StringBuilder();
      if (!packageName.isEmpty()) {
        source.append("package ").append(packageName).append(";\n\n");
      }
      source.append("// Generated from semantic mappings; not present in the input JAR.\n");
      source.append("public interface ").append(simpleName).append(" {\n");
      synthetic.stream().sorted((a, b) -> a.name().compareTo(b.name())).forEach(value ->
        source.append("   ").append(javaType(value.desc())).append(' ').append(value.name())
          .append(" = ").append(javaLiteral(value)).append(";\n")
      );
      source.append("}\n");
      String outputPath = slash < 0 ? "" : owner.substring(0, slash);
      saver.saveFolder(outputPath);
      saver.saveClassFile("", owner, owner + ".java", source.toString(), null);
    }
  }

  private String inheritedBinding(Map<MemberKey, String> bindings, MemberKey requested) {
    MemberKey key = inheritedKey(bindings, requested);
    return key == null ? null : bindings.get(key);
  }

  private <T> MemberKey inheritedKey(Map<MemberKey, T> bindings, MemberKey requested) {
    if (bindings.containsKey(requested)) {
      return requested;
    }
    return findInHierarchy(bindings, requested, requested.owner(), new HashSet<>());
  }

  private MemberKey inheritedFieldKey(MemberKey requested) {
    if (hasIndexBinding(requested)) {
      return requested;
    }
    return findFieldInHierarchy(requested, requested.owner(), new HashSet<>());
  }

  private <T> MemberKey findInHierarchy(Map<MemberKey, T> bindings, MemberKey requested, String owner, Set<String> seen) {
    if (!seen.add(owner)) return null;
    MemberKey candidate = new MemberKey(owner, requested.name(), requested.desc());
    if (bindings.containsKey(candidate)) return candidate;
    StructClass cl = DecompilerContext.getStructContext().getClass(owner);
    if (cl == null) return null;
    if (cl.superClass != null) {
      MemberKey found = findInHierarchy(bindings, requested, cl.superClass.getString(), seen);
      if (found != null) return found;
    }
    for (String iface : cl.getInterfaceNames()) {
      MemberKey found = findInHierarchy(bindings, requested, iface, seen);
      if (found != null) return found;
    }
    return null;
  }

  private MemberKey findFieldInHierarchy(MemberKey requested, String owner, Set<String> seen) {
    if (!seen.add(owner)) return null;
    MemberKey candidate = new MemberKey(owner, requested.name(), requested.desc());
    if (hasIndexBinding(candidate) || slotDomains.containsKey(candidate)) return candidate;
    StructClass cl = DecompilerContext.getStructContext().getClass(owner);
    if (cl == null) return null;
    if (cl.superClass != null) {
      MemberKey found = findFieldInHierarchy(requested, cl.superClass.getString(), seen);
      if (found != null) return found;
    }
    for (String iface : cl.getInterfaceNames()) {
      MemberKey found = findFieldInHierarchy(requested, iface, seen);
      if (found != null) return found;
    }
    return null;
  }

  private boolean hasIndexBinding(MemberKey field) {
    return indexDomains.keySet().stream().anyMatch(key -> key.field().equals(field));
  }

  private static boolean isAccessible(Value value, String currentOwner) {
    if (value.synthetic() || (value.access() & CodeConstants.ACC_PUBLIC) != 0) return true;
    if ((value.access() & CodeConstants.ACC_PRIVATE) != 0) return value.owner().equals(currentOwner);
    return packageName(value.owner()).equals(packageName(currentOwner));
  }

  private static String packageName(String owner) {
    int slash = owner.lastIndexOf('/');
    return slash < 0 ? "" : owner.substring(0, slash);
  }

  private static String javaType(String desc) {
    return switch (desc) {
      case "B" -> "byte";
      case "S" -> "short";
      case "C" -> "char";
      case "I" -> "int";
      case "J" -> "long";
      default -> throw new IllegalArgumentException("Unsupported semantic constant descriptor: " + desc);
    };
  }

  private static String javaLiteral(Value value) {
    return switch (value.desc()) {
      case "B" -> "(byte)" + value.value();
      case "S" -> "(short)" + value.value();
      case "C" -> "(char)" + value.value();
      case "J" -> value.value() + "L";
      default -> Long.toString(value.value());
    };
  }

  private static Map<MemberKey, String> readFieldBindings(Object value, String label, Map<String, String> domains) throws IOException {
    Map<MemberKey, String> result = new LinkedHashMap<>();
    for (Object item : array(value, label)) {
      Map<String, Object> entry = object(item, label + " entry");
      MemberKey key = member(entry);
      String domain = string(entry.get("domain"), label + " domain");
      requireDomain(domains, domain);
      if (result.putIfAbsent(key, domain) != null) {
        throw new IOException("Duplicate " + label + " binding: " + key);
      }
    }
    return result;
  }

  private static void requireDomain(Map<String, String> domains, String domain) throws IOException {
    if (!domains.containsKey(domain)) throw new IOException("Unknown semantic domain: " + domain);
  }

  private static MemberKey member(Map<String, Object> entry) throws IOException {
    return new MemberKey(
      string(entry.get("owner"), "member owner"),
      string(entry.get("name"), "member name"),
      string(entry.get("desc"), "member descriptor")
    );
  }

  @SuppressWarnings("unchecked")
  private static Map<String, Object> object(Object value, String label) throws IOException {
    if (!(value instanceof Map<?, ?>)) throw new IOException("Expected JSON object for " + label);
    return (Map<String, Object>)value;
  }

  @SuppressWarnings("unchecked")
  private static List<Object> array(Object value, String label) throws IOException {
    if (!(value instanceof List<?>)) throw new IOException("Expected JSON array for " + label);
    return (List<Object>)value;
  }

  private static List<Object> optionalArray(Object value, String label) throws IOException {
    return value == null ? List.of() : array(value, label);
  }

  private static String string(Object value, String label) throws IOException {
    if (!(value instanceof String text)) throw new IOException("Expected JSON string for " + label);
    return text;
  }

  private static String nullableString(Object value) throws IOException {
    return value == null ? null : string(value, "optional string");
  }

  private static long number(Object value, String label) throws IOException {
    if (!(value instanceof Long number)) throw new IOException("Expected JSON integer for " + label);
    return number;
  }

  private static boolean bool(Object value, String label) throws IOException {
    if (!(value instanceof Boolean result)) throw new IOException("Expected JSON boolean for " + label);
    return result;
  }

  private static final class JsonParser {
    private final String text;
    private int offset;

    private JsonParser(String text) {
      this.text = text;
    }

    Object parse() throws IOException {
      Object value = parseValue();
      whitespace();
      if (offset != text.length()) throw error("Trailing JSON content");
      return value;
    }

    private Object parseValue() throws IOException {
      whitespace();
      if (offset >= text.length()) throw error("Unexpected end of JSON");
      return switch (text.charAt(offset)) {
        case '{' -> parseObject();
        case '[' -> parseArray();
        case '"' -> parseString();
        case 't' -> literal("true", Boolean.TRUE);
        case 'f' -> literal("false", Boolean.FALSE);
        case 'n' -> literal("null", null);
        default -> parseNumber();
      };
    }

    private Map<String, Object> parseObject() throws IOException {
      offset++;
      Map<String, Object> result = new LinkedHashMap<>();
      whitespace();
      if (take('}')) return result;
      while (true) {
        whitespace();
        String key = parseString();
        whitespace();
        expect(':');
        result.put(key, parseValue());
        whitespace();
        if (take('}')) return result;
        expect(',');
      }
    }

    private List<Object> parseArray() throws IOException {
      offset++;
      List<Object> result = new ArrayList<>();
      whitespace();
      if (take(']')) return result;
      while (true) {
        result.add(parseValue());
        whitespace();
        if (take(']')) return result;
        expect(',');
      }
    }

    private String parseString() throws IOException {
      expect('"');
      StringBuilder result = new StringBuilder();
      while (offset < text.length()) {
        char c = text.charAt(offset++);
        if (c == '"') return result.toString();
        if (c != '\\') {
          result.append(c);
          continue;
        }
        if (offset >= text.length()) throw error("Unterminated JSON escape");
        char escaped = text.charAt(offset++);
        switch (escaped) {
          case '"', '\\', '/' -> result.append(escaped);
          case 'b' -> result.append('\b');
          case 'f' -> result.append('\f');
          case 'n' -> result.append('\n');
          case 'r' -> result.append('\r');
          case 't' -> result.append('\t');
          case 'u' -> {
            if (offset + 4 > text.length()) throw error("Incomplete Unicode escape");
            try {
              result.append((char)Integer.parseInt(text.substring(offset, offset + 4), 16));
            } catch (NumberFormatException ex) {
              throw error("Invalid Unicode escape");
            }
            offset += 4;
          }
          default -> throw error("Invalid JSON escape");
        }
      }
      throw error("Unterminated JSON string");
    }

    private Long parseNumber() throws IOException {
      int start = offset;
      if (take('-') && offset >= text.length()) throw error("Invalid JSON number");
      while (offset < text.length() && Character.isDigit(text.charAt(offset))) offset++;
      if (start == offset) throw error("Expected JSON value");
      try {
        return Long.parseLong(text.substring(start, offset));
      } catch (NumberFormatException ex) {
        throw error("Invalid JSON integer");
      }
    }

    private Object literal(String literal, Object value) throws IOException {
      if (!text.startsWith(literal, offset)) throw error("Invalid JSON literal");
      offset += literal.length();
      return value;
    }

    private void whitespace() {
      while (offset < text.length() && Character.isWhitespace(text.charAt(offset))) offset++;
    }

    private boolean take(char expected) {
      if (offset < text.length() && text.charAt(offset) == expected) {
        offset++;
        return true;
      }
      return false;
    }

    private void expect(char expected) throws IOException {
      if (!take(expected)) throw error("Expected '" + expected + "'");
    }

    private IOException error(String message) {
      return new IOException(message + " at character " + offset);
    }
  }
}
