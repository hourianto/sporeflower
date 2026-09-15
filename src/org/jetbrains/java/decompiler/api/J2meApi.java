package org.jetbrains.java.decompiler.api;

import org.jetbrains.java.decompiler.main.extern.IContextSource;
import org.objectweb.asm.*;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.nio.file.attribute.FileTime;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.jar.JarFile;

/** Shared API selection for standalone decompilation, toolkit mappings and compile checks. */
public final class J2meApi {
  public static final String RESOURCE_ROOT = "/META-INF/j2me-api/";

  private J2meApi() {}

  private record Member(String name, String descriptor, boolean field, Boolean isStatic) {
    Member withStatic(Boolean value) { return new Member(name, descriptor, field, value); }
  }

  private record Definition(byte[] bytes, List<String> parents, Set<Member> members) {
    boolean declares(Member member) {
      return member.isStatic == null
        ? members.contains(member.withStatic(false)) || members.contains(member.withStatic(true))
        : members.contains(member);
    }
  }

  public static final class Library {
    private final String name;
    private final Map<String, Definition> classes = new LinkedHashMap<>();
    private final boolean compileOnly;
    private final boolean fallback;

    private Library(String name, boolean compileOnly, boolean fallback) {
      this.name = name;
      this.compileOnly = compileOnly;
      this.fallback = fallback;
    }
  }

  private record CachedLibrary(long size, FileTime modified, Library library) {}
  private static final Map<Path, CachedLibrary> LIBRARIES = new ConcurrentHashMap<>();

  private static final class Bundled {
    static final List<Library> LIBRARIES = loadBundled();
  }

  public static List<Library> bundled() { return Bundled.LIBRARIES; }

  private static List<Library> loadBundled() {
    try (InputStream index = J2meApi.class.getResourceAsStream(RESOURCE_ROOT + "index.tsv")) {
      if (index == null) return List.of();
      Map<String, Library> libraries = new LinkedHashMap<>();
      for (String line : new String(index.readAllBytes(), StandardCharsets.UTF_8).lines().toList()) {
        String[] parts = line.split("\t");
        Library library = libraries.computeIfAbsent(parts[0], name -> new Library(name + ".jar", true, Boolean.parseBoolean(parts[1])));
        try (InputStream input = J2meApi.class.getResourceAsStream(RESOURCE_ROOT + parts[0] + "/" + parts[2] + ".class")) {
          if (input == null) throw new IOException("Missing bundled API class: " + line);
          addDefinition(library, input.readAllBytes());
        }
      }
      return List.copyOf(libraries.values());
    } catch (IOException ex) {
      throw new UncheckedIOException("Cannot read bundled J2ME declarations", ex);
    }
  }

  public static Library library(String name, Map<String, byte[]> classes, boolean compileOnly, boolean fallback) {
    Library library = new Library(name, compileOnly, fallback);
    classes.values().forEach(bytes -> addDefinition(library, bytes));
    return library;
  }

  public static Library library(Path path) throws IOException {
    Path key = path.toAbsolutePath().normalize();
    long size = Files.size(key);
    FileTime modified = Files.getLastModifiedTime(key);
    CachedLibrary cached = LIBRARIES.get(key);
    if (cached != null && cached.size == size && cached.modified.equals(modified)) return cached.library;
    try (JarFile jar = new JarFile(key.toFile())) {
      boolean compileOnly = "compile-only".equals(attribute(jar, "J2ME-Stub-Kind"));
      String explicitFallback = attribute(jar, "J2ME-Stub-Fallback");
      Library library = new Library(key.toString(), compileOnly,
        "true".equals(explicitFallback) || !"false".equals(explicitFallback) && compileOnly);
      for (var entry : Collections.list(jar.entries())) {
        if (!isClassEntry(entry.getName()) || entry.isDirectory()) continue;
        try (InputStream input = jar.getInputStream(entry)) { addDefinition(library, input.readAllBytes()); }
      }
      // Retain only the latest contents of each local SDK, even after rebuilds.
      LIBRARIES.put(key, new CachedLibrary(size, modified, library));
      return library;
    }
  }

