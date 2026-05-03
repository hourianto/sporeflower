package org.jetbrains.java.decompiler.main.decompiler;

import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;

import java.io.File;
import java.nio.file.InvalidPathException;
import java.nio.file.Path;

final class SavePathUtil {
  private SavePathUtil() {
  }

  static File resolveFile(File root, String operation, String... parts) {
    Path resolved = resolve(root.toPath(), operation, parts);
    return resolved == null ? null : resolved.toFile();
  }

  static Path resolve(Path root, String operation, String... parts) {
    Path normalizedRoot = root.toAbsolutePath().normalize();
    Path resolved = normalizedRoot;

    for (String part : parts) {
      if (part == null || part.isEmpty()) {
        continue;
      }

      if (isUnsafePathPart(part)) {
        logInvalid(operation, part);
        return null;
      }

      Path partPath;
      try {
        partPath = Path.of(part);
      } catch (InvalidPathException ex) {
        logInvalid(operation, part);
        return null;
      }

      if (partPath.isAbsolute()) {
        logInvalid(operation, part);
        return null;
      }

      resolved = resolved.resolve(partPath);
    }

    Path normalized = resolved.normalize();
    if (!normalized.startsWith(normalizedRoot)) {
      logInvalid(operation, describe(parts));
      return null;
    }

    return normalized;
  }

  private static boolean isUnsafePathPart(String part) {
    if (part.indexOf('\0') >= 0 || part.indexOf('\\') >= 0) {
      return true;
    }

    return part.length() >= 2 && part.charAt(1) == ':' && Character.isLetter(part.charAt(0));
  }

  private static void logInvalid(String operation, String path) {
    DecompilerContext context = DecompilerContext.getCurrentContext();
    if (context != null) {
      DecompilerContext.getLogger().writeMessage(
        "Skipping " + operation + " with unsafe output path: " + path,
        IFernflowerLogger.Severity.WARN
      );
    }
  }

  private static String describe(String... parts) {
    StringBuilder ret = new StringBuilder();
    for (String part : parts) {
      if (part == null || part.isEmpty()) {
        continue;
      }
      if (ret.length() > 0) {
        ret.append('/');
      }
      ret.append(part);
    }
    return ret.toString();
  }
}
