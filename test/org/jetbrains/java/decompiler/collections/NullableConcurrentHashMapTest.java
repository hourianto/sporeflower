package org.jetbrains.java.decompiler.collections;

import org.jetbrains.java.decompiler.util.collections.NullableConcurrentHashMap;
import org.junit.jupiter.api.Test;

import java.util.AbstractMap.SimpleImmutableEntry;
import java.util.Map;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

class NullableConcurrentHashMapTest {
  @Test
  void nullMappingsRoundTrip() {
    Map<String, String> map = new NullableConcurrentHashMap<>();
    assertNull(map.put(null, "first"));
    assertEquals("first", map.put(null, null));
    assertTrue(map.containsKey(null));
    assertNull(map.get(null));

    assertNull(map.put("T", null));
    assertTrue(map.containsKey("T"));
    assertNull(map.put("T", "value"));
    assertEquals("value", map.remove("T"));
    assertNull(map.remove(null));
    assertTrue(map.isEmpty());
  }

  @Test
  void iterationRetainsItsSnapshotWhileInferenceUpdatesValues() {
    Map<String, String> map = new NullableConcurrentHashMap<>();
    map.put(null, null);
    map.put("T", "first");
    Set<Map.Entry<String, String>> snapshot = map.entrySet();

    for (Map.Entry<String, String> entry : snapshot) {
      map.put(entry.getKey(), "updated");
    }

    assertEquals(Set.of(new SimpleImmutableEntry<>(null, null), new SimpleImmutableEntry<>("T", "first")), snapshot);
    assertEquals("updated", map.get(null));
    assertEquals("updated", map.get("T"));
    map.clear();
    assertTrue(map.isEmpty());
    assertEquals(2, snapshot.size());
  }
}