  private static String attribute(JarFile jar, String name) {
    try {
      return jar.getManifest() == null ? null : jar.getManifest().getMainAttributes().getValue(name);
    } catch (IOException ex) {
      return null; // Malformed advisory manifests must not hide usable classes.
    }
  }

  private static boolean isClassEntry(String name) { return name.endsWith(".class") && !name.startsWith("META-INF/"); }
  private static boolean isClass(byte[] bytes) {
    return bytes != null && bytes.length >= 4 && bytes[0] == (byte)0xca && bytes[1] == (byte)0xfe && bytes[2] == (byte)0xba && bytes[3] == (byte)0xbe;
  }

  private static void addDefinition(Library library, byte[] bytes) {
    if (!isClass(bytes)) return;
    ClassReader reader = new ClassReader(bytes);
    library.classes.put(reader.getClassName(), definition(bytes, reader));
  }

  private static Definition definition(byte[] bytes, ClassReader reader) {
    List<String> parents = new ArrayList<>();
    if (reader.getSuperName() != null) parents.add(reader.getSuperName());
    parents.addAll(Arrays.asList(reader.getInterfaces()));
    Set<Member> members = new LinkedHashSet<>();
    reader.accept(new ClassVisitor(Opcodes.ASM9) {
      @Override public FieldVisitor visitField(int access, String name, String descriptor, String signature, Object value) {
        if ((access & Opcodes.ACC_PRIVATE) == 0) members.add(new Member(name, descriptor, true, (access & Opcodes.ACC_STATIC) != 0));
        return null;
      }
      @Override public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
        if ((access & Opcodes.ACC_PRIVATE) == 0) members.add(new Member(name, descriptor, false, (access & Opcodes.ACC_STATIC) != 0));
        return null;
      }
    }, ClassReader.SKIP_CODE | ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
    return new Definition(bytes, parents, members);
  }

  public static final class Requirements {
    private final Map<String, Set<Member>> members = new LinkedHashMap<>();
    private final Set<String> classes = new LinkedHashSet<>();
    private final Map<String, Definition> projectClasses = new LinkedHashMap<>();
    private final Set<String> configurations = new HashSet<>();
    private boolean floatingPoint;

    public void configuration(String value) {
      if (value != null) configurations.add(value.toUpperCase(Locale.ROOT));
    }

    /** Avoid installing a CLDC core for unrelated desktop Java programs. */
    public boolean isJ2me() {
      if (!configurations.isEmpty()) return true;
      return classes.stream().anyMatch(name -> name.startsWith("javax/microedition/") || name.startsWith("javax/bluetooth/") ||
        name.startsWith("javax/obex/") || name.startsWith("javax/wireless/") || name.startsWith("com/nokia/") ||
        name.startsWith("com/siemens/") || name.startsWith("com/motorola/") || name.startsWith("com/samsung/") ||
        name.startsWith("com/mascotcapsule/") || name.startsWith("com/vodafone/") || name.startsWith("com/sprintpcs/") ||
        name.startsWith("com/jblend/") || name.startsWith("org/pigler/api/") || name.startsWith("com/gameloft/android2d/iap/") ||
        name.equals("com/framework/VservAgent") || name.equals("com/framework/VSERV_BCI_CLASS_000") ||
        name.startsWith("net/rim/") || name.startsWith("mmpp/"));
    }

    private void type(Type type) {
      switch (type.getSort()) {
        case Type.OBJECT -> classes.add(type.getInternalName());
        case Type.ARRAY -> type(type.getElementType());
        case Type.FLOAT, Type.DOUBLE -> floatingPoint = true;
        case Type.METHOD -> {
          for (Type argument : type.getArgumentTypes()) type(argument);
          type(type.getReturnType());
        }
      }
    }

    private void reference(String owner, Member member) { members.computeIfAbsent(owner, key -> new LinkedHashSet<>()).add(member); }

    public void inspectDescriptor(String descriptor) {
      for (int start = descriptor.indexOf('L'); start >= 0; ) {
        int end = descriptor.indexOf(';', start);
        if (end < 0) break;
        classes.add(descriptor.substring(start + 1, end));
        start = descriptor.indexOf('L', end + 1);
      }
    }

