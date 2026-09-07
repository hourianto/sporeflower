// Copyright 2000-2020 JetBrains s.r.o. Use of this source code is governed by the Apache 2.0 license that can be found in the LICENSE file.
package org.jetbrains.java.decompiler.util.collections;

import java.util.*;

public class FastSparseSetFactory<E> {

  private final PackedMap<E> colValuesInternal = new PackedMap<>();

  public FastSparseSetFactory(Collection<? extends E> set) {
    for (E element : set) {
      if (!colValuesInternal.containsKey(element)) addElement(element);
    }
  }

  private long addElement(E element) {
    // Each distinct element owns one bit, at the same index as its registry key.
    int index = colValuesInternal.size();
    return colValuesInternal.putWithKey(1 << (index & 31), index >>> 5, element);
  }

  public FastSparseSet<E> createEmptySet() {
    return new FastSparseSet<>(this);
  }

  /** Mutable version set, with allocation-free bitmap storage when all members occupy one word. */
  public static final class FastSparseSet<E> implements Iterable<E> {
    public static final FastSparseSet[] EMPTY_ARRAY = new FastSparseSet[0];

    private final FastSparseSetFactory<E> factory;
    // data == null: word is stored inline at activeLength - 1 (or the set is empty).
    // Otherwise at least two words are occupied, and next links skip empty words.
    private int word;
    private int[] data;
    private int[] next;
    private int activeLength;

    private FastSparseSet(FastSparseSetFactory<E> factory) {
      this.factory = factory;
    }

    public FastSparseSet<E> getCopy() {
      FastSparseSet<E> copy = new FastSparseSet<>(factory);
      copy.word = word;
      copy.activeLength = activeLength;
      if (data != null) {
        copy.data = Arrays.copyOf(data, activeLength);
        copy.next = Arrays.copyOf(next, activeLength);
      }
      return copy;
    }

    public void add(E element) {
      PackedMap<E> values = factory.colValuesInternal;
      long index = values.containsKey(element) ? values.getWithKey(element) : factory.addElement(element);
      int block = PackedMap.unpackLow(index);
      putWord(block, wordAt(block) | PackedMap.unpackHigh(index));
    }

    public void remove(E element) {
      PackedMap<E> values = factory.colValuesInternal;
      if (!values.containsKey(element)) return;
      long index = values.getWithKey(element);
      int block = PackedMap.unpackLow(index);
      putWord(block, wordAt(block) & ~PackedMap.unpackHigh(index));
      compact();
    }

    public boolean contains(E element) {
      PackedMap<E> values = factory.colValuesInternal;
      if (!values.containsKey(element)) return false;
      long index = values.getWithKey(element);
      return (wordAt(PackedMap.unpackLow(index)) & PackedMap.unpackHigh(index)) != 0;
    }

    private int wordAt(int block) {
      if (data == null) return block == activeLength - 1 ? word : 0;
      return block < activeLength ? data[block] : 0;
    }

    private void putWord(int block, int value) {
      if (data == null) {
        if (word == 0 || block == activeLength - 1) {
          word = value;
          activeLength = value == 0 ? 0 : block + 1;
          return;
        }
        if (value == 0) return;
        // A second occupied word promotes the inline value to a sparse bitmap.
        int length = Math.max(activeLength, block + 1);
        data = new int[length];
        next = new int[length];
        data[activeLength - 1] = word;
        Arrays.fill(next, 0, activeLength - 1, activeLength - 1);
        word = 0;
      }
      if (block >= data.length) {
        if (value == 0) return;
        int capacity = (int)Math.min(Integer.MAX_VALUE, Math.max((long)block + 1, (long)data.length * 2));
        data = Arrays.copyOf(data, capacity);
        next = Arrays.copyOf(next, capacity);
      }
      int previous = data[block];
      if (previous == value) return;
      data[block] = value;
      if (previous == 0) {
        changeNext(block, next[block], block);
        activeLength = Math.max(activeLength, block + 1);
      } else if (value == 0) {
        changeNext(block, block, next[block]);
        if (block + 1 == activeLength) {
          while (activeLength > 0 && data[activeLength - 1] == 0) activeLength--;
        }
      }
    }

    private void changeNext(int block, int oldNext, int newNext) {
      for (int i = block - 1; i >= 0 && next[i] == oldNext; i--) next[i] = newNext;
    }

    // Removing words may leave a single word (possibly far above word zero).
    // Collapse only after the operation, so its traversal can still use the old indices.
    private void compact() {
      if (data == null) return;
      int first = data[0] == 0 ? next[0] : 0;
      if (next[first] == 0) {
        word = data[first];
        activeLength = word == 0 ? 0 : first + 1;
        data = null;
        next = null;
      }
    }

