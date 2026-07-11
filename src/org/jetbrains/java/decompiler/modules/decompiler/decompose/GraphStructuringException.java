// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.decompose;

/**
 * Signals that a valid CFG could not be reduced to structured statements.
 *
 * <p>This is deliberately distinct from invariant and implementation failures: callers may retry a different
 * semantics-preserving structural view for this failure, but must not hide unrelated runtime exceptions.</p>
 */
public final class GraphStructuringException extends RuntimeException {
  public GraphStructuringException(String message) {
    super(message);
  }
}
