package org.jetbrains.java.decompiler.api;

import org.jetbrains.java.decompiler.modules.renamer.Tiny2IdentifierRenamer;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Collectors;
import java.util.Map;
import java.util.TreeMap;

/** Complete declaration names, keyed only by original bytecode identities. */
public record NamingPlan(Map<String, String> classes, Map<Member, String> fields, Map<Member, String> methods,
                         Map<Member, Map<Integer, String>> parameters, Map<String, String> innerNames) {
  public record Member(String owner, String name, String descriptor) { }

  public NamingPlan(Map<String, String> classes, Map<Member, String> fields, Map<Member, String> methods) {
    this(classes, fields, methods, Map.of(), Map.of());
  }

  public NamingPlan {
    classes = Map.copyOf(classes);
    fields = Map.copyOf(fields);
    methods = Map.copyOf(methods);
    parameters = parameters.entrySet().stream().collect(Collectors.toUnmodifiableMap(Map.Entry::getKey, e -> Map.copyOf(e.getValue())));
    innerNames = Map.copyOf(innerNames);
  }

  public static NamingPlan read(Path path) throws IOException {
    return Tiny2IdentifierRenamer.fromFile(path, "official", "named").declarationNames();
  }

  public void write(Path path) throws IOException {
    StringBuilder text = new StringBuilder("tiny\t2\t0\tofficial\tnamed\n\tescaped-names\n");
    for (var entry : new TreeMap<>(innerNames).entrySet()) {
      text.append("\tsporeflower-inner-name:").append(escape(entry.getKey())).append('\t').append(escape(entry.getValue())).append('\n');
    }
    var fieldsByOwner = fields.entrySet().stream().collect(Collectors.groupingBy(e -> e.getKey().owner));
    var methodsByOwner = methods.entrySet().stream().collect(Collectors.groupingBy(e -> e.getKey().owner));
    for (var entry : new TreeMap<>(classes).entrySet()) {
      text.append("c\t").append(escape(entry.getKey())).append('\t').append(escape(entry.getValue())).append('\n');
      appendMembers(text, "f", fieldsByOwner.getOrDefault(entry.getKey(), List.of()));
      appendMembers(text, "m", methodsByOwner.getOrDefault(entry.getKey(), List.of()));
    }
    if (path.getParent() != null) Files.createDirectories(path.getParent());
    Files.writeString(path, text);
  }

  private void appendMembers(StringBuilder text, String kind, List<Map.Entry<Member, String>> members) {
    members.stream().sorted(Map.Entry.comparingByKey(Comparator.comparing(Member::name).thenComparing(Member::descriptor)))
      .forEach(e -> {
        text.append('\t').append(kind).append('\t').append(escape(e.getKey().descriptor))
          .append('\t').append(escape(e.getKey().name)).append('\t').append(escape(e.getValue())).append('\n');
        if (kind.equals("m")) new TreeMap<>(parameters.getOrDefault(e.getKey(), Map.of())).forEach((slot, name) ->
          text.append("\t\tp\t").append(slot).append("\t\t").append(escape(name)).append('\n'));
      });
  }

  private static String escape(String name) {
    return name.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r").replace("\0", "\\0");
  }
}
