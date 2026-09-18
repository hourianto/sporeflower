package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.ArrayList;
import java.util.List;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.Condition;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ContainerSemantics;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.SlotSource;

/** One normalized contract, independent of the annotation or transport entry that supplied it. */
record SemanticContract(Meaning meaning, ArraySemantics array, ContainerSemantics container) {
  static final SemanticContract NONE = new SemanticContract(null, null, null);
  sealed interface Meaning permits Fixed, Argument, Column, Conditional {}
  record Fixed(String domain) implements Meaning {}
  record Argument(int parameter) implements Meaning {}
  record Column(SlotSource source) implements Meaning {}
  record Conditional(List<Condition> cases) implements Meaning {
    Conditional {
      cases = List.copyOf(cases);
    }
  }

  SemanticContract withMeaning(Meaning value) {
    if (meaning != null)
      throw new IllegalArgumentException("Competing semantic meanings: " + meaning + " and " + value);
    return new SemanticContract(value, array, container);
  }
  SemanticContract withArray(ArraySemantics value) {
    if (array != null)
      throw new IllegalArgumentException("Duplicate semantic array shape");
    return new SemanticContract(meaning, value, container);
  }
  SemanticContract withContainer(ContainerSemantics value) {
    if (container != null)
      throw new IllegalArgumentException("Duplicate semantic container shape");
    return new SemanticContract(meaning, array, value);
  }
  SemanticContract withCondition(Condition value) {
    if (meaning != null && !(meaning instanceof Conditional))
      throw new IllegalArgumentException("Conditional and fixed semantic meanings cannot be combined");
    List<Condition> cases = new ArrayList<>(conditions());
    cases.add(value);
    return new SemanticContract(new Conditional(cases), array, container);
  }
  String domain() {
    return meaning instanceof Fixed fixed ? fixed.domain() : null;
  }
  Integer argument() {
    return meaning instanceof Argument argument ? argument.parameter() : null;
  }
  SlotSource column() {
    return meaning instanceof Column column ? column.source() : null;
  }
  List<Condition> conditions() {
    return meaning instanceof Conditional conditional ? conditional.cases() : List.of();
  }
}
