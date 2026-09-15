package org.vineflower.apistubs

import groovy.json.JsonOutput
import groovy.json.JsonSlurper
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassWriter
import org.objectweb.asm.Type
import org.objectweb.asm.tree.ClassNode

import java.util.jar.JarFile

import static org.objectweb.asm.Opcodes.*

/** Build-time declaration import and generation. SDK bytecode never enters the catalog. */
class ApiStubs {
  static void extract(File inputs, File output) {
    List<File> jars = inputs.listFiles()?.findAll { it.name.endsWith('.jar') }?.sort { it.name }
    if (!jars) throw new IllegalArgumentException("No API jars in $inputs")
    output.mkdirs()
    jars.each { file ->
      new JarFile(file).withCloseable { jar ->
        def classes = jar.entries().findAll { !it.directory && it.name.endsWith('.class') && !it.name.startsWith('META-INF/') }
          .sort { it.name }.collect { entry ->
            ClassNode node = new ClassNode()
            jar.getInputStream(entry).withCloseable {
              new ClassReader(it).accept(node, ClassReader.SKIP_CODE | ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES)
            }
            declaration(node)
          }
        def attributes = jar.manifest?.mainAttributes
        String explicitFallback = attributes?.getValue('J2ME-Stub-Fallback')
        boolean fallback = explicitFallback != null ? explicitFallback.toBoolean() :
          attributes?.getValue('J2ME-Stub-Kind') == 'compile-only'
        writeCatalog(new File(output, file.name.replaceFirst(/\.jar$/, '.json')), fallback, classes)
      }
    }
  }

  private static Map declaration(ClassNode node) {
    def result = [name: node.name, version: node.version, access: node.access, superName: node.superName,
                  interfaces: node.interfaces]
    if (node.signature != null) result.signature = node.signature
    if (node.outerClass != null) result.enclosing = [node.outerClass, node.outerMethod, node.outerMethodDesc]
    result.innerClasses = node.innerClasses.collect { [it.name, it.outerName, it.innerName, it.access] }.sort { it[0] }
    // Keep private constructors: removing them can change javac's view of whether
    // a class is instantiable. Other private members are implementation details.
    result.fields = node.fields.findAll { (it.access & ACC_PRIVATE) == 0 }.collect { field ->
      def item = [name: field.name, descriptor: field.desc, access: field.access]
      if (field.signature != null) item.signature = field.signature
      if (field.value != null) item.constant = constantText(field.value)
      item
    }.sort { a, b -> (a.name <=> b.name) ?: (a.descriptor <=> b.descriptor) }
    result.methods = node.methods.findAll {
      it.name != '<clinit>' && ((it.access & ACC_PRIVATE) == 0 || it.name == '<init>')
    }.collect { method ->
      def item = [name: method.name, descriptor: method.desc, access: method.access]
      if (method.signature != null) item.signature = method.signature
      if (method.exceptions) item.exceptions = method.exceptions
      item
    }.sort { a, b -> (a.name <=> b.name) ?: (a.descriptor <=> b.descriptor) }
    result
  }

  private static String constantText(Object value) {
    // JSON numbers cannot preserve NaNs, infinities or negative zero. Store the
    // exact IEEE bits for floating-point constants, and decimal integers as text.
    if (value instanceof Float) return Integer.toHexString(Float.floatToRawIntBits(value))
    if (value instanceof Double) return Long.toHexString(Double.doubleToRawLongBits(value))
    value.toString()
  }

  private static Object constant(String descriptor, String value) {
    if (value == null) return null
    switch (descriptor) {
      case 'F': return Float.intBitsToFloat(Integer.parseUnsignedInt(value, 16))
      case 'D': return Double.longBitsToDouble(Long.parseUnsignedLong(value, 16))
      case 'J': return Long.valueOf(value)
      case 'Ljava/lang/String;': return value
      default: return Integer.valueOf(value)
    }
  }

  private static void writeCatalog(File file, boolean fallback, List<Map> classes) {
    // One member per line keeps the large declaration tables reviewable without
    // adding a custom parser or a separate file for every SDK class.
    file.withWriter('UTF-8') { out ->
      out.write('{"fallback": ' + fallback + ', "classes": [\n')
      classes.eachWithIndex { cls, index ->
        def header = cls.findAll { key, value -> key != 'fields' && key != 'methods' }
        out.write('  ' + JsonOutput.toJson(header)[0..-2] + ',\n')
        ['fields', 'methods'].eachWithIndex { key, section ->
          out.write('    "' + key + '": [\n')
          cls[key].eachWithIndex { member, i ->
            out.write('      ' + JsonOutput.toJson(member) + (i + 1 < cls[key].size() ? ',' : '') + '\n')
          }
          out.write('    ]' + (section == 0 ? ',' : '') + '\n')
        }
        out.write('  }' + (index + 1 < classes.size() ? ',' : '') + '\n')
      }
      out.write(']}\n')
    }
  }

  static void generate(File catalog, File output) {
    def files = catalog.listFiles()?.findAll { it.name.endsWith('.json') }?.sort { it.name }
    if (!files) throw new IllegalArgumentException("No API catalogs in $catalog")
    output.mkdirs()
    StringBuilder index = new StringBuilder()
    files.each { file ->
      def data = new JsonSlurper().parse(file, 'UTF-8')
      String variant = file.name.replaceFirst(/\.json$/, '')
      data.classes.sort { it.name }.each { cls ->
        File target = new File(output, "META-INF/j2me-api/$variant/${cls.name}.class")
        target.parentFile.mkdirs()
        target.bytes = stub(cls)
        index.append(variant).append('\t').append(data.fallback).append('\t').append(cls.name).append('\n')
      }
    }
    new File(output, 'META-INF/j2me-api/index.tsv').setText(index.toString(), 'UTF-8')
  }

  private static byte[] stub(Map cls) {
    ClassWriter writer = new ClassWriter(0)
    writer.visit(cls.version as int, cls.access as int, cls.name, cls.signature, cls.superName, cls.interfaces as String[])
    if (cls.enclosing) writer.visitOuterClass(*cls.enclosing)
    cls.innerClasses.each { writer.visitInnerClass(it[0], it[1], it[2], it[3] as int) }
    cls.fields.each { field ->
      writer.visitField(field.access as int, field.name, field.descriptor, field.signature,
        constant(field.descriptor, field.constant)).visitEnd()
    }
    cls.methods.each { method ->
      int access = method.access as int
      def visitor = writer.visitMethod(access, method.name, method.descriptor, method.signature, method.exceptions as String[])
      if ((access & (ACC_ABSTRACT | ACC_NATIVE)) == 0) {
        visitor.visitCode()
        // No host-library calls, superclass constructor assumptions or copied
        // implementation. Even a constructor can throw before initializing this.
        visitor.visitInsn(ACONST_NULL)
        visitor.visitInsn(ATHROW)
        int locals = (access & ACC_STATIC) == 0 ? 1 : 0
        Type.getArgumentTypes(method.descriptor).each { locals += it.size }
        visitor.visitMaxs(1, locals)
      }
      visitor.visitEnd()
    }
    writer.visitEnd()
    writer.toByteArray()
  }

}
