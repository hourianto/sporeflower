package org.jetbrains.java.decompiler.api;

import java.util.List;

/** A source declaration, before the compiler assigns local/anonymous binary names. */
public record EmittedClass(String name, Kind kind, String parent, String simpleName,
                           String enclosingMethod, String source, int line, int endLine, List<String> hiddenMembers) {
  public enum Kind { ROOT, MEMBER, LOCAL, ANONYMOUS }
}
