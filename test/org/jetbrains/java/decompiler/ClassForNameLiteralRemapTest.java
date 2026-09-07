package org.jetbrains.java.decompiler;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertTrue;

public class ClassForNameLiteralRemapTest extends TinyMappingTestBase {
  @Override
  protected String tinyMappings() {
    return """
tiny\t2\t0\tofficial\tnamed
c\tLoader\tLoaderReadable
\tm\t()Ljava/lang/Class;\tload\tloadAdapterClass
c\ta\tAdapter
""";
  }

  @Test
  public void testClassForNameStringLiteralUsesMappedBinaryName() throws IOException {
    Path source = writeSource("Loader.java", """
public class Loader {
  public static Class load() throws Exception {
    return Class.forName("a");
  }
}

class a {
}
""");

    compileJava8NoDebug(source, outRoot());

    fixture.getDecompiler().addSource(outRoot().toFile());
    fixture.getDecompiler().decompileContext();

    List<Path> javaFiles = listJavaSources(fixture.getTargetDir());
    assertTrue(!javaFiles.isEmpty(), "No decompiled .java files found in " + fixture.getTargetDir());

    String needle = "Class.forName(\"Adapter\")";
    for (Path javaFile : javaFiles) {
      String content = DecompilerTestFixture.getContent(javaFile);
      if (content.contains(needle)) {
        return;
      }
    }

    StringBuilder dump = new StringBuilder();
    for (Path javaFile : javaFiles) {
      dump.append(javaFile).append('\n')
        .append(DecompilerTestFixture.getContent(javaFile))
        .append("\n---\n");
    }
    throw new IOException("Expected literal not found: " + needle + "\n" + dump);
  }
}
