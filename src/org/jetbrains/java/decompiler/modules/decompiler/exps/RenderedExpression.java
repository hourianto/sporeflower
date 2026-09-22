package org.jetbrains.java.decompiler.modules.decompiler.exps;

import org.jetbrains.java.decompiler.util.TextBuffer;

/** Source text and the precedence of that exact representation. */
public record RenderedExpression(TextBuffer text, int precedence) { }
