// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.struct;

import org.jetbrains.annotations.Nullable;
import org.jetbrains.java.decompiler.api.J2meApi;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.jetbrains.java.decompiler.main.extern.IFernflowerLogger;
import org.jetbrains.java.decompiler.main.extern.IResultSaver;
import org.jetbrains.java.decompiler.main.plugins.PluginContext;
import org.jetbrains.java.decompiler.struct.gen.generics.GenericMain;
import org.jetbrains.java.decompiler.struct.gen.generics.GenericMethodDescriptor;
import org.jetbrains.java.decompiler.util.DataInputFullStream;

import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.SeekableByteChannel;
import java.nio.file.Files;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.jar.Attributes;
import java.util.jar.Manifest;
import java.util.stream.Collectors;

public class StructContext {
  private static volatile StructClass SENTINEL_CLASS;

  static StructClass getSentinel() {
    if (SENTINEL_CLASS == null) {
      synchronized (StructContext.class) {
        if (SENTINEL_CLASS == null) {
          try (final InputStream stream = StructContext.class.getResourceAsStream("StructContext.class")) {
            byte[] data = stream.readAllBytes();
            SENTINEL_CLASS = StructClass.create(new DataInputFullStream(data), false);
          } catch (final IOException ex) {
            throw new UncheckedIOException(ex);
          }
        }
      }
    }
    return SENTINEL_CLASS;
  }

  private final IResultSaver saver;
  private final IDecompiledData decompiledData;
  private final List<ContextUnit> units = new ArrayList<>();
  private final List<ContextUnit> lazyUnits = new ArrayList<>();
  private final Map<String, StructClass> classes = new ConcurrentHashMap<>();
  private final Map<String, String> badlyPlacedClasses = new ConcurrentHashMap<>(); // original -> corrected
  private final Map<String, ContextUnit> unitsByClassName = new ConcurrentHashMap<>();
  private final Map<String, List<String>> abstractNames = new HashMap<>();
  
  private final PluginContext pluginContext = new PluginContext();

  public StructContext(IResultSaver saver, IDecompiledData decompiledData) {
    this.saver = saver;
    this.decompiledData = decompiledData;
  }

  public @Nullable StructClass getClass(String name) {
    if (name == null) {
      return null;
    }

    final StructClass ret = this.classes.computeIfAbsent(name, key -> {
      // load class from a context unit
      final ContextUnit unitForClass = this.unitsByClassName.get(key);
      if (unitForClass != null) {
        final StructClass clazz = tryLoadClass(unitForClass, key);
        if (clazz != null) {
          return clazz;
        }
      }
      for (final ContextUnit unit : this.lazyUnits) {
        final StructClass clazz = tryLoadClass(unit, key);
        if (clazz != null) {
          return clazz;
        }
      }
      return getSentinel();
    });
    if (ret == getSentinel()) {
      return null;
    } else {
      final var correctedName = this.badlyPlacedClasses.remove(name);
      // Correct the class location
      if (correctedName != null) {
        this.classes.put(correctedName, ret);
      }
      return ret;
    }
  }

  private StructClass tryLoadClass(final ContextUnit unitForClass, final String key) {
    try {
      DecompilerContext.getLogger().writeMessage("Loading Class: " + key + " from " + unitForClass.getName(), IFernflowerLogger.Severity.INFO);
      final byte[] classBytes = unitForClass.getClassBytes(key);
      if (classBytes == null) {
        return null;
      }
      StructClass clazz = StructClass.create(new DataInputFullStream(classBytes), unitForClass.isOwn());
      if (!key.equals(clazz.qualifiedName)) {
        // also place the class in the right key if it's wrong
        this.unitsByClassName.put(clazz.qualifiedName, unitForClass);
        this.badlyPlacedClasses.put(key, clazz.qualifiedName);
      }
      return clazz;
    } catch (final IOException ex) {
      DecompilerContext.getLogger().writeMessage("Failed to read class " + key + " from " + unitForClass.getName(), IFernflowerLogger.Severity.ERROR, ex);
    }

    return null;
  }

