package org.jetbrains.java.decompiler.collections;

import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory;
import org.jetbrains.java.decompiler.util.collections.FastSparseSetFactory.FastSparseSet;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.TestFactory;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.Arguments;
import org.junit.jupiter.params.provider.MethodSource;

import java.util.*;
import java.util.stream.IntStream;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.*;

public class FastSparseSetTest {
  @Test
  void duplicateUniverseEntriesShareOneBit() {
    var factory = new FastSparseSetFactory<Integer>(IntStream.range(0, 80).map(i -> i % 40).boxed().toList());
    var set = factory.createEmptySet();
    List<Integer> elements = List.of(0, 31, 32, 39, 40);
    elements.forEach(set::add);
    assertEquals(new HashSet<>(elements), set.toPlainSet());
    iteratorVisitsInOrder(set, elements);
    assertEquals(new HashSet<>(elements).hashCode(), set.hashCode());
  }

  @Test
  void intersectionRepairsLinksBeforeCopyingOrMergingTheResult() {
    var factory = new FastSparseSetFactory<Integer>(IntStream.range(0, 160).boxed().toList());
    var set = factory.createEmptySet();
    set.add(64);
    set.add(128);
    var other = factory.createEmptySet();
    other.add(1);
    set.intersection(other);
    assertTrue(set.isEmpty());
    var copy = set.getCopy();
    assertFalse(copy.iterator().hasNext());
    other.union(copy);
    assertEquals(Set.of(1), other.toPlainSet());
  }

  @Test
  void iteratorRemovalRepairsLinksAndCopyBounds() {
    var factory = new FastSparseSetFactory<Integer>(IntStream.range(0, 160).boxed().toList());
    var set = factory.createEmptySet();
    set.add(128);
    var iterator = set.iterator();
    assertEquals(128, iterator.next());
    iterator.remove();
    assertTrue(set.isEmpty());
    assertEquals(0, set.getCardinality());
    var copy = set.getCopy();
    copy.add(1);
    assertEquals(Set.of(1), copy.toPlainSet());
    iteratorVisitsInOrder(copy, List.of(1));
  }

