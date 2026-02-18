// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.struct.consts;

import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.main.DecompilerContext;
import org.jetbrains.java.decompiler.modules.renamer.PoolInterceptor;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.gen.FieldDescriptor;
import org.jetbrains.java.decompiler.struct.gen.MethodDescriptor;
import org.jetbrains.java.decompiler.struct.gen.NewClassNameBuilder;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DataInputFullStream;

import java.io.DataInputStream;
import java.io.IOException;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

@SuppressWarnings("AssignmentToForLoopParameter")
public class ConstantPool implements NewClassNameBuilder {
  public static final int FIELD = 1;
  public static final int METHOD = 2;

  private final List<PooledConstant> pool;
  private final PoolInterceptor interceptor;
  private final Map<String, String> hierarchyResolvedMemberNames = new HashMap<>();

  public ConstantPool(DataInputStream in) throws IOException {
    int size = in.readUnsignedShort();
    pool = new ArrayList<>(size);
    BitSet[] nextPass = {new BitSet(size), new BitSet(size), new BitSet(size)};

    // first dummy constant
    pool.add(null);

    // first pass: read the elements
    for (int i = 1; i < size; i++) {
      byte tag = (byte)in.readUnsignedByte();

      switch (tag) {
        case CodeConstants.CONSTANT_Utf8:
          pool.add(new PrimitiveConstant(CodeConstants.CONSTANT_Utf8, in.readUTF()));
          break;

        case CodeConstants.CONSTANT_Integer:
          pool.add(new PrimitiveConstant(CodeConstants.CONSTANT_Integer, Integer.valueOf(in.readInt())));
          break;

        case CodeConstants.CONSTANT_Float:
          pool.add(new PrimitiveConstant(CodeConstants.CONSTANT_Float, in.readFloat()));
          break;

        case CodeConstants.CONSTANT_Long:
          pool.add(new PrimitiveConstant(CodeConstants.CONSTANT_Long, in.readLong()));
          pool.add(null);
          i++;
          break;

        case CodeConstants.CONSTANT_Double:
          pool.add(new PrimitiveConstant(CodeConstants.CONSTANT_Double, in.readDouble()));
          pool.add(null);
          i++;
          break;

        case CodeConstants.CONSTANT_Class:
        case CodeConstants.CONSTANT_String:
        case CodeConstants.CONSTANT_MethodType:
        case CodeConstants.CONSTANT_Module:
        case CodeConstants.CONSTANT_Package:
          pool.add(new PrimitiveConstant(tag, in.readUnsignedShort()));
          nextPass[0].set(i);
          break;

        case CodeConstants.CONSTANT_NameAndType:
          pool.add(new LinkConstant(tag, in.readUnsignedShort(), in.readUnsignedShort()));
          nextPass[0].set(i);
          break;

        case CodeConstants.CONSTANT_Fieldref:
        case CodeConstants.CONSTANT_Methodref:
        case CodeConstants.CONSTANT_InterfaceMethodref:
        case CodeConstants.CONSTANT_InvokeDynamic:
        case CodeConstants.CONSTANT_Dynamic:
          pool.add(new LinkConstant(tag, in.readUnsignedShort(), in.readUnsignedShort()));
          nextPass[1].set(i);
          break;

        case CodeConstants.CONSTANT_MethodHandle:
          pool.add(new LinkConstant(tag, in.readUnsignedByte(), in.readUnsignedShort()));
          nextPass[2].set(i);
          break;

        default:
          throw new RuntimeException("Invalid Constant Pool entry #" + i + " Type: " + tag);
      }
    }

    // resolving complex pool elements
    for (BitSet pass : nextPass) {
      int idx = 0;
      while ((idx = pass.nextSetBit(idx + 1)) > 0) {
        pool.get(idx).resolveConstant(this);
      }
    }

    // get global constant pool interceptor instance, if any available
    interceptor = DecompilerContext.getPoolInterceptor();
  }

  public String[] getClassElement(int elementType, String className, int nameIndex, int descriptorIndex) {
    String elementName = ((PrimitiveConstant)getConstant(nameIndex)).getString();
    String descriptor = ((PrimitiveConstant)getConstant(descriptorIndex)).getString();

    if (interceptor != null) {
      String oldClassName = interceptor.getOldName(className);
      if (oldClassName != null) {
        className = oldClassName;
      }

      String newElement = interceptor.getName(className + ' ' + elementName + ' ' + descriptor);
      if (newElement != null) {
        elementName = newElement.split(" ")[1];
      }

      String newDescriptor = buildNewDescriptor(elementType == FIELD, descriptor);
      if (newDescriptor != null) {
        descriptor = newDescriptor;
      }
    }

    return new String[]{elementName, descriptor};
  }

  public PooledConstant getConstant(int index) {
    return pool.get(index);
  }