  public boolean hasClass(final String name) {
    if (this.unitsByClassName.containsKey(name)) {
      return true;
    }

    for (final ContextUnit unit : this.lazyUnits) {
      try {
        if (unit.hasClass(name)) {
          return true;
        }
      } catch (final IOException ex) {
        DecompilerContext.getLogger().writeMessage("Failed to check if class " + name + " exists in " + unit.getName(), IFernflowerLogger.Severity.ERROR, ex);
      }
    }

    return false;
  }

  public List<StructClass> getOwnClasses() {
    return this.units.stream()
      .filter(ContextUnit::isOwn)
      .flatMap(unit -> unit.getClassNames().stream())
      .map(name -> Objects.requireNonNull(this.getClass(name), () -> "Could not find class " + name))
      .collect(Collectors.toUnmodifiableList());
  }

  public void reloadContext() throws IOException {
    this.classes.clear();
    this.unitsByClassName.clear();
    this.abstractNames.clear();

    final List<ContextUnit> units = List.copyOf(this.units);
    this.units.clear();
    this.lazyUnits.clear();
    for (ContextUnit unit : units) {
      if (unit.isRoot()) {
        unit.clear();
        this.units.add(unit);
        if (unit.isLazy()) {
          this.lazyUnits.add(unit);
        }
        this.initUnit(unit);
      }
    }
  }

  public void saveContext() {
    saveContext(List.of());
  }

  public void saveContext(List<IContextSource.OutputClass> additionalClasses) {
    boolean wroteAdditionalClasses = false;
    for (ContextUnit unit : this.units) {
      if (unit.isOwn()) {
        try {
          boolean includeAdditionalClasses = !wroteAdditionalClasses && unit.isRoot();
          unit.save(this::getClass, includeAdditionalClasses ? additionalClasses : List.of());
          wroteAdditionalClasses |= includeAdditionalClasses;
        } catch (final IOException ex) {
          DecompilerContext.getLogger().writeMessage("Failed to save data for context unit" + unit.getName(), IFernflowerLogger.Severity.ERROR, ex);
        }
      }
    }
  }

  private static boolean isJarFile(File file) {
    if (!file.isFile()) return false;
    String name = file.getName();
    if (name.endsWith(".jar") || name.endsWith(".zip")) return true;
    if (name.endsWith(".class")) return false;
    try (SeekableByteChannel channel = Files.newByteChannel(file.toPath())) {
      long size = channel.size();
      // The EOCD ZIP record has 22+n bytes depending on the length of the comment.
      if (size < 22) return false;
      int bufferSize = (int) Math.min(size & ~3, 1024);
      channel.position(size - bufferSize);
      ByteBuffer buffer = ByteBuffer.allocate(bufferSize).order(ByteOrder.LITTLE_ENDIAN);
      int read = 0;
      while (read < bufferSize) {
        read += channel.read(buffer);
      }
      buffer.flip();
      for (int pos = buffer.limit() - 22; pos >= 0; pos--) {
        if (buffer.getInt(pos) == 0x06054b50) {
          return true;
        }
      }
    } catch (IOException e) {
      DecompilerContext.getLogger().writeMessage("Could not determine if " + file + " contains a JAR file", IFernflowerLogger.Severity.WARN, e);
    }
    return false;
  }

  public void addSpace(File file, boolean isOwn) {
    if (file.isDirectory()) {
      addSpace(new DirectoryContextSource(file), isOwn);
    } else if (isJarFile(file)) {
      // archive
      try {
        addSpace(new JarContextSource(file), isOwn);
      } catch (final IOException ex) {
        final String message = "Invalid archive " + file;
        DecompilerContext.getLogger().writeMessage(message, IFernflowerLogger.Severity.ERROR, ex);
        throw new UncheckedIOException(message, ex);
      }
    } else {
      try {
        addSpace(new SingleFileContextSource(file), isOwn);
      } catch (final IOException ex) {
        final String message = "Invalid file " + file;
        DecompilerContext.getLogger().writeMessage(message, IFernflowerLogger.Severity.ERROR, ex);
        throw new UncheckedIOException(message, ex);
      }
    }
  }

  public void addSpace(final IContextSource source, final boolean isOwn) {
    this.addSpace(source, isOwn, true);
  }

