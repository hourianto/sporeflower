package org.jetbrains.java.decompiler;

import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.MemberKey;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

public class SemanticBindingLookupTest extends DecompileRegressionTestBase {
  @Test
  void sharedDeclarationsResolveEachBindingShapeIndependently() throws Exception {
    compileJava8NoDebug(writeSource("sample/Contracts.java", """
      package sample;
      interface Left { int values(int input); int mixed(int input); }
      interface Right { int values(int input); int mixed(int input); }
      abstract class Combined implements Left, Right {
        public abstract int values(int input);
        public abstract int mixed(int input);
      }
      """), outRoot());
    fixture.getDecompiler().addSource(outRoot().toFile());
    Path data = fixture.getTempDir().resolve("bindings.json");
    Files.writeString(data, """
      {
        "scalar_bindings": [
          {"target":{"kind":"return","owner":"sample/Left","name":"values","desc":"(I)I"},"domain":"State"},
          {"target":{"kind":"return","owner":"sample/Right","name":"values","desc":"(I)I"},"domain":"State"},
          {"target":{"kind":"return","owner":"sample/Left","name":"mixed","desc":"(I)I"},"domain":"State"}
        ],
        "return_domain_sources": [
          {"target":{"kind":"return","owner":"sample/Right","name":"mixed","desc":"(I)I"},"source_parameter":0}
        ]
      }
      """);
    SemanticMappings mappings = SemanticMappings.load(data);
    MemberKey values = new MemberKey("sample/Combined", "values", "(I)I");
    MemberKey mixed = new MemberKey("sample/Combined", "mixed", "(I)I");
    assertEquals("State", mappings.returnDomain(values));
    assertNull(mappings.returnDomainSource(values));
    assertNull(mappings.returnDomainSource(mixed));
    assertNull(mappings.returnDomain(mixed), "An explicit contract of another shape must participate in ambiguity");
    assertEquals("State", mappings.returnDomain(values));
  }
}
