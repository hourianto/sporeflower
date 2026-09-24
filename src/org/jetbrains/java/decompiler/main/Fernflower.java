// Copyright 2000-2018 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.main;

import java.io.File;
import java.io.IOException;

import org.jetbrains.java.decompiler.api.plugin.LanguageSpec;
import org.jetbrains.java.decompiler.api.SemanticMappingData;
import org.jetbrains.java.decompiler.api.NamingPlan;
import org.jetbrains.java.decompiler.api.EmittedClass;
import org.jetbrains.java.decompiler.api.SourceMetadata;
import org.jetbrains.java.decompiler.util.token.ClassTextToken;
import org.jetbrains.java.decompiler.api.plugin.Plugin;
import org.jetbrains.java.decompiler.main.ClassesProcessor.ClassNode;
import org.jetbrains.java.decompiler.main.decompiler.OptionParser;
import org.jetbrains.java.decompiler.main.extern.*;
import org.jetbrains.java.decompiler.main.plugins.JarPluginLoader;
import org.jetbrains.java.decompiler.api.plugin.PluginSource;
import org.jetbrains.java.decompiler.main.plugins.PluginSources;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings;
import org.jetbrains.java.decompiler.modules.renamer.ConverterHelper;
import org.jetbrains.java.decompiler.modules.renamer.IdentifierConverter;
import org.jetbrains.java.decompiler.modules.renamer.NamingPlanApplication;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.modules.renamer.Tiny2IdentifierRenamer;
import org.jetbrains.java.decompiler.struct.IDecompiledData;
import org.jetbrains.java.decompiler.main.plugins.PluginContext;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.attr.StructGeneralAttribute;
import org.jetbrains.java.decompiler.util.ClasspathScanner;
import org.jetbrains.java.decompiler.util.TextBuffer;
import org.jetbrains.java.decompiler.util.token.TextTokenDumpVisitor;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

public class Fernflower implements IDecompiledData {
  private final StructContext structContext;
  private final ClassesProcessor classProcessor;
  private final IIdentifierRenamer helper;
  private final IdentifierConverter converter;
  private NamingPlan namingPlan;
  private final NamingPlan preparedNames;
  private final Map<String, EmittedClass> emittedClasses = new ConcurrentHashMap<>();

  public Fernflower(IResultSaver saver, Map<String, Object> customProperties, IFernflowerLogger logger) {
    this(saver, customProperties, logger, null);
  }

  public Fernflower(IResultSaver saver, Map<String, Object> customProperties, IFernflowerLogger logger,
                    SemanticMappingData semanticMappings) {
    this(saver, customProperties, logger, semanticMappings, null);
  }