  private void addSpace(final IContextSource source, final boolean isOwn, final boolean isRoot) {
    final ContextUnit unit = new ContextUnit(source, isOwn, isRoot, saver, decompiledData);
    this.units.add(unit);
    if (unit.isLazy()) {
      this.lazyUnits.add(unit);
    }
    initUnit(unit);
  }

  public PluginContext getPluginContext() {
    return this.pluginContext;
  }

  /** Install selected declarations before renaming or type analysis, entirely in memory. */
  public void addBundledJ2meApis() {
    if (units.stream().anyMatch(unit -> unit.getSource() instanceof J2meApi.Resolution)) return;
    try {
      J2meApi.Requirements requirements = new J2meApi.Requirements();
      Map<String, byte[]> inputs = new LinkedHashMap<>();
      for (ContextUnit unit : units) {
        if (!unit.isOwn()) continue;
        requirements.configuration(manifestAttributes(unit).getValue("MicroEdition-Configuration"));
        for (String name : unit.getClassNames()) {
          byte[] bytes = unit.getClassBytes(name);
          inputs.putIfAbsent(name, bytes);
          StructClass own = getClass(name);
          if (own != null) {
            // Field and method types need not have CONSTANT_Class entries.
            own.getFields().forEach(field -> requirements.inspectDescriptor(field.getDescriptor()));
            own.getMethods().forEach(method -> requirements.inspectDescriptor(method.getDescriptor()));
          }
          try {
            requirements.inspectClassNames(bytes);
          } catch (IllegalArgumentException | IndexOutOfBoundsException | NegativeArraySizeException ignored) {
            // Class parsing and recovery remain the engine's responsibility.
          }
        }
      }
      if (!requirements.isJ2me()) return;
      for (var input : inputs.entrySet()) {
        try {
          requirements.addClass(input.getValue());
        } catch (IllegalArgumentException | IndexOutOfBoundsException | NegativeArraySizeException ex) {
          // Preserve references collected before malformed code/metadata. API
          // discovery must not disable the engine's existing recovery paths.
          DecompilerContext.getLogger().writeMessage("Incomplete API references for " + input.getKey(), IFernflowerLogger.Severity.WARN);
        }
      }
      List<J2meApi.Library> libraries = new ArrayList<>(J2meApi.bundled());
      for (ContextUnit unit : units) {
        if (unit.isOwn() || unit.isLazy()) continue;
        Map<String, byte[]> bytes = new LinkedHashMap<>();
        for (String name : unit.getClassNames()) bytes.put(name, unit.getClassBytes(name));
        Attributes attributes = manifestAttributes(unit);
        boolean compileOnly = "compile-only".equals(attributes.getValue("J2ME-Stub-Kind"));
        String fallback = attributes.getValue("J2ME-Stub-Fallback");
        libraries.add(J2meApi.library(unit.getName(), bytes, compileOnly,
          "true".equals(fallback) || !"false".equals(fallback) && compileOnly));
      }
      J2meApi.Resolution api = J2meApi.resolve(requirements, libraries);
      ContextUnit resolved = new ContextUnit(api, false, true, saver, decompiledData);
      // Keep this precedence after reloadContext too. Project classes always win;
      // external classes use the shared resolver's bytecode-compatible definition.
      units.add(0, resolved);
      for (String name : api.classes().keySet()) {
        ContextUnit existing = unitsByClassName.get(name);
        if (existing == null || !existing.isOwn()) {
          unitsByClassName.put(name, resolved);
          classes.remove(name);
        }
      }
      DecompilerContext.getLogger().writeMessage("Loaded bundled J2ME API declarations", IFernflowerLogger.Severity.INFO);
    } catch (IOException ex) {
      throw new UncheckedIOException("Cannot resolve J2ME API declarations", ex);
    }
  }

  private static Attributes manifestAttributes(ContextUnit unit) {
    if (unit.getOtherEntries().stream().noneMatch(entry -> entry.path().equals("META-INF/MANIFEST.MF"))) return new Attributes();
    try (InputStream input = unit.getSource().getInputStream("META-INF/MANIFEST.MF")) {
      return input == null ? new Attributes() : new Manifest(input).getMainAttributes();
    } catch (IOException ex) {
      return new Attributes(); // API declarations remain usable with broken metadata.
    }
  }
  
