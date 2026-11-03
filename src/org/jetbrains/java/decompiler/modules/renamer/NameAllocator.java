package org.jetbrains.java.decompiler.modules.renamer;

import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.function.Function;
import java.util.function.Supplier;

/** Shared allocation policy; callers define member identity and source visibility. */
final class NameAllocator {
  private NameAllocator() { }

  static Scope reserve(Collection<? extends Slot> members) {
    Scope scope = new Scope();
    for (Slot member : members) {
      if (member.name != null) scope.reserve(member.name);
      member.scopes.add(scope);
    }
    return scope;
  }

  static <T extends Slot> void allocate(Collection<T> members, Comparator<T> tieBreak, Function<T, String> nextName) {
    List<T> ordered = new ArrayList<>(members);
    // Pin library/native contracts, then place original and explicitly requested
    // names before generating replacements. Stable sorting preserves input ties.
    ordered.sort((first, second) -> {
      int firstTier = first.fixed ? 0 : first.name != null ? 1 : 2;
      int secondTier = second.fixed ? 0 : second.name != null ? 1 : 2;
      int priority = Integer.compare(firstTier, secondTier);
      // The tie-break chooses which explicit spelling survives a collision.
      // Generated candidates keep declaration order and its stable counters.
      return priority != 0 ? priority : firstTier == 1 ? tieBreak.compare(first, second) : 0;
    });
    for (T member : ordered) {
      member.allocate(() -> nextName.apply(member));
    }
  }

  static class Slot {
    private final List<Scope> scopes = new ArrayList<>();
    // Null means a generated name is needed; generators run only after all
    // explicit requests have been collected and reserved across their scopes.
    String name;
    boolean fixed;

    final void allocate(Supplier<String> nextName) {
      if (name == null || !fixed && scopes.stream().anyMatch(scope -> scope.assignedNames.contains(name))) {
        rename(nextName, Set.of());
      } else {
        reserveAssignedName();
      }
    }

    void rename(Supplier<String> nextName, Set<String> blocked) {
      do {
        name = nextName.get();
      } while (blocked.contains(name) || scopes.stream().anyMatch(scope -> scope.reservedNames.contains(name)));
      reserveAssignedName();
    }

    private void reserveAssignedName() {
      for (Scope scope : scopes) {
        scope.assignedNames.add(name);
        scope.reservedNames.add(name);
      }
    }
  }

  static final class Scope {
    // Shared scopes avoid quadratic conflict edges. Reservations are monotonic:
    // a fallback cannot steal a later request in another scope, and a qualifier
    // repair cannot reuse a name still relevant to a previously checked class.
    private final Set<String> reservedNames = new HashSet<>();
    private final Set<String> assignedNames = new HashSet<>();

    void reserve(String name) { reservedNames.add(name); }
  }
}