    /** Detection does not parse methods: unrelated legacy bytecode may have
     * malformed attributes that the decompiler can recover but ASM rejects. */
    public void inspectClassNames(byte[] bytes) {
      if (!isClass(bytes)) return;
      ClassReader reader = new ClassReader(bytes);
      char[] chars = new char[reader.getMaxStringLength()];
      for (int index = 1; index < reader.getItemCount(); index++) {
        int offset = reader.getItem(index);
        if (offset != 0 && reader.readByte(offset - 1) == 7) {
          String name = reader.readUTF8(offset, chars);
          if (name != null) {
            if (name.startsWith("[")) inspectDescriptor(name); else classes.add(name);
          }
        }
      }
    }

    public void addClass(byte[] bytes) {
      if (!isClass(bytes)) return;
      ClassReader reader = new ClassReader(bytes);
      char[] chars = new char[reader.getMaxStringLength()];
      // Include unused/guarded constant-pool references. Return descriptors matter
      // when selecting vendor revisions with source-incompatible overloads.
      for (int index = 1; index < reader.getItemCount(); index++) {
        int offset = reader.getItem(index);
        if (offset == 0) continue;
        int tag = reader.readByte(offset - 1);
        if (tag == 4 || tag == 6) floatingPoint = true;
        if (tag == 7) {
          String name = reader.readUTF8(offset, chars);
          if (name.startsWith("[")) type(Type.getType(name)); else classes.add(name);
        } else if (tag == 9 || tag == 10 || tag == 11) {
          int nameAndType = reader.getItem(reader.readUnsignedShort(offset + 2));
          Member member = new Member(reader.readUTF8(nameAndType, chars), reader.readUTF8(nameAndType + 2, chars), tag == 9, null);
          reference(reader.readClass(offset, chars), member);
          type(Type.getType(member.descriptor));
        }
      }
      Definition declared = definition(bytes, reader);
      reader.accept(new ClassVisitor(Opcodes.ASM9) {
        @Override public FieldVisitor visitField(int access, String name, String descriptor, String signature, Object value) {
          type(Type.getType(descriptor));
          declared.members.add(new Member(name, descriptor, true, (access & Opcodes.ACC_STATIC) != 0));
          return null;
        }
        @Override public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
          type(Type.getMethodType(descriptor));
          declared.members.add(new Member(name, descriptor, false, (access & Opcodes.ACC_STATIC) != 0));
          return new MethodVisitor(Opcodes.ASM9) {
            @Override public void visitInsn(int opcode) {
              switch (opcode) {
                case Opcodes.FCONST_0, Opcodes.FCONST_1, Opcodes.FCONST_2, Opcodes.DCONST_0, Opcodes.DCONST_1,
                     Opcodes.I2F, Opcodes.I2D, Opcodes.L2F, Opcodes.L2D -> floatingPoint = true;
              }
            }
            @Override public void visitIntInsn(int opcode, int operand) {
              if (opcode == Opcodes.NEWARRAY && (operand == Opcodes.T_FLOAT || operand == Opcodes.T_DOUBLE)) floatingPoint = true;
            }
            @Override public void visitMethodInsn(int opcode, String owner, String name, String descriptor, boolean isInterface) {
              reference(owner, new Member(name, descriptor, false, opcode == Opcodes.INVOKESTATIC));
            }
            @Override public void visitFieldInsn(int opcode, String owner, String name, String descriptor) {
              reference(owner, new Member(name, descriptor, true, opcode == Opcodes.GETSTATIC || opcode == Opcodes.PUTSTATIC));
            }
          };
        }
      }, ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
      projectClasses.put(reader.getClassName(), declared);
    }
  }

  public static Resolution resolve(Path input, List<Path> jars, boolean includeBundled) throws IOException {
    List<Library> libraries = new ArrayList<>(includeBundled ? bundled() : List.of());
    for (Path path : jars.stream().map(p -> p.toAbsolutePath().normalize()).distinct().sorted().toList()) libraries.add(library(path));
    if (libraries.isEmpty()) return new Resolution(Map.of());
    Requirements requirements = new Requirements();
    try (JarFile jar = new JarFile(input.toFile())) {
      requirements.configuration(attribute(jar, "MicroEdition-Configuration"));
      for (var entry : Collections.list(jar.entries())) {
        if (!entry.isDirectory() && isClassEntry(entry.getName())) {
          try (InputStream stream = jar.getInputStream(entry)) { requirements.addClass(stream.readAllBytes()); }
        }
      }
    }
    return resolve(requirements, libraries);
  }