    public void union(FastSparseSet<E> set) {
      if (set == this || set.activeLength == 0) return;
      if (set.data == null) {
        int block = set.activeLength - 1;
        putWord(block, wordAt(block) | set.word);
        return;
      }
      int block = set.data[0] == 0 ? set.next[0] : 0;
      do {
        putWord(block, wordAt(block) | set.data[block]);
        block = set.next[block];
      } while (block != 0);
    }

    public void intersection(FastSparseSet<E> set) {
      filter(set, true);
    }

    public void complement(FastSparseSet<E> set) {
      filter(set, false);
    }

    private void filter(FastSparseSet<E> set, boolean intersection) {
      if (data == null) {
        if (word != 0) {
          int other = set.wordAt(activeLength - 1);
          word &= intersection ? other : ~other;
          if (word == 0) activeLength = 0;
        }
        return;
      }
      int block = data[0] == 0 ? next[0] : 0;
      boolean removedWord = false;
      do {
        int other = set.wordAt(block);
        data[block] &= intersection ? other : ~other;
        removedWord |= data[block] == 0;
        block = next[block];
      } while (block != 0);
      if (removedWord) {
        // Repair links once after bulk removal. Updating each removed word's
        // predecessors separately would repeatedly rewrite the same empty prefix.
        int following = 0;
        int length = 0;
        for (int i = activeLength - 1; i >= 0; i--) {
          next[i] = following;
          if (data[i] != 0) {
            if (following == 0) length = i + 1;
            following = i;
          }
        }
        activeLength = length;
        compact();
      }
    }

    @Override
    public int hashCode() {
      int hash = 0;
      for (E element : this) hash += Objects.hashCode(element);
      return hash;
    }

    @Override
    public boolean equals(Object object) {
      if (object == this) return true;
      if (!(object instanceof FastSparseSet<?> other) || activeLength != other.activeLength) return false;
      if (data == null || other.data == null) return data == other.data && word == other.word;
      return Arrays.equals(data, 0, activeLength, other.data, 0, activeLength);
    }

    /** Returns 0, 1, or 2, where 2 means at least two members. */
    public int getCardinality() {
      if (data != null) return 2;
      return word == 0 ? 0 : (word & (word - 1)) == 0 ? 1 : 2;
    }

    public boolean isEmpty() {
      return activeLength == 0;
    }

    @Override
    public Iterator<E> iterator() {
      return new FastSparseSetIterator<>(this);
    }

    private int nextIndex(int from) {
      int block = from >>> 5;
      if (block >= activeLength) return -1;
      int bits;
      if (data == null) {
        int inlineBlock = activeLength - 1;
        bits = block == inlineBlock ? word & (-1 << (from & 31)) : word;
        block = inlineBlock;
      } else {
        bits = data[block] & (-1 << (from & 31));
        if (bits == 0) {
          block = next[block];
          if (block == 0) return -1;
          bits = data[block];
        }
      }
      return bits == 0 ? -1 : (block << 5) + Integer.numberOfTrailingZeros(bits);
    }

    public Set<E> toPlainSet() {
      Set<E> result = new HashSet<>();
      // Preserve the historical descending insertion order, including collisions
      // in the returned HashSet. Visit occupied bits instead of the whole universe.
      for (int block = activeLength - 1; block >= 0; block--) {
        int bits = wordAt(block);
        while (bits != 0) {
          int bit = 31 - Integer.numberOfLeadingZeros(bits);
          int index = (block << 5) + bit;
          if (index < factory.colValuesInternal.size()) result.add(factory.colValuesInternal.getKey(index));
          bits &= ~(1 << bit);
        }
        if (data == null) break;
      }
      return result;
    }

    @Override
    public String toString() {
      return toPlainSet().toString();
    }
  }

  public static final class FastSparseSetIterator<E> implements Iterator<E> {
    private final FastSparseSet<E> set;
    private final PackedMap<E> values;
    private final int size;
    private int pointer = -1;
    private int nextPointer = -1;
    private boolean canRemove;

    private FastSparseSetIterator(FastSparseSet<E> set) {
      this.set = set;
      values = set.factory.colValuesInternal;
      size = values.size();
    }

    @Override
    public boolean hasNext() {
      nextPointer = set.nextIndex(pointer + 1);
      return nextPointer >= 0 && nextPointer < size;
    }

    @Override
    public E next() {
      if (nextPointer < 0) hasNext();
      if (nextPointer < 0 || nextPointer >= size) {
        pointer = size;
        canRemove = false;
        return null; // Retain the historical exhaustion convention.
      }
      pointer = nextPointer;
      nextPointer = -1;
      canRemove = true;
      return values.getKey(pointer);
    }

    @Override
    public void remove() {
      if (!canRemove) throw new IllegalStateException();
      int block = pointer >>> 5;
      set.putWord(block, set.wordAt(block) & ~(1 << (pointer & 31)));
      set.compact();
      canRemove = false;
    }
  }
}
