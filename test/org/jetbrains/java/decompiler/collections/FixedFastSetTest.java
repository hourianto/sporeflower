package org.jetbrains.java.decompiler.collections;

import org.jetbrains.java.decompiler.util.collections.fixed.FastFixedSet;
import org.jetbrains.java.decompiler.util.collections.fixed.FastFixedSetFactory;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.Arguments;
import org.junit.jupiter.params.provider.MethodSource;

import java.util.*;
import java.util.stream.IntStream;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.*;

public class FixedFastSetTest {
  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void emptyFullAndClearedSets(String name, List<T> elements) {
    FastFixedSetFactory<T> factory = FastFixedSetFactory.create(elements);
    FastFixedSet<T> empty = factory.createEmptySet();
    FastFixedSet<T> full = factory.createCopiedSet();
    assertState(empty, elements, Set.of());
    assertState(full, elements, new HashSet<>(elements));
    assertEquals(elements, new ArrayList<>(factory.getEntries()));
    assertEquals(empty, empty);
    assertEquals(full, full);
    assertEquals(empty, factory.createEmptySet());
    assertEquals(full, factory.createCopiedSet());
    assertEquals(empty, empty.clone());
    assertEquals(full, full.clone());
    assertEquals(full.hashCode(), full.clone().hashCode());
    assertEquals(elements.isEmpty(), empty.equals(full));
    assertEquals(elements.isEmpty(), full.equals(empty));
    assertFalse(full.toString().isEmpty());

    full.clear();
    assertState(full, elements, Set.of());
    full.setAllElements();
    assertState(full, elements, new HashSet<>(elements));
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void shuffledAddRemoveAndDuplicateOperations(String name, List<T> elements) {
    FastFixedSet<T> set = FastFixedSetFactory.create(elements).createEmptySet();
    Set<T> expected = new HashSet<>();
    List<T> shuffled = shuffled(elements);
    for (T element : shuffled) {
      assertTrue(set.add(element));
      expected.add(element);
      assertFalse(set.add(element));
      // Adding a previously present element must also leave the whole set intact.
      assertFalse(set.add(shuffled.get(0)));
      assertState(set, elements, expected);
    }
    for (T element : shuffled) {
      assertTrue(set.remove(element));
      expected.remove(element);
      assertFalse(set.remove(element));
      assertFalse(set.remove(shuffled.get(0)));
      assertState(set, elements, expected);
    }
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void reverseInsertionAndInterleavedMutations(String name, List<T> elements) {
    FastFixedSet<T> set = FastFixedSetFactory.create(elements).createEmptySet();
    Set<T> expected = new HashSet<>();
    for (int i = elements.size() - 1; i >= 0; i--) set.add(elements.get(i));
    assertState(set, elements, new HashSet<>(elements));
    set.clear();

    List<T> shuffled = shuffled(elements);
    for (int i = 0; i < shuffled.size(); i++) {
      T element = shuffled.get(i);
      assertEquals(expected.add(element), set.add(element));
      if (i % 2 == 0) {
        T removed = shuffled.get(i / 2);
        assertEquals(expected.remove(removed), set.remove(removed));
      }
      assertState(set, elements, expected);
    }
    for (T element : shuffled) {
      assertEquals(expected.add(element), set.add(element));
      assertState(set, elements, expected);
    }
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void iteratorRemovalAndIndependentCopies(String name, List<T> elements) {
    FastFixedSet<T> set = FastFixedSetFactory.create(elements).createCopiedSet();
    Set<T> expected = new HashSet<>(elements);
    Iterator<T> iterator = set.iterator();
    int index = 0;
    while (iterator.hasNext()) {
      T element = iterator.next();
      if (index++ % 2 == 0) {
        iterator.remove();
        expected.remove(element);
        assertFalse(set.contains(element));
      }
    }
    assertState(set, elements, expected);
    FastFixedSet<T> copy = set.clone();
    assertEquals(set, copy);
    assertEquals(set.hashCode(), copy.hashCode());
    // Check both directions: neither copy may share mutable storage with the other.
    set.setAllElements();
    assertState(copy, elements, expected);
    copy.clear();
    assertState(set, elements, new HashSet<>(elements));
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void setsWithDifferentSizesOrElementsAreUnequal(String name, List<T> elements) {
    FastFixedSetFactory<T> factory = FastFixedSetFactory.create(elements);
    FastFixedSet<T> left = factory.createEmptySet();
    FastFixedSet<T> right = factory.createEmptySet();
    for (int i = 1; i < elements.size(); i++) {
      left.add(elements.get(i));
      assertNotEquals(left, right);
      assertNotEquals(right, left);
      right.add(elements.get(i - 1));
      assertNotEquals(left, right);
      assertNotEquals(right, left);
    }
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void bulkOperationsMatchPlainSets(String name, List<T> elements) {
    FastFixedSetFactory<T> factory = FastFixedSetFactory.create(elements);
    FastFixedSet<T> left = factory.createEmptySet();
    FastFixedSet<T> right = factory.createEmptySet();
    Set<T> expectedLeft = new HashSet<>();
    Set<T> expectedRight = new HashSet<>();
    for (int i = 0; i < elements.size(); i++) {
      T element = elements.get(i);
      if (i % 3 != 0) { left.add(element); expectedLeft.add(element); }
      if (i % 3 != 1) { right.add(element); expectedRight.add(element); }
    }

    // Exercise both the specialized bitwise overloads (via Collection dispatch)
    // and the ordinary collection fallback, with overlapping and disjoint bits.
    for (Collection<T> operand : List.of(right, expectedRight)) {
      assertEquals(expectedLeft.containsAll(expectedRight), left.containsAll(operand));
      assertTrue(left.containsAll(List.of()));
      FastFixedSet<T> union = left.clone();
      Set<T> expected = new HashSet<>(expectedLeft);
      assertEquals(expected.addAll(expectedRight), union.addAll(operand));
      assertFalse(union.addAll(operand));
      assertState(union, elements, expected);
      assertTrue(union.containsAll(operand));

      FastFixedSet<T> intersection = left.clone();
      expected = new HashSet<>(expectedLeft);
      assertEquals(expected.retainAll(expectedRight), intersection.retainAll(operand));
      assertFalse(intersection.retainAll(operand));
      assertState(intersection, elements, expected);

      FastFixedSet<T> difference = left.clone();
      expected = new HashSet<>(expectedLeft);
      assertEquals(expected.removeAll(expectedRight), difference.removeAll(operand));
      assertFalse(difference.removeAll(operand));
      assertState(difference, elements, expected);
    }
    assertState(left, elements, expectedLeft);
    assertState(right, elements, expectedRight);
  }

  @ParameterizedTest(name = "{displayName}: {0}")
  @MethodSource("universes")
  <T> void repeatedBitFlipsLeaveSquareIndices(String name, List<T> elements) {
    FastFixedSet<T> set = FastFixedSetFactory.create(elements).createEmptySet();
    if (!elements.isEmpty()) set.add(elements.get(0));
    for (int step = 1; step < elements.size(); step++) {
      for (int i = step; i < elements.size(); i += step) {
        T element = elements.get(i);
        if (set.contains(element)) set.remove(element);
        else set.add(element);
      }
    }
    Set<T> expected = new HashSet<>();
    for (int i = 0; i * i < elements.size(); i++) expected.add(elements.get(i * i));
    assertState(set, elements, expected);
  }

  private static <T> void assertState(FastFixedSet<T> set, List<T> universe, Set<T> expected) {
    assertEquals(expected.size(), set.size());
    assertEquals(expected.isEmpty(), set.isEmpty());
    assertEquals(expected, set.toPlainSet());
    Iterator<T> guarded = set.iterator();
    Iterator<T> unguarded = set.iterator();
    // Exact sequence comparison catches omissions, duplicates and order changes.
    // The previous subsequence checks could silently accept an empty iterator.
    for (T element : universe) {
      assertEquals(expected.contains(element), set.contains(element));
      if (expected.contains(element)) {
        assertTrue(guarded.hasNext());
        assertEquals(element, guarded.next());
        assertEquals(element, unguarded.next());
      }
    }
    assertFalse(guarded.hasNext());
    assertFalse(unguarded.hasNext());
    // These internal iterators currently return null on exhaustion.
    assertNull(guarded.next());
    assertNull(unguarded.next());
  }

  private static <T> List<T> shuffled(List<T> elements) {
    List<T> copy = new ArrayList<>(elements);
    Collections.shuffle(copy, new Random(0x50_B1A5ED));
    return copy;
  }

  private static Stream<Arguments> universes() {
    // Cover empty universes, the short/long implementation switch, and full and
    // partial final words. Large universes still run every mutation assertion.
    Stream<Arguments> integers = IntStream.of(0, 1, 3, 10, 16, 20, 63, 64, 65, 127, 128, 129, 1023, 1024, 1025, 1030)
      .mapToObj(size -> Arguments.of("integers-" + size, IntStream.range(0, size).boxed().toList()));
    return Stream.concat(integers, Stream.of(
      Arguments.of("signed-keys", List.of(0, 0x1000, 0x2000, 0x4000, 0x8000, 0x10000, 0x100000, 0x1000000,
        0x10000000, 0x20000000, 0x40000000, Integer.MIN_VALUE, 0xF0000000, 0xFFFF0000, 0xFFFF0001, -1)),
      Arguments.of("strings-1024", IntStream.range(0, 1024).mapToObj(i -> "key-" + i).toList())
    ));
  }
}