  public Fernflower(IResultSaver saver, Map<String, Object> customProperties, IFernflowerLogger logger,
                    SemanticMappingData semanticMappings, NamingPlan preparedNames) {
    Map<String, Object> properties = new HashMap<>(IFernflowerPreferences.DEFAULTS);
    if (customProperties != null) {
      for (Map.Entry<String, Object> entry : customProperties.entrySet()) {
        if (entry.getKey().length() == 3) {
          // Short name, reparse to long name
          OptionParser.parseShort("-" + entry.getKey() + "=" + entry.getValue(), properties);
        } else {
          properties.put(entry.getKey(), entry.getValue());
        }
      }
    }

    String level = (String)properties.get(IFernflowerPreferences.LOG_LEVEL);
    if (level != null) {
      try {
        logger.setSeverity(IFernflowerLogger.Severity.valueOf(level.toUpperCase(Locale.ENGLISH)));
      }
      catch (IllegalArgumentException ignore) { }
    }

    structContext = new StructContext(saver, this);
    classProcessor = new ClassesProcessor(structContext);

    String mappingsPath = trimToNull(properties.get(IFernflowerPreferences.MAPPINGS_PATH));
    if (mappingsPath != null && !isOptionEnabled(properties.get(IFernflowerPreferences.RENAME_ENTITIES))) {
      properties.put(IFernflowerPreferences.RENAME_ENTITIES, "1");
      logger.writeMessage("Enabled --rename-members because --mappings-path is set.", IFernflowerLogger.Severity.INFO);
    }

    String preparedPath = trimToNull(properties.get(IFernflowerPreferences.PREPARED_NAMES_PATH));
    if (preparedPath != null) {
      if (preparedNames != null) throw new IllegalArgumentException("Use either preparedNames or --prepared-names-path");
      try { preparedNames = NamingPlan.read(Path.of(preparedPath)); }
      catch (IOException ex) { throw new IllegalArgumentException("Cannot read prepared names: " + preparedPath, ex); }
    }
    if (preparedNames != null) {
      if (mappingsPath != null || trimToNull(properties.get(IFernflowerPreferences.USER_RENAMER_CLASS)) != null) {
        throw new IllegalArgumentException("Prepared names cannot be combined with requested mappings or a custom renamer");
      }
      properties.put(IFernflowerPreferences.RENAME_ENTITIES, "1");
    }
    String defaultPackage = trimToNull(properties.get(IFernflowerPreferences.DEFAULT_PACKAGE));
    if (defaultPackage != null) {
      for (String segment : defaultPackage.split("/", -1)) {
        if (segment.isEmpty() || ConverterHelper.mustRenameForJava(IIdentifierRenamer.Type.ELEMENT_CLASS, segment)) {
          throw new IllegalArgumentException("Invalid --default-package (use a Java package with '/' separators): " + defaultPackage);
        }
      }
      properties.put(IFernflowerPreferences.DEFAULT_PACKAGE, defaultPackage);
    }
    if (isOptionEnabled(properties.get(IFernflowerPreferences.PREPARE_NAMES_ONLY))
        && trimToNull(properties.get(IFernflowerPreferences.NAMING_OUTPUT)) == null) {
      throw new IllegalArgumentException("--prepare-names-only requires --naming-output");
    }
    this.preparedNames = preparedNames;
    if (trimToNull(properties.get(IFernflowerPreferences.SOURCE_METADATA_OUTPUT)) != null
        && trimToNull(properties.get(IFernflowerPreferences.NAMING_OUTPUT)) == null) {
      throw new IllegalArgumentException("--source-metadata-output requires --naming-output");
    }

    PoolInterceptor interceptor = null;
    if (isOptionEnabled(properties.get(IFernflowerPreferences.RENAME_ENTITIES))) {
      helper = preparedNames == null ? loadHelper(properties, logger) : Tiny2IdentifierRenamer.fromPlan(preparedNames);
      interceptor = new PoolInterceptor();
      converter = new IdentifierConverter(structContext, helper, interceptor);
    }
    else {
      helper = null;
      converter = null;
    }

    DecompilerContext context = new DecompilerContext(properties, logger, structContext, classProcessor, interceptor);
    DecompilerContext.setCurrentContext(context);

    String semanticMappingsPath = trimToNull(properties.get(IFernflowerPreferences.SEMANTIC_MAPPINGS_PATH));
    if (semanticMappings != null) {
      DecompilerContext.setProperty(DecompilerContext.SEMANTIC_MAPPINGS, SemanticMappings.fromData(semanticMappings));
    } else if (semanticMappingsPath != null) {
      try {
        SemanticMappings loadedMappings = SemanticMappings.load(Path.of(semanticMappingsPath));
        DecompilerContext.setProperty(DecompilerContext.SEMANTIC_MAPPINGS, loadedMappings);
        logger.writeMessage("Loaded semantic mappings: " + semanticMappingsPath, IFernflowerLogger.Severity.INFO);
      }
      catch (IOException | RuntimeException e) {
        throw new IllegalArgumentException("Cannot load semantic mappings '" + semanticMappingsPath + "'", e);
      }
    }

    PluginContext plugins = structContext.getPluginContext();
    int pluginCount = plugins.findPlugins();

    logger.writeMessage("Loaded " + pluginCount + " plugins", IFernflowerLogger.Severity.INFO);

    plugins.initialize();

    IVariableNamingFactory renamer = plugins.getVariableRenamer();
    if (renamer == null) {
      renamer = new IdentityRenamerFactory();
    }
    if (helper instanceof Tiny2IdentifierRenamer tiny2Renamer) {
      renamer = tiny2Renamer.createVariableNamingFactory(renamer);
    }

    context.renamerFactory = renamer;

    String vendor = System.getProperty("java.vendor", "missing vendor");
    String javaVersion = System.getProperty("java.version", "missing java version");
    String jvmVersion = System.getProperty("java.vm.version", "missing jvm version");
    logger.writeMessage(String.format("JVM info: %s - %s - %s", vendor, javaVersion, jvmVersion), IFernflowerLogger.Severity.INFO);

    if (DecompilerContext.getOption(IFernflowerPreferences.INCLUDE_ENTIRE_CLASSPATH)) {
      ClasspathScanner.addAllClasspath(structContext);
    }
  }

