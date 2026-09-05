package org.jetbrains.java.decompiler.util.collections;

import java.util.AbstractMap;
import java.util.HashSet;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/** Nullable storage for inference maps. Iteration uses a snapshot while inference updates the map. */
public final class NullableConcurrentHashMap<K, V> extends AbstractMap<K, V> {
  private final ConcurrentHashMap<Optional<K>, Optional<V>> entries = new ConcurrentHashMap<>();

  @Override
  public V get(Object key) {
    Optional<V> value = entries.get(Optional.ofNullable(key));
    return value == null ? null : value.orElse(null);
  }

  @Override
  public V put(K key, V value) {
    Optional<V> previous = entries.put(Optional.ofNullable(key), Optional.ofNullable(value));
    return previous == null ? null : previous.orElse(null);
  }

  @Override
  public boolean containsKey(Object key) {
    return entries.containsKey(Optional.ofNullable(key));
  }

  @Override
  public V remove(Object key) {
    Optional<V> previous = entries.remove(Optional.ofNullable(key));
    return previous == null ? null : previous.orElse(null);
  }

  @Override
  public int size() {
    return entries.size();
  }

  @Override
  public void clear() {
    entries.clear();
  }

  @Override
  public Set<Entry<K, V>> entrySet() {
    Set<Entry<K, V>> snapshot = new HashSet<>();
    entries.forEach((key, value) -> snapshot.add(new SimpleImmutableEntry<>(key.orElse(null), value.orElse(null))));
    return snapshot;
  }
}
