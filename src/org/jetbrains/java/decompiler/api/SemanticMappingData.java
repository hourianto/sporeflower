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
  List<ReturnDomainSourceEntry> returnDomainSources,
  List<CallBindingEntry> callBindings,
  List<StringValueEntry> stringValues,
  List<ConditionalBindingEntry> conditionalBindings,
  List<ContainerBindingEntry> containerBindings,
  List<SlotDomainSourceEntry> slotDomainSources
) {
  private static final Gson JSON = new GsonBuilder()
    .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
    .disableHtmlEscaping().setPrettyPrinting().create();

  public record DomainEntry(String id, String kind, List<Long> exclusiveMasks, List<BitFieldEntry> bitFields, NumberFormatEntry format) {}
  public record BitFieldEntry(String domain, int shift, int bits, boolean signed, long selectorMask, long selectorValue) {}
  public record NumberFormatEntry(String kind, int fractionBits) {}
  public record StringValueEntry(String domain, String value, String owner, String name, int access, boolean synthetic) {}
  public record ConditionalBindingEntry(TargetEntry target, int parameter, Long equalsValue, String domain,
                                        Long notEqualsValue, boolean otherwise) {}
  public record SlotDomainSourceEntry(TargetEntry target, int sourceParameter, int slot, int dimension) {}
  public record ContainerBindingEntry(TargetEntry target, String elements, String keys, String values) {}
  public record ValueEntry(String domain, long value, String owner, String name, String desc, int access,
                           boolean synthetic, String elementDomain) {}
  public record TargetEntry(String kind, String owner, String name, String desc, Integer index) {}
  public record ScalarBindingEntry(TargetEntry target, String domain) {}
  public record DimensionEntry(int dimension, String domain) {}
  public record ArrayBindingEntry(TargetEntry target, List<DimensionEntry> indexDomains,
                                  List<DimensionEntry> slotDomains, String elementDomain, List<RecordLayoutEntry> records) {}
  public record RecordLayoutEntry(int dimension, String domain, int stride, int offset, boolean planes) {}
  /** The offset identifies a call instruction in the original containing method, never a generated local name. */
  public record CallBindingEntry(TargetEntry method, int offset, TargetEntry callee, String domain) {}
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
