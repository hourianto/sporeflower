// Copyright 2000-2017 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.main.extern;

public interface IIdentifierRenamer {

  enum Type {ELEMENT_CLASS, ELEMENT_FIELD, ELEMENT_METHOD}

  boolean toBeRenamed(Type elementType, String className, String element, String descriptor);

  /**
   * Suggests a simple or internal class name using the original identity.
   * The allocator asks once; an illegal or occupied suggestion uses a built-in fallback.
   */
  String getNextClassName(String fullName, String shortName);

  /**
   * Suggests a name once using the original bytecode identity.
   * Illegal or occupied suggestions use the built-in fallback.
   */
  String getNextFieldName(String className, String field, String descriptor);

  /**
   * Suggests a name using the original bytecode identity. Override-related
   * methods share one selected suggestion; illegal or occupied suggestions
   * use the built-in fallback.
   */
  String getNextMethodName(String className, String method, String descriptor);
}