  public static Resolution resolve(Requirements requirements, List<Library> libraries) {
    return new Selection(requirements, libraries).resolve();
  }

  private static final class Selection {
    private final Requirements requirements;
    private final Map<String, List<Library>> providers = new LinkedHashMap<>();
    private final Map<String, Set<Member>> demands = new LinkedHashMap<>();
    private final Map<Library, Map<String, Integer>> missingCounts = new HashMap<>();
    private final List<Library> boots;
    private final Set<String> coreOwners = new HashSet<>();
    private final boolean needsFloatingPoint;
    private final boolean wantsCldc10;
    private Library boot;

    Selection(Requirements requirements, List<Library> libraries) {
      this.requirements = requirements;
      for (Library library : libraries) for (String owner : library.classes.keySet()) providers.computeIfAbsent(owner, key -> new ArrayList<>()).add(library);
      requirements.members.forEach((owner, members) -> {
        Set<Member> references = new LinkedHashSet<>(members);
        for (Member member : members) if (member.isStatic != null) references.remove(member.withStatic(null));
        demands.put(owner, references);
      });
      // Calls may name a project subclass instead of their API declaration owner.
      new LinkedHashMap<>(demands).forEach((owner, members) -> {
        if (!requirements.projectClasses.containsKey(owner)) return;
        for (Member member : members) {
          String inherited = inheritedOwner(owner, member, new HashSet<>());
          if (inherited != null) demands.computeIfAbsent(inherited, key -> new LinkedHashSet<>()).add(member);
        }
      });
      boots = providers.getOrDefault("java/lang/Object", List.of());
      List<Library> standalone = boots.stream().filter(l -> !l.classes.containsKey("javax/microedition/midlet/MIDlet")).toList();
      for (Library library : standalone.isEmpty() ? boots : standalone) {
        for (String owner : library.classes.keySet()) if (owner.startsWith("java/")) coreOwners.add(owner);
      }
      needsFloatingPoint = requirements.floatingPoint || requirements.classes.contains("java/lang/Float") || requirements.classes.contains("java/lang/Double");
      wantsCldc10 = requirements.configurations.equals(Set.of("CLDC-1.0")) && !needsFloatingPoint;
    }

    private boolean supports(Library library, String owner, Member member, Set<String> visited) {
      if (!visited.add(owner)) return false;
      try {
        Definition own = library.classes.get(owner);
        List<Definition> alternatives = own != null ? List.of(own) : providers.getOrDefault(owner, List.of()).stream().map(l -> l.classes.get(owner)).toList();
        for (Definition definition : alternatives) {
          if (definition.declares(member)) return true;
          if (!member.name.equals("<init>")) for (String parent : definition.parents) if (supports(library, parent, member, visited)) return true;
        }
        return false;
      } finally { visited.remove(owner); }
    }

    private String inheritedOwner(String owner, Member member, Set<String> visited) {
      if (!visited.add(owner)) return null;
      Definition own = requirements.projectClasses.get(owner);
      if (own == null) return providers.getOrDefault(owner, List.of()).stream().anyMatch(l -> supports(l, owner, member, new HashSet<>())) ? owner : null;
      if (member.name.equals("<init>") || own.declares(member)) return null;
      for (String parent : own.parents) {
        String inherited = inheritedOwner(parent, member, new HashSet<>(visited));
        if (inherited != null) return inherited;
      }
      return null;
    }

    private int missing(Library library, String owner) {
      return missingCounts.computeIfAbsent(library, key -> new HashMap<>()).computeIfAbsent(owner,
        key -> (int)demands.getOrDefault(owner, Set.of()).stream().filter(m -> !supports(library, owner, m, new HashSet<>())).count());
    }