  private void initUnit(final ContextUnit unit) {
    DecompilerContext.getLogger().writeMessage("Scanning classes from " + unit.getName(), IFernflowerLogger.Severity.INFO);
    boolean isOwn = unit.isOwn();
    for (final String clazz : unit.getClassNames()) {
      final ContextUnit existing = this.unitsByClassName.putIfAbsent(clazz, unit);
      if (existing != null) {
        if (!isOwn || existing.isOwn()) continue;

        if (!this.unitsByClassName.replace(clazz, existing, unit)) continue;
      }

      DecompilerContext.getLogger().writeMessage("    " + clazz, IFernflowerLogger.Severity.TRACE);
      if (isOwn) { // pre-load classes
        this.getClass(clazz);
      }
    }

    for (final IContextSource child : unit.getChildContexts()) {
      this.addSpace(child, isOwn, false);
    }
  }

  // return (valclass instanceof reflcass)
  public boolean instanceOf(String valclass, String refclass) {
    if (refclass == null || valclass == null) {
      return false;
    }

    if (valclass.equals(refclass)) {
      return true;
    }

    StructClass cl = this.getClass(valclass);
    // Don't know what we are? We must at least be an object.
    if ((cl == null || cl.superClass == null) && refclass.equals("java/lang/Object")) {
      return true;
    }

    if (cl == null) {
      return false;
    }

    if (cl.superClass != null && this.instanceOf(cl.superClass.getString(), refclass)) {
      return true;
    }

    for (int iface : cl.getInterfaces()) {
      if (this.instanceOf(cl.getPool().getPrimitiveConstant(iface).getString(), refclass)) {
        return true;
      }
    }

    return false;
  }

  // JOIN both classes on the lattice. Find a common ancestor.
  public @Nullable StructClass findCommonAncestor(String classA, String classB) {
    StructClass a = this.getClass(classA);
    StructClass b = this.getClass(classB);

    if (a == null || b == null) {
      return null;
    }

    if (instanceOf(classB, classA)) {
      return a;
    }

    // Iterate through the superclasses and interfaces to find any that can be assigned.
    // Don't return if the returned class is Object, otherwise we may miss out on other opportunities to do better
    // The returned type is essentially arbitrary; extended superclasses are preferred before interfaces.

    if (a.superClass != null) {
      StructClass cl = findCommonAncestor(a.superClass.getString(), classB);
      if (cl != null && !cl.qualifiedName.equals("java/lang/Object")) {
        return cl;
      }
    }

    for (int iface : a.getInterfaces()) {
      StructClass cl = findCommonAncestor(a.getPool().getPrimitiveConstant(iface).getString(), classB);
      if (cl != null && !cl.qualifiedName.equals("java/lang/Object")) {
        return cl;
      }
    }

    return null;
  }

  public void loadAbstractMetadata(String string) {
    for (String line : string.split("\n")) {
      String[] pts = line.split(" ");
      if (pts.length < 4) //class method desc [args...]
        continue;
      GenericMethodDescriptor desc = GenericMain.parseMethodSignature(pts[2]);
      List<String> params = new ArrayList<>();
      for (int x = 0; x < pts.length - 3; x++) {
        for (int y = 0; y < desc.parameterTypes.get(x).stackSize; y++)
            params.add(pts[x+3]);
      }
      this.abstractNames.put(pts[0] + ' '+ pts[1] + ' ' + pts[2], params);
    }
  }

  public String renameAbstractParameter(String className, String methodName, String descriptor, int index, String _default) {
    List<String> params = this.abstractNames.get(className + ' ' + methodName + ' ' + descriptor);
    return params != null && index < params.size() ? params.get(index) : _default;
  }

  public void clear() {
    try {
      this.saver.close();
    } catch (final IOException ex) {
      DecompilerContext.getLogger().writeMessage("Failed to close out result saver", IFernflowerLogger.Severity.ERROR, ex);
    }

    for (final ContextUnit unit : this.units) {
      try {
        unit.close();
      } catch (final Exception ex) {
        DecompilerContext.getLogger().writeMessage("Failed to close context unit " + unit.getName(), IFernflowerLogger.Severity.ERROR, ex);
      }
    }
    this.units.clear();
    this.unitsByClassName.clear();
    this.classes.clear();
  }

}