  public PrimitiveConstant getPrimitiveConstant(int index) {
    PrimitiveConstant cn = (PrimitiveConstant)getConstant(index);

    if (cn != null && interceptor != null) {
      if (cn.type == CodeConstants.CONSTANT_Class) {
        String newName = buildNewClassname(cn.getString());
        if (newName != null) {
          cn = new PrimitiveConstant(CodeConstants.CONSTANT_Class, newName);
        }
      }
    }

    return cn;
  }

  public LinkConstant getLinkConstant(int index) {
    LinkConstant ln = (LinkConstant)getConstant(index);

    if (ln != null && interceptor != null &&
        (ln.type == CodeConstants.CONSTANT_Fieldref ||
         ln.type == CodeConstants.CONSTANT_Methodref ||
         ln.type == CodeConstants.CONSTANT_InterfaceMethodref)) {
      boolean isField = ln.type == CodeConstants.CONSTANT_Fieldref;
      String newClassName = buildNewClassname(ln.classname);
      String newElement = resolveMemberRename(ln, isField);
      String newDescriptor = buildNewDescriptor(ln.type == CodeConstants.CONSTANT_Fieldref, ln.descriptor);

      if (newClassName != null || newElement != null || newDescriptor != null) {
        String className = newClassName == null ? ln.classname : newClassName;
        String elementName = newElement == null ? ln.elementname : newElement.split(" ")[1];
        String descriptor = newDescriptor == null ? ln.descriptor : newDescriptor;
        ln = new LinkConstant(ln.type, className, elementName, descriptor);
      }
    }

    return ln;
  }

  @Override
  public String buildNewClassname(String className) {
    VarType vt = new VarType(className, true);

    String newName = interceptor.getName(vt.value);
    if (newName != null) {
      StringBuilder buffer = new StringBuilder();
      if (vt.arrayDim > 0) {
        // No functional change, just a revert for J8 compatibility
        for (int i = 0; i < vt.arrayDim; i++) {
          buffer.append('[');
        }

        buffer.append('L').append(newName).append(';');
      }
      else {
        buffer.append(newName);
      }
      return buffer.toString();
    }

    return null;
  }

  public List<PooledConstant> getPool() {
    return pool;
  }

  private String resolveMemberRename(LinkConstant ln, boolean isField) {
    String directKey = ln.classname + ' ' + ln.elementname + ' ' + ln.descriptor;
    String direct = interceptor.getName(directKey);
    if (direct != null) {
      return direct;
    }

    String cacheKey = (isField ? "F " : "M ") + directKey;
    if (hierarchyResolvedMemberNames.containsKey(cacheKey)) {
      return hierarchyResolvedMemberNames.get(cacheKey);
    }

    String mapped = null;
    String declaringClass = findDeclaringClassName(ln.classname, ln.elementname, ln.descriptor, isField);
    if (declaringClass != null) {
      mapped = interceptor.getName(declaringClass + ' ' + ln.elementname + ' ' + ln.descriptor);
    }

    hierarchyResolvedMemberNames.put(cacheKey, mapped);
    return mapped;
  }

  private String findDeclaringClassName(String owner, String name, String descriptor, boolean isField) {
    if (owner == null || owner.isEmpty()) {
      return null;
    }

    ArrayDeque<String> queue = new ArrayDeque<>();
    Set<String> visited = new HashSet<>();
    queue.add(owner);

    while (!queue.isEmpty()) {
      String current = toOldClassName(queue.removeFirst());
      if (!visited.add(current)) {
        continue;
      }

      if (interceptor.getName(current + ' ' + name + ' ' + descriptor) != null) {
        return current;
      }

      StructClass cl = DecompilerContext.getStructContext().getClass(current);
      if (cl == null) {
        String renamed = interceptor.getName(current);
        if (renamed != null) {
          cl = DecompilerContext.getStructContext().getClass(renamed);
        }
      }
      if (cl == null) {
        continue;
      }

      if (isField && classDeclaresField(cl, name, descriptor)) {
        return null;
      }

      if (cl.superClass != null && cl.superClass.value instanceof String superClassName) {
        queue.addLast(toOldClassName(superClassName));
      }

      for (String iface : cl.getInterfaceNames()) {
        queue.addLast(toOldClassName(iface));
      }
    }

    return null;
  }

  private String toOldClassName(String className) {
    String oldName = interceptor.getOldName(className);
    return oldName == null ? className : oldName;
  }

  private boolean classDeclaresField(StructClass cl, String name, String descriptor) {
    if (cl.getField(name, descriptor) != null) {
      return true;
    }

    String mappedDescriptor = buildNewDescriptor(true, descriptor);
    return mappedDescriptor != null && cl.getField(name, mappedDescriptor) != null;
  }

  private String buildNewDescriptor(boolean isField, String descriptor) {
    if (isField) {
      return FieldDescriptor.parseDescriptor(descriptor).buildNewDescriptor(this);
    }
    else {
      return MethodDescriptor.parseDescriptor(descriptor).buildNewDescriptor(this);
    }
  }
}
