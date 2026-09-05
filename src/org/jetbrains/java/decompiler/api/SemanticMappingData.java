package org.jetbrains.java.decompiler.api;

import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;

import java.io.IOException;
import java.io.Reader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * Semantic facts supplied by the toolkit after validating the authored mappings.
 * Member identities use mapped names; parameter indices count declared parameters.
 * JSON is only a transport for subprocesses and the standalone command-line tool.
 */
public record SemanticMappingData(
  List<DomainEntry> domains,
  List<ValueEntry> values,
  List<ScalarBindingEntry> scalarBindings,
  List<ArrayBindingEntry> arrayBindings,
  List<ReturnDomainSourceEntry> returnDomainSources
) {
  private static final Gson JSON = new GsonBuilder()
    .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
    .disableHtmlEscaping().setPrettyPrinting().create();

  public record DomainEntry(String id, String kind) {}
  public record ValueEntry(String domain, long value, String owner, String name, String desc, int access,
                           boolean synthetic, String elementDomain) {}
  public record TargetEntry(String kind, String owner, String name, String desc, Integer index) {}
  public record ScalarBindingEntry(TargetEntry target, String domain) {}
  public record DimensionEntry(int dimension, String domain) {}
  public record ArrayBindingEntry(TargetEntry target, List<DimensionEntry> indexDomains,
                                  List<DimensionEntry> slotDomains, String elementDomain) {}
  public record ReturnDomainSourceEntry(TargetEntry target, int sourceParameter) {}

  public static SemanticMappingData read(Path path) throws IOException {
    try (Reader reader = Files.newBufferedReader(path)) {
      return JSON.fromJson(reader, SemanticMappingData.class);
    }
  }

  public void write(Path path) throws IOException {
    if (path.getParent() != null) Files.createDirectories(path.getParent());
    Files.writeString(path, JSON.toJson(this) + "\n");
  }
}