    private Library chooseBoot() {
      Set<String> requiredCore = new HashSet<>(requirements.classes);
      requiredCore.addAll(demands.keySet());
      if (needsFloatingPoint) requiredCore.addAll(Set.of("java/lang/Float", "java/lang/Double"));
      requiredCore.retainAll(coreOwners);
      return boots.stream().min(Comparator
        .comparingInt((Library l) -> (int)requiredCore.stream().filter(c -> !l.classes.containsKey(c)).count())
        .thenComparingInt(l -> demands.keySet().stream().filter(coreOwners::contains).mapToInt(c -> missing(l, c)).sum())
        .thenComparingInt(l -> l.classes.containsKey("javax/microedition/midlet/MIDlet") ? 1 : 0)
        .thenComparingInt(l -> (!l.classes.containsKey("java/lang/Float")) == wantsCldc10 ? 0 : 1)
        .thenComparingInt(l -> l.compileOnly ? 1 : 0)
        .thenComparingInt(l -> l.classes.size()).thenComparing(l -> l.name)).orElse(null);
    }

    private Map<String, Library> selectAll() {
      Map<String, Library> selected = new TreeMap<>();
      providers.forEach((owner, alternatives) -> {
        if (coreOwners.contains(owner)) {
          if (boot != null && boot.classes.containsKey(owner)) selected.put(owner, boot);
        } else {
          selected.put(owner, alternatives.stream().min(Comparator
            .comparingInt((Library l) -> missing(l, owner))
            .thenComparingInt(l -> l.fallback ? 1 : 0)
            .thenComparingInt(l -> l.compileOnly ? 1 : 0)
            .thenComparingInt(l -> l.classes.containsKey("java/lang/Object") ? 1 : 0)
            .thenComparingInt(l -> l.classes.size()).thenComparing(l -> l.name)).orElseThrow());
        }
      });
      return selected;
    }

    Resolution resolve() {
      boot = chooseBoot();
      Map<String, Library> selected = selectAll();
      boolean changed;
      do {
        changed = false;
        // Propagate inherited requirements before fixing each class's provider.
        for (var entry : new LinkedHashMap<>(demands).entrySet()) {
          Library library = selected.get(entry.getKey());
          if (library == null) continue;
          Definition definition = library.classes.get(entry.getKey());
          for (Member member : List.copyOf(entry.getValue())) {
            if (member.name.equals("<init>") || definition.declares(member)) continue;
            for (String parent : definition.parents) {
              if (providers.getOrDefault(parent, List.of()).stream().anyMatch(l -> supports(l, parent, member, new HashSet<>()))) {
                if (demands.computeIfAbsent(parent, key -> new LinkedHashSet<>()).add(member)) changed = true;
                break;
              }
            }
          }
        }
        if (changed) { missingCounts.clear(); boot = chooseBoot(); selected = selectAll(); }
      } while (changed);
      return new Resolution(selected);
    }
  }

  /** Read-only in-memory library; no files are extracted during decompilation. */
  public static final class Resolution implements IContextSource {
    private final Map<String, byte[]> classes = new TreeMap<>();
    private final String providerIndex;

    private Resolution(Map<String, Library> selected) {
      StringBuilder index = new StringBuilder("class\tlibrary\tcompile_only\n");
      selected.forEach((owner, library) -> {
        classes.put(owner, library.classes.get(owner).bytes);
        String name = library.name.substring(library.name.lastIndexOf(File.separatorChar) + 1);
        index.append(owner).append('\t').append(name).append('\t').append(library.compileOnly).append('\n');
      });
      providerIndex = index.toString();
    }

    /** The returned arrays are shared declaration data and must not be modified. */
    public Map<String, byte[]> classes() { return Collections.unmodifiableMap(classes); }
    public String providerIndex() { return providerIndex; }
    @Override public String getName() { return "resolved J2ME APIs"; }
    @Override public Entries getEntries() { return new Entries(classes.keySet().stream().map(Entry::atBase).toList(), List.of(), List.of()); }
    @Override public InputStream getInputStream(String resource) {
      byte[] bytes = resource.endsWith(".class") ? classes.get(resource.substring(0, resource.length() - 6)) : null;
      return bytes == null ? null : new ByteArrayInputStream(bytes);
    }
  }
}
