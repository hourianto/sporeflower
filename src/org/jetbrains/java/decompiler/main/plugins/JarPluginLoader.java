package org.jetbrains.java.decompiler.main.plugins;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.jar.JarFile;

// Finds plugins included in the main Vineflower Jar via Jar-In-Jar
public class JarPluginLoader {
  static List<Class<?>> PLUGIN_CLASSES = new ArrayList<>();

  public static void init() {
    try {
      Path myFile = Paths.get(JarPluginLoader.class.getProtectionDomain().getCodeSource().getLocation().toURI());

      // Ensure we are running out of a file
      if (Files.isRegularFile(myFile) && myFile.getFileName().toString().endsWith(".jar")) {
        // Java 8's ZIP provider cannot open a ZIP inside another ZIP. Read the
        // small embedded plugins into memory, also closing the outer archive.
        try (JarFile archive = new JarFile(myFile.toFile())) {
          var entries = archive.entries();
          while (entries.hasMoreElements()) {
            var entry = entries.nextElement();
            if (entry.isDirectory() || !entry.getName().startsWith("META-INF/plugins/") || !entry.getName().endsWith(".jar")) continue;
            InJarClassLoader loader = new InJarClassLoader(archive.getInputStream(entry), JarPluginLoader.class.getClassLoader());
            try (InputStream descriptor = loader.getResourceAsStream("META-INF/services/org.jetbrains.java.decompiler.api.plugin.Plugin")) {
              if (descriptor == null) continue;
              String pluginClass = new String(descriptor.readAllBytes(), StandardCharsets.UTF_8).trim();
              PLUGIN_CLASSES.add(Class.forName(pluginClass, true, loader));
            }
          }
        }
      }

    } catch (Exception e) {
      // TODO: use decompiler logger, but it's not ready yet. potentially set up a logger defer system?
//      DecompilerContext.getLogger().writeMessage("Couldn't load plugins!", IFernflowerLogger.Severity.INFO, e);
      e.printStackTrace();
    }
  }
}