  private static IIdentifierRenamer loadHelper(Map<String, Object> properties, IFernflowerLogger logger) {
    String className = trimToNull(properties.get(IFernflowerPreferences.USER_RENAMER_CLASS));
    if (className != null) {
      try {
        Class<?> renamerClass = Fernflower.class.getClassLoader().loadClass(className);
        return (IIdentifierRenamer) renamerClass.getDeclaredConstructor().newInstance();
      }
      catch (Exception e) {
        logger.writeMessage("Cannot load renamer '" + className + "'", IFernflowerLogger.Severity.WARN, e);
      }
    }

    String mappingsPath = trimToNull(properties.get(IFernflowerPreferences.MAPPINGS_PATH));
    if (mappingsPath != null) {
      String sourceNamespace = trimToNull(properties.get(IFernflowerPreferences.MAPPINGS_SOURCE_NAMESPACE));
      String targetNamespace = trimToNull(properties.get(IFernflowerPreferences.MAPPINGS_TARGET_NAMESPACE));
      try {
        Tiny2IdentifierRenamer renamer = Tiny2IdentifierRenamer.fromFile(Path.of(mappingsPath), sourceNamespace, targetNamespace);
        logger.writeMessage(
          "Loaded Tiny v2 mappings: " + mappingsPath
            + " (classes=" + renamer.classRenameCount()
            + ", fields=" + renamer.fieldRenameCount()
            + ", methods=" + renamer.methodRenameCount()
            + ", params=" + renamer.parameterRenameCount() + ")",
          IFernflowerLogger.Severity.INFO
        );
        return renamer;
      }
      catch (Exception e) {
        logger.writeMessage("Cannot load Tiny mappings '" + mappingsPath + "'", IFernflowerLogger.Severity.WARN, e);
      }
    }

    return new ConverterHelper();
  }

  private static boolean isOptionEnabled(Object value) {
    if (value == null) {
      return false;
    }
    String s = value.toString();
    return "1".equals(s) || "true".equalsIgnoreCase(s);
  }

  private static String trimToNull(Object value) {
    if (value == null) {
      return null;
    }
    String s = value.toString().trim();
    return s.isEmpty() ? null : s;
  }

  public void addSource(IContextSource source) {
    structContext.addSpace(source, true);
  }

  public void addSource(File source) {
    structContext.addSpace(source, true);
  }

  public void addLibrary(IContextSource library) {
    structContext.addSpace(library, false);
  }

  public void addLibrary(File library) {
    structContext.addSpace(library, false);
  }

  public NamingPlan prepareNames() {
    if (namingPlan != null) return namingPlan;
    if (DecompilerContext.getOption(IFernflowerPreferences.BUNDLED_J2ME_API)) {
      structContext.addBundledJ2meApis();
    }
    if (converter != null && preparedNames == null) {
      // Reuse the class loader's ownership recovery, including pre-Java-5 local classes.
      classProcessor.loadClasses();
    }
    namingPlan = converter == null ? NamingPlanApplication.capture(structContext, null) : converter.rename(preparedNames);
    classProcessor.loadClasses();
    String output = trimToNull(DecompilerContext.getProperty(IFernflowerPreferences.NAMING_OUTPUT));
    if (output != null) {
      try { namingPlan.write(Path.of(output)); }
      catch (IOException ex) { throw new IllegalStateException("Cannot write completed names", ex); }
    }
    return namingPlan;
  }

  public void decompileContext() {
    prepareNames();
    if (DecompilerContext.getOption(IFernflowerPreferences.PREPARE_NAMES_ONLY)) return;

    SemanticMappings semanticMappings = DecompilerContext.getContextProperty(DecompilerContext.SEMANTIC_MAPPINGS);
    structContext.saveContext(semanticMappings == null ? List.of() : semanticMappings.syntheticSources());
    String output = trimToNull(DecompilerContext.getProperty(IFernflowerPreferences.SOURCE_METADATA_OUTPUT));
    if (output != null) {
      try {
        Path path = Path.of(output);
        Path names = Path.of((String)DecompilerContext.getProperty(IFernflowerPreferences.NAMING_OUTPUT));
        String relativeNames = path.toAbsolutePath().getParent().relativize(names.toAbsolutePath()).toString();
        new SourceMetadata(DecompilerContext.getOption(IFernflowerPreferences.PRESERVE_CLASS_NAME_STRINGS) ? "original" : "renamed",
          relativeNames, emittedClasses.values().stream().sorted(Comparator.comparing(EmittedClass::name)).toList()).write(path);
      } catch (IOException ex) {
        throw new IllegalStateException("Cannot write emitted class correspondence", ex);
      }
    }
  }

