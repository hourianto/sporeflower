package org.jetbrains.java.decompiler.main.plugins;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.HashMap;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

/**
 * Loads an embedded plugin without a nested ZIP filesystem or temporary files.
 */
class InJarClassLoader extends ClassLoader {

  private final Map<String, byte[]> entries = new HashMap<>();

  public InJarClassLoader(InputStream input, ClassLoader parent) throws IOException {
    super(parent);
    try (ZipInputStream zip = new ZipInputStream(input)) {
      ZipEntry entry;
      while ((entry = zip.getNextEntry()) != null) {
        if (!entry.isDirectory()) entries.put(entry.getName(), zip.readAllBytes());
      }
    }
  }

  @Override
  protected Class<?> findClass(String name) throws ClassNotFoundException {

    byte[] bytes = entries.get(name.replace('.', '/') + ".class");
    if (bytes == null) throw new ClassNotFoundException(name);
    return defineClass(name, bytes, 0, bytes.length);
  }

  @Override
  public InputStream getResourceAsStream(String name) {
    byte[] bytes = entries.get(name);
    return bytes == null ? super.getResourceAsStream(name) : new ByteArrayInputStream(bytes);
  }
}
