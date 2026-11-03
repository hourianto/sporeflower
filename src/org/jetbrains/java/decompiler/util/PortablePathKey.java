package org.jetbrains.java.decompiler.util;

import java.text.Normalizer;
import java.util.Locale;

/** Conservative collision keys for artifacts that may be written on case-insensitive filesystems. */
public final class PortablePathKey {
  private PortablePathKey() { }

  public static String of(String path) {
    // Lowercasing alone misses case equivalents such as long s and final sigma;
    // uppercasing alone misses the shared lowercase form of capital sharp s.
    // The round trip may merge more spellings than a particular filesystem;
    // preserving both artifacts is preferable to relying on host-specific rules.
    String canonical = Normalizer.normalize(path, Normalizer.Form.NFC);
    return Normalizer.normalize(canonical.toLowerCase(Locale.ROOT).toUpperCase(Locale.ROOT).toLowerCase(Locale.ROOT), Normalizer.Form.NFC);
  }
}
