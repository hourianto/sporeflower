package org.jetbrains.java.decompiler.api;

import com.google.gson.Gson;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/** Portable companion to generated sources and their completed Tiny names. */
public record SourceMetadata(String classNameStrings, String names, List<EmittedClass> classes) {
  public SourceMetadata {
    if (!"original".equals(classNameStrings) && !"renamed".equals(classNameStrings)) {
      throw new IllegalArgumentException("Unknown class-name string mode: " + classNameStrings);
    }
    if (names == null || names.isBlank()) throw new IllegalArgumentException("Source metadata requires a naming file");
    classes = List.copyOf(classes);
  }

  public static SourceMetadata read(Path path) throws IOException {
    return new Gson().fromJson(Files.readString(path), SourceMetadata.class);
  }

  public void write(Path path) throws IOException {
    if (path.getParent() != null) Files.createDirectories(path.getParent());
    Files.writeString(path, new Gson().toJson(this));
  }
}
