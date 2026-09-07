package org.jetbrains.java.decompiler.collections;

import org.jetbrains.java.decompiler.modules.decompiler.exps.VarExprent;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.jetbrains.java.decompiler.util.collections.SFormsFastMapDirect;
import org.junit.jupiter.api.Test;

import java.lang.reflect.Field;
import java.util.*;

import static org.junit.jupiter.api.Assertions.*;

class SFormsFastMapDirectTest {
  @Test
  void sequentialGrowthCopiesALinearAmountOfStorage() throws Exception {
    var factory = new FastSparseSetFactory<Integer>(List.of(1));
    int count = 2048;
    for (int segment = 0; segment < 3; segment++) {
      var map = new SFormsFastMapDirect(factory);
      Object[] previous = storage(map)[segment];
      long copiedSlots = 0;
      for (int i = 0; i < count; i++) {
        map.setCurrentVar(key(segment, i), 1);
        Object[] current = storage(map)[segment];
        if (current != previous) copiedSlots += previous.length;
        previous = current;
      }
      // A deterministic complexity check: independent of wall time or the precise growth factor.
      assertTrue(copiedSlots < 6L * count, "Repeated growth copied " + copiedSlots + " slots");
      assertEquals(count, map.size());
      for (int i = 0; i < count; i++) assertEquals(Set.of(1), map.get(key(segment, i)).toPlainSet());
    }
  }

  @Test
  void clearedSegmentsDoNotReserveStorageInUnionOrCopies() throws Exception {
    var factory = new FastSparseSetFactory<Integer>(List.of(1));
    var source = new SFormsFastMapDirect(factory);
    source.setCurrentVar(-4096, 1);
    source.setCurrentVar(VarExprent.STACK_BASE + 4096, 1);
    source.removeAllFields();
    source.removeAllStacks();
    var target = new SFormsFastMapDirect(factory);
    target.union(source);
    assertTrue(target.isEmpty());
    for (Object[] segment : storage(target)) assertEquals(0, segment.length);
    for (Object[] segment : storage(source.getCopy())) assertEquals(0, segment.length);

    source.setCurrentVar(-3, 1);
    target.union(source);
    assertEquals(Set.of(1), target.get(-3).toPlainSet());
    assertTrue(storage(target)[2].length < 32, "Union retained a cleared high field's capacity");
    assertEquals(1, target.size());
  }

  @Test
  void sparseMutationsAndSetOperationsMatchPlainMaps() {
    var factory = new FastSparseSetFactory<Integer>(List.of(1, 2, 3, 4));
    var map = new SFormsFastMapDirect(factory);
    Map<Integer, Set<Integer>> expected = new HashMap<>();
    Random random = new Random(0x5F0A);
    for (int step = 0; step < 300; step++) {
      int key = key(random.nextInt(3), random.nextInt(40) * 3);
      if (random.nextBoolean()) {
        int version = 1 + random.nextInt(4);
        map.setCurrentVar(key, version);
        expected.put(key, new HashSet<>(Set.of(version)));
      } else {
        map.remove(key);
        expected.remove(key);
      }
      assertMap(map, expected);
    }

    var copy = map.getCopy();
    assertTrue(map.entriesEqual(copy));
    var other = new SFormsFastMapDirect(factory);
    Map<Integer, Set<Integer>> otherExpected = new HashMap<>();
    for (int segment = 0; segment < 3; segment++) {
      for (int i = 0; i < 50; i++) {
        int key = key(segment, i * 5);
        other.setCurrentVar(key, 2);
        otherExpected.put(key, Set.of(2));
      }
    }
    for (String operation : List.of("union", "intersection", "complement")) {
      var actual = map.getCopy();
      Map<Integer, Set<Integer>> result = new HashMap<>();
      expected.forEach((k, v) -> result.put(k, new HashSet<>(v)));
      switch (operation) {
        case "union" -> {
          actual.union(other);
          otherExpected.forEach((k, v) -> result.computeIfAbsent(k, unused -> new HashSet<>()).addAll(v));
        }
        case "intersection" -> {
          actual.intersection(other);
          result.forEach((k, v) -> v.retainAll(otherExpected.getOrDefault(k, Set.of())));
        }
        case "complement" -> {
          actual.complement(other);
          result.forEach((k, v) -> v.removeAll(otherExpected.getOrDefault(k, Set.of())));
        }
      }
      result.values().removeIf(Set::isEmpty);
      assertMap(actual, result);
      assertMap(map, expected);
      assertMap(other, otherExpected);
    }
    map.removeAllFields();
    map.removeAllStacks();
    expected.keySet().removeIf(k -> k < 0 || k >= VarExprent.STACK_BASE);
    assertMap(map, expected);
    assertFalse(map.entriesEqual(copy));
  }

  private static int key(int segment, int index) {
    return switch (segment) {
      case 0 -> index;
      case 1 -> VarExprent.STACK_BASE + index;
      default -> -index - 1;
    };
  }

  private static void assertMap(SFormsFastMapDirect map, Map<Integer, Set<Integer>> expected) {
    Map<Integer, Set<Integer>> actual = new HashMap<>();
    for (var entry : map.entryList()) actual.put(entry.getKey(), entry.getValue().toPlainSet());
    assertEquals(expected, actual);
    assertEquals(expected.size(), map.size());
    assertEquals(expected.isEmpty(), map.isEmpty());
    expected.forEach((key, value) -> assertEquals(value, map.get(key).toPlainSet()));
  }

  private static Object[][] storage(SFormsFastMapDirect map) throws Exception {
    Field field = SFormsFastMapDirect.class.getDeclaredField("elements");
    field.setAccessible(true);
    return (Object[][])field.get(map);
  }
}
