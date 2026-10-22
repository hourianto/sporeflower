import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Collections;
import org.jetbrains.java.decompiler.api.SourceMetadata;
import org.jetbrains.java.decompiler.api.SemanticMappingData;

/** Compiled for Java 8 and run against the actual shipped engine. */
public class RuntimeProbe {
    public static void main(String[] args) throws Exception {
        Path metadata = Paths.get(args[0], "metadata.json");
        SourceMetadata expected = new SourceMetadata("original", "names.tiny", Collections.emptyList());
        expected.write(metadata);
        SourceMetadata actual = SourceMetadata.read(metadata);
        if (!expected.equals(actual) || expected.hashCode() != actual.hashCode()) {
            throw new AssertionError("Record equality changed during JSON transport");
        }
        try {
            actual.classes().add(null);
            throw new AssertionError("Source metadata lost its immutable class list");
        } catch (UnsupportedOperationException expectedFailure) { }

        Files.write(metadata, "{\"classNameStrings\":\"invalid\",\"names\":\"names.tiny\",\"classes\":[]}".getBytes(StandardCharsets.UTF_8));
        try {
            SourceMetadata.read(metadata);
            throw new AssertionError("Source metadata bypassed constructor validation");
        } catch (IllegalArgumentException expectedFailure) {
            if (!expectedFailure.getMessage().contains("Unknown class-name string mode")) throw expectedFailure;
        }

        Path semantics = Paths.get(args[0], "semantics.json");
        Files.write(semantics, "{\"domains\":[{\"id\":\"test\",\"format\":{\"kind\":\"fixed\",\"fraction_bits\":16}}]}".getBytes(StandardCharsets.UTF_8));
        SemanticMappingData.NumberFormatEntry format = SemanticMappingData.read(semantics).domains().get(0).format();
        if (!"fixed".equals(format.kind()) || format.fractionBits() != 16) {
            throw new AssertionError("Numeric format changed during JSON transport");
        }
        Files.write(semantics, "{\"domains\":[{\"id\":\"test\",\"format\":{\"kind\":\"fixed\",\"fraction_bits\":64}}]}".getBytes(StandardCharsets.UTF_8));
        try {
            SemanticMappingData.read(semantics);
            throw new AssertionError("Numeric format bypassed constructor validation");
        } catch (IllegalArgumentException expectedFailure) {
            if (!expectedFailure.getMessage().contains("Invalid numeric format")) throw expectedFailure;
        }
        System.out.println("Runtime compatibility passed on Java " + System.getProperty("java.specification.version"));
    }
}
