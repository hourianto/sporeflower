// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.renamer;

import java.util.HashMap;
import java.util.Map;
import java.util.HashSet;
import java.util.Set;
import org.jetbrains.java.decompiler.code.CodeConstants;
import org.jetbrains.java.decompiler.struct.StructContext;
import org.jetbrains.java.decompiler.struct.StructClass;
import org.jetbrains.java.decompiler.struct.consts.LinkConstant;

public class PoolInterceptor {
  private final Map<String, String> mapOldToNewNames = new HashMap<>();
  private final Map<String, String> mapNewToOldNames = new HashMap<>();
  private final Map<String, String> originalMemberOwners = new HashMap<>();

  private final Map<String, String> innerNames = new HashMap<>();

  public void setInnerName(String emitted, String simpleName) { innerNames.put(emitted, simpleName); }

  public String innerName(String emitted, String fallback) { return innerNames.getOrDefault(emitted, fallback); }

  private final Set<String> retainedBridges = new HashSet<>();

  public void retainBridge(String owner, String name, String descriptor) {
    retainedBridges.add(owner + " " + name + " " + descriptor);
  }

  public boolean isRetainedBridge(String owner, String name, String descriptor) {
    return retainedBridges.contains(owner + " " + name + " " + descriptor);
  }

  public void bindMemberReferences(StructContext context) {
    for (StructClass owner : context.getOwnClasses()) {
      for (var constant : owner.getPool().getPool()) {
        if (!(constant instanceof LinkConstant link)
            || link.type != CodeConstants.CONSTANT_Fieldref && link.type != CodeConstants.CONSTANT_Methodref
              && link.type != CodeConstants.CONSTANT_InterfaceMethodref) continue;
        boolean field = link.type == CodeConstants.CONSTANT_Fieldref;
        String key = referenceKey(link, field);
        if (!originalMemberOwners.containsKey(key)) {
          String declaration = findDeclaration(context, link.classname, link.elementname, link.descriptor, field, new HashSet<>());
          if (declaration != null) originalMemberOwners.put(key, declaration);
        }
      }
    }
  }

  public String originalMemberOwner(LinkConstant reference, boolean field) {
    return originalMemberOwners.get(referenceKey(reference, field));
  }

  private static String referenceKey(LinkConstant reference, boolean field) {
    return (field ? "F " : "M ") + reference.classname + " " + reference.elementname + " " + reference.descriptor;
  }

  private static String findDeclaration(StructContext context, String owner, String name, String descriptor,
                                        boolean field, Set<String> visited) {
    if (!visited.add(owner)) return null;
    StructClass type = context.getClass(owner);
    if (type == null) return null;
    if (field ? type.getField(name, descriptor) != null : type.getMethod(name, descriptor) != null) return owner;
    if (!field && name.startsWith("<")) return null;
    if (!field && type.superClass != null) {
      String found = findDeclaration(context, type.superClass.getString(), name, descriptor, false, visited);
      if (found != null) return found;
    }
    for (String iface : type.getInterfaceNames()) {
      String found = findDeclaration(context, iface, name, descriptor, field, visited);
      if (found != null) return found;
    }
    return field && type.superClass != null
      ? findDeclaration(context, type.superClass.getString(), name, descriptor, true, visited) : null;
  }

  public void addName(String oldName, String newName) {
    String previous = mapOldToNewNames.put(oldName, newName);
    if (previous != null) mapNewToOldNames.remove(previous, oldName);
    mapNewToOldNames.put(newName, oldName);
  }

  public String getName(String oldName) {
    return mapOldToNewNames.get(oldName);
  }

  public String getOldName(String newName) {
    return mapNewToOldNames.get(newName);
  }
}