  public void addWhitelist(String prefix) {
    classProcessor.addWhitelist(prefix);
  }

  public void clearContext() {
    structContext.clear();
    DecompilerContext.setCurrentContext(null);
  }

  @Override
  public String getClassEntryName(StructClass cl, String entryName) {
    LanguageSpec spec = PluginContext.getCurrentContext().getLanguageSpec(cl);
    String extension = spec == null ? "java" : spec.extension;

    ClassNode node = classProcessor.getMapRootClasses().get(cl.qualifiedName);
    if (node == null || node.type != ClassNode.Type.ROOT) {
      return null;
    }
    else if (converter != null) {
      return cl.qualifiedName + "." + extension;
    }
    else {
      final int clazzIdx = entryName.lastIndexOf(".class");
      if (clazzIdx == -1) {
        return entryName + "." + extension;
      } else {
        return entryName.substring(0, clazzIdx) + "." + extension;
      }
    }
  }

  @Override
  public void processClass(final StructClass cl) throws IOException {
      classProcessor.processClass(cl); // unhandled exceptions handled later on
  }

  @Override
  public String getClassContent(StructClass cl) {
    TextBuffer buffer = new TextBuffer(ClassesProcessor.AVERAGE_CLASS_SIZE);
    try {
      buffer.append(DecompilerContext.getProperty(IFernflowerPreferences.BANNER).toString());
      classProcessor.writeClass(cl, buffer);

      if (DecompilerContext.getOption(IFernflowerPreferences.DUMP_TEXT_TOKENS)) {
        buffer.visitTokens(TextTokenVisitor.createVisitor(next -> new TextTokenDumpVisitor(next, buffer)));
      } else {
        buffer.visitTokens(TextTokenVisitor.createVisitor());
      }

      Map<Integer, Integer> sourceLines = null;
      if (trimToNull(DecompilerContext.getProperty(IFernflowerPreferences.SOURCE_METADATA_OUTPUT)) != null) {
        sourceLines = new HashMap<>();
        for (var token : buffer.getTokens()) {
          if (token instanceof ClassTextToken && (token.isDeclaration() || token.getLength() == 0)) sourceLines.put(token.getStart(), 0);
        }
      }
      String res = buffer.convertToStringAndAllowDataDiscard(sourceLines);
      if (sourceLines != null) {
        Map<String, Integer> ends = new HashMap<>();
        for (var token : buffer.getTokens()) {
          if (token instanceof ClassTextToken type && !token.isDeclaration() && token.getLength() == 0) {
            ends.put(type.qualifiedName, token.getStart());
          }
        }
        for (var token : buffer.getTokens()) {
          if (token instanceof ClassTextToken type && token.isDeclaration()) {
            ClassNode node = classProcessor.getMapRootClasses().get(type.qualifiedName);
            if (node == null || node.type == ClassNode.Type.LAMBDA) continue;
            int line = sourceLines.get(token.getStart());
            int endLine = sourceLines.get(ends.getOrDefault(type.qualifiedName, token.getStart()));
            emittedClasses.put(type.qualifiedName, new EmittedClass(type.qualifiedName, EmittedClass.Kind.valueOf(node.type.name()),
              node.parent == null ? null : node.parent.classStruct.qualifiedName, node.simpleName, node.enclosingMethod,
              cl.qualifiedName + ".java", line, endLine,
              node.getWrapper() == null ? List.of() : node.getWrapper().getHiddenMembers().stream().sorted().toList()));
          }
        }
      }
      if (res == null) {
        return "$ VF: Unable to decompile class " + cl.qualifiedName;
      }

      return res;
    }
    catch (Throwable t) {
      DecompilerContext.getLogger().writeMessage("Class " + cl.qualifiedName + " couldn't be fully decompiled.", t);
      if (DecompilerContext.getOption(IFernflowerPreferences.DUMP_EXCEPTION_ON_ERROR)) {
        List<String> lines = new ArrayList<>();
        lines.add("/*");
        lines.add("$VF: Unable to decompile class");
        lines.addAll(ClassWriter.getErrorComment());
        ClassWriter.collectErrorLines(t, lines);
        lines.add("*/");
        return String.join(DecompilerContext.getNewLineSeparator(), lines);
      } else {
        return null;
      }
    }
  }

  @Override
  public void releaseClass(StructClass cl) {
    classProcessor.releaseClass(cl);
  }

  static {
    Init.init();
  }
}