  @Test
  void branchingCopiesAndMixedMutationsMatchIndependentSets() {
    var factory = new FastSparseSetFactory<Integer>(IntStream.range(0, 4096).boxed().toList());
    List<FastSparseSet<Integer>> sets = new ArrayList<>();
    List<Set<Integer>> expected = new ArrayList<>();
    for (int i = 0; i < 12; i++) {
      sets.add(factory.createEmptySet());
      expected.add(new HashSet<>());
    }
    Random random = new Random(0x51A5E7);
    for (int step = 0; step < 3_000; step++) {
      int target = random.nextInt(sets.size()), source = random.nextInt(sets.size());
      int element = step % 3 == 0 ? random.nextInt(5000) : 128 + random.nextInt(32);
      var set = sets.get(target);
      var reference = expected.get(target);
      switch (random.nextInt(8)) {
        case 0 -> { set.add(element); reference.add(element); }
        case 1 -> { set.remove(element); reference.remove(element); }
        case 2 -> {
          sets.set(target, sets.get(source).getCopy());
          expected.set(target, new HashSet<>(expected.get(source)));
        }
        case 3 -> { set.union(sets.get(source)); reference.addAll(expected.get(source)); }
        case 4 -> { set.intersection(sets.get(source)); reference.retainAll(expected.get(source)); }
        case 5 -> { set.complement(sets.get(source)); reference.removeAll(expected.get(source)); }
        case 6 -> {
          var iterator = set.iterator();
          if (iterator.hasNext()) {
            reference.remove(iterator.next());
            iterator.remove();
          }
        }
        case 7 -> { set.complement(set); reference.clear(); }
      }
      for (int i = 0; i < sets.size(); i++) {
        assertEquals(expected.get(i), sets.get(i).toPlainSet(), "step=" + step + ", set=" + i);
        assertEquals(expected.get(i).isEmpty(), sets.get(i).isEmpty());
        assertEquals(Math.min(2, expected.get(i).size()), sets.get(i).getCardinality());
        iteratorVisitsEachOnce(sets.get(i), expected.get(i));
      }
    }
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void membershipMutationAndCopies(String name, List<T> elements, List<T> additions) {
    var factory = new FastSparseSetFactory<T>(elements);
    var set = factory.createEmptySet();
    Set<T> expected = new HashSet<>();
    assertState(set, elements, expected);
    assertFalse(set.toString().isEmpty());
    for (T element : additions) {
      assertFalse(set.contains(element));
      set.remove(element);
    }
    assertState(set.getCopy(), elements, expected);

    List<T> shuffled = new ArrayList<>(elements);
    Collections.shuffle(shuffled, newRandom());
    for (T element : shuffled) {
      set.add(element);
      set.add(element);
      expected.add(element);
      assertTrue(set.contains(element));
    }
    assertState(set, elements, expected);
    assertFalse(set.toString().isEmpty());
    var copy = set.getCopy();
    assertEquals(set, copy);
    assertEquals(set.hashCode(), copy.hashCode());

    // Grow the shared universe after taking the copy. Membership checks and
    // removals above must not have allocated indices for these absent keys.
    List<T> universe = new ArrayList<>(elements);
    for (T element : additions) {
      set.add(element);
      universe.add(element);
      expected.add(element);
      assertState(set, universe, expected);
    }
    assertState(copy, universe, new HashSet<>(elements));
    assertNotEquals(set, copy);
    assertNotEquals(copy, set);

    var iterator = set.iterator();
    int index = 0;
    while (iterator.hasNext()) {
      T element = iterator.next();
      if (index++ % 2 == 0) {
        iterator.remove();
        expected.remove(element);
        assertFalse(set.contains(element));
      }
    }
    assertState(set, universe, expected);
    assertState(copy, universe, new HashSet<>(elements));
    for (T element : universe) {
      set.remove(element);
      set.remove(element);
      expected.remove(element);
      assertFalse(set.contains(element));
    }
    assertState(set, universe, Set.of());
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void setOperationsPreserveOperandsAndUniverseOrder(String name, List<T> elements, List<T> additions) {
    var factory = new FastSparseSetFactory<T>(elements);
    var left = factory.createEmptySet();
    var right = factory.createEmptySet();
    for (int i = 0; i < elements.size(); i++) {
      if (i % 3 != 0) left.add(elements.get(i));
      if (i % 3 != 1) right.add(elements.get(i));
    }
    // Include new indices beyond the original factory capacity and exercise
    // both growing and non-growing operand directions.
    additions.forEach(right::add);
    List<T> universe = new ArrayList<>(elements);
    universe.addAll(additions);
    checkOperations(left, right, universe);
    checkOperations(right, left, universe);
    checkOperations(left, left, universe);
    checkOperations(left, factory.createEmptySet(), universe);
  }

  @Test
  void plainSetToleratesFactoryGrowthByOtherSets() {
    var factory = new FastSparseSetFactory<Integer>(List.of());
    var oldEmptySet = factory.createEmptySet();
    var growingSet = factory.createEmptySet();
    List<Integer> universe = IntStream.range(0, 128).boxed().toList();
    universe.forEach(growingSet::add);
    assertState(oldEmptySet, universe, Set.of());
    assertState(oldEmptySet.getCopy(), universe, Set.of());
    assertState(growingSet, universe, new HashSet<>(universe));
  }

  @TestFactory
  Stream<DynamicTest> operationsOnVerySparseSets() {
    // Share only the million-entry index, never the mutable sets.
    List<Integer> universe = IntStream.range(0, 1_000_000).boxed().toList();
    var factory = new FastSparseSetFactory<Integer>(universe);
    return IntStream.of(10, 100, 1000, 10000).boxed().flatMap(sparsity ->
      IntStream.of(0, 10, 100).mapToObj(overlap -> DynamicTest.dynamicTest(
        "sparsity=" + sparsity + ", overlap=" + overlap,
        () -> {
          Random random = newRandom();
          var left = factory.createEmptySet();
          var right = factory.createEmptySet();
          List<Integer> populated = new ArrayList<>();
          for (int i = 0; i < universe.size(); i++) {
            if (random.nextInt(sparsity) == 0) {
              left.add(i);
              populated.add(i);
              if (overlap != 0 && random.nextInt(overlap) == 0) right.add(i);
            } else if (random.nextInt(sparsity - 1) == 0) {
              right.add(i);
              populated.add(i);
            }
          }
          // Only populated indices are needed to check exact iteration order.
          checkOperations(left, right, populated);
        })));
  }

  private static <T> void checkOperations(FastSparseSet<T> left, FastSparseSet<T> right, List<T> universe) {
    Set<T> beforeLeft = left.toPlainSet(), beforeRight = right.toPlainSet();
    var union = left.getCopy();
    union.union(right);
    Set<T> expected = new HashSet<>(beforeLeft);
    expected.addAll(beforeRight);
    assertState(union, universe, expected);

    var intersection = left.getCopy();
    intersection.intersection(right);
    expected = new HashSet<>(beforeLeft);
    expected.retainAll(beforeRight);
    assertState(intersection, universe, expected);

    var difference = left.getCopy();
    difference.complement(right);
    expected = new HashSet<>(beforeLeft);
    expected.removeAll(beforeRight);
    assertState(difference, universe, expected);
    assertState(left, universe, beforeLeft);
    assertState(right, universe, beforeRight);
  }

  private static <T> void assertState(FastSparseSet<T> set, List<T> universe, Set<T> expected) {
    assertEquals(expected, set.toPlainSet());
    assertEquals(expected.isEmpty(), set.isEmpty());
    assertEquals(Math.min(2, expected.size()), set.getCardinality());
    for (T element : universe) assertEquals(expected.contains(element), set.contains(element));
    iteratorVisitsInOrder(set, universe.stream().filter(expected::contains).toList());
  }

  private static <T> void iteratorVisitsEachOnce(FastSparseSet<T> set, Collection<T> expected) {
    Set<T> remaining = new HashSet<>(expected);
    for (T element : set) assertTrue(remaining.remove(element), "Repeated or unexpected element: " + element);
    assertTrue(remaining.isEmpty(), "Iterator omitted elements: " + remaining);
  }

  private static <T> void iteratorVisitsInOrder(FastSparseSet<T> set, List<T> expected) {
    List<T> actual = new ArrayList<>();
    Iterator<T> iterator = set.iterator();
    while (iterator.hasNext()) actual.add(iterator.next());
    assertEquals(expected, actual);
    assertNull(iterator.next());
  }

  private static Random newRandom() {
    return new Random(0x50_B1A5ED);
  }

  private static Stream<Arguments> universes() {
    // Boundaries are determined by insertion index, not by the key value.
    Stream<Arguments> integers = IntStream.of(0, 1, 31, 32, 33, 1024).mapToObj(size ->
      Arguments.of("integers-" + size, IntStream.range(0, size).boxed().toList(),
        IntStream.range(size, size + 80).boxed().toList()));
    return Stream.concat(integers, Stream.of(
      Arguments.of("signed-keys", List.of(0, 0x1000, 0x2000, 0x4000, 0x8000, 0x10000, 0x100000, 0x1000000,
        0x10000000, 0x20000000, 0x40000000, Integer.MIN_VALUE, 0xF0000000, 0xFFFF0000, 0xFFFF0001, -1),
        IntStream.range(1, 81).boxed().toList()),
      Arguments.of("strings-empty", List.of(), IntStream.range(0, 80).mapToObj(i -> "extra-" + i).toList()),
      Arguments.of("strings-1024", IntStream.range(0, 1024).mapToObj("A"::repeat).toList(),
        IntStream.range(0, 80).mapToObj(i -> "extra-" + i).toList())
    ));
  }
}
