// Copyright 2000-2021 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.modules.decompiler.semantics;

import java.util.HashSet;
import java.util.List;
import java.util.Set;
import org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticMappings.ArraySemantics;
import static org.jetbrains.java.decompiler.modules.decompiler.semantics.SemanticExpressions.*;

/** Producer alternatives and consumer requirements are kept in separate graph cells. */
record SemanticFacts(Set<String> domains, Set<ArraySemantics> arrays, Set<SemanticMappings.ContainerSemantics> containers,
  Set<DependentDomain> dependent, boolean unknown) {
  sealed interface DependentDomain {}
  record ConditionalDomain(SemanticContext.Key selector, List<SemanticMappings.Condition> conditions) implements DependentDomain {}
  record PackedCase(SemanticContext.Key selector, long value, String domain) {}
  record PackedDomain(List<PackedCase> cases) implements DependentDomain {}

  static final SemanticFacts BOTTOM = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), false);
  static final SemanticFacts UNKNOWN = new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(), true);

  SemanticFacts {
    domains = Set.copyOf(domains);
    arrays = Set.copyOf(arrays);
    containers = Set.copyOf(containers);
    dependent = Set.copyOf(dependent);
  }

  static SemanticFacts of(String domain, ArraySemantics array) {
    return declaration(domain, array, null);
  }

  static SemanticFacts declaration(SemanticContract contract) {
    return declaration(contract.domain(), contract.array(), contract.container());
  }

  static SemanticFacts declaration(String domain, ArraySemantics array, SemanticMappings.ContainerSemantics container) {
    return new SemanticFacts(domain == null ? Set.of() : Set.of(domain), array == null ? Set.of() : Set.of(array),
      container == null ? Set.of() : Set.of(container), Set.of(), domain == null && array == null && container == null);
  }

  static SemanticFacts conditional(SemanticContext.Key selector, List<SemanticMappings.Condition> conditions) {
    return selector == null ? UNKNOWN
                            : new SemanticFacts(Set.of(), Set.of(), Set.of(), Set.of(new ConditionalDomain(selector, conditions)), false);
  }

  SemanticFacts merge(SemanticFacts other) {
    return new SemanticFacts(union(domains, other.domains), union(arrays, other.arrays), union(containers, other.containers),
      union(dependent, other.dependent), unknown || other.unknown);
  }

  static <T> Set<T> union(Set<T> a, Set<T> b) {
    if (a.containsAll(b))
      return a;
    if (b.containsAll(a))
      return b;
    Set<T> result = new HashSet<>(a);
    result.addAll(b);
    return result;
  }

  SemanticFacts require(SemanticFacts requested) {
    SemanticFacts combined = merge(requested);
    if (arrays.isEmpty() || requested.arrays.isEmpty())
      return combined;
    Set<ArraySemantics> shapes = new HashSet<>();
    boolean conflict = false;
    for (ArraySemantics source : arrays)
      for (ArraySemantics demand : requested.arrays) {
        ArraySemantics shape = source.combine(demand);
        if (shape == null)
          conflict = true;
        else
          shapes.add(shape);
      }
    return new SemanticFacts(combined.domains, shapes, combined.containers, combined.dependent, combined.unknown || conflict);
  }

  boolean isEmpty() {
    return equals(BOTTOM);
  }
}
