package org.jetbrains.java.decompiler.struct.attr;

import org.jetbrains.java.decompiler.code.BytecodeVersion;
import org.jetbrains.java.decompiler.struct.consts.ConstantPool;
import org.jetbrains.java.decompiler.struct.gen.VarType;
import org.jetbrains.java.decompiler.util.DataInputFullStream;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.NavigableMap;
import java.util.TreeMap;

/**
 * Legacy CLDC StackMap attribute.
 *
 * <p>Format:
 * <pre>
 * u2 number_of_entries;
 * stack_map_frame entries[number_of_entries];
 *
 * stack_map_frame {
 *   u2 offset;
 *   u2 number_of_locals;
 *   verification_type_info locals[number_of_locals];
 *   u2 number_of_stack_items;
 *   verification_type_info stack[number_of_stack_items];
 * }
 * </pre>
 */
public class StructStackMapAttribute extends StructGeneralAttribute {
  private static final int ITEM_Top = 0;
  private static final int ITEM_Integer = 1;
  private static final int ITEM_Float = 2;
  private static final int ITEM_Double = 3;
  private static final int ITEM_Long = 4;
  private static final int ITEM_Null = 5;
  private static final int ITEM_UninitializedThis = 6;
  private static final int ITEM_Object = 7;
  private static final int ITEM_Uninitialized = 8;

  private List<StackMapEntry> entries = Collections.emptyList();
  private NavigableMap<Integer, StackMapEntry> entriesByOffset = Collections.emptyNavigableMap();

  @Override
  public void initContent(DataInputFullStream data, ConstantPool pool, BytecodeVersion version) throws IOException {
    int entryCount = data.readUnsignedShort();
    if (entryCount == 0) {
      entries = Collections.emptyList();
      entriesByOffset = Collections.emptyNavigableMap();
      return;
    }

    List<StackMapEntry> parsedEntries = new ArrayList<>(entryCount);
    NavigableMap<Integer, StackMapEntry> byOffset = new TreeMap<>();
    for (int i = 0; i < entryCount; i++) {
      int offset = data.readUnsignedShort();
      List<VerificationTypeInfo> locals = readVerificationTypes(data, pool, data.readUnsignedShort());
      List<VerificationTypeInfo> stack = readVerificationTypes(data, pool, data.readUnsignedShort());
      StackMapEntry entry = new StackMapEntry(offset, locals, stack);
      parsedEntries.add(entry);
      byOffset.put(offset, entry);
    }

    entries = Collections.unmodifiableList(parsedEntries);
    entriesByOffset = Collections.unmodifiableNavigableMap(byOffset);
  }

  public List<StackMapEntry> getEntries() {
    return entries;
  }

  public VarType getLocalTypeExact(int offset, int localIndex) {
    StackMapEntry frame = entriesByOffset.get(offset);
    return frame == null ? null : frame.getLocalType(localIndex);
  }

  public VarType getLocalType(int offset, int localIndex) {
    Map.Entry<Integer, StackMapEntry> frame = entriesByOffset.floorEntry(offset);
    if (frame == null) {
      return null;
    }
    return frame.getValue().getLocalType(localIndex);
  }

  private static List<VerificationTypeInfo> readVerificationTypes(DataInputFullStream data, ConstantPool pool, int count) throws IOException {
    if (count == 0) {
      return Collections.emptyList();
    }

    List<VerificationTypeInfo> result = new ArrayList<>(count);
    for (int i = 0; i < count; i++) {
      result.add(readVerificationType(data, pool));
    }
    return Collections.unmodifiableList(result);
  }

  private static VerificationTypeInfo readVerificationType(DataInputFullStream data, ConstantPool pool) throws IOException {
    int tag = data.readUnsignedByte();
    switch (tag) {
      case ITEM_Top:
        return new VerificationTypeInfo(tag, null);
      case ITEM_Integer:
        return new VerificationTypeInfo(tag, VarType.VARTYPE_INT);
      case ITEM_Float:
        return new VerificationTypeInfo(tag, VarType.VARTYPE_FLOAT);
      case ITEM_Double:
        return new VerificationTypeInfo(tag, VarType.VARTYPE_DOUBLE);
      case ITEM_Long:
        return new VerificationTypeInfo(tag, VarType.VARTYPE_LONG);
      case ITEM_Null:
        return new VerificationTypeInfo(tag, VarType.VARTYPE_NULL);
      case ITEM_UninitializedThis:
        return new VerificationTypeInfo(tag, VarType.VARTYPE_OBJECT);
      case ITEM_Object:
        return new VerificationTypeInfo(tag, new VarType(pool.getPrimitiveConstant(data.readUnsignedShort()).getString(), true));
      case ITEM_Uninitialized:
        data.readUnsignedShort();
        return new VerificationTypeInfo(tag, null);
      default:
        throw new IOException("Unknown StackMap verification type tag: " + tag);
    }
  }

  private static int slotSize(int tag) {
    return tag == ITEM_Double || tag == ITEM_Long ? 2 : 1;
  }

  private static final class VerificationTypeInfo {
    private final int tag;
    private final VarType type;

    private VerificationTypeInfo(int tag, VarType type) {
      this.tag = tag;
      this.type = type;
    }

    private int slotSize() {
      return StructStackMapAttribute.slotSize(tag);
    }
  }

  public static final class StackMapEntry {
    private final int offset;
    private final List<VerificationTypeInfo> locals;
    private final List<VerificationTypeInfo> stack;
    private final Map<Integer, VarType> localsBySlot;

    private StackMapEntry(int offset, List<VerificationTypeInfo> locals, List<VerificationTypeInfo> stack) {
      this.offset = offset;
      this.locals = locals;
      this.stack = stack;
      this.localsBySlot = buildLocalsBySlot(locals);
    }

    public int getOffset() {
      return offset;
    }

    public List<VarType> getLocals() {
      List<VarType> types = new ArrayList<>(locals.size());
      for (VerificationTypeInfo local : locals) {
        types.add(local.type);
      }
      return Collections.unmodifiableList(types);
    }

    public List<VarType> getStack() {
      List<VarType> types = new ArrayList<>(stack.size());
      for (VerificationTypeInfo item : stack) {
        types.add(item.type);
      }
      return Collections.unmodifiableList(types);
    }

    public VarType getLocalType(int localIndex) {
      return localsBySlot.get(localIndex);
    }

    private static Map<Integer, VarType> buildLocalsBySlot(List<VerificationTypeInfo> locals) {
      if (locals.isEmpty()) {
        return Collections.emptyMap();
      }

      int slot = 0;
      Map<Integer, VarType> map = new HashMap<>();
      for (VerificationTypeInfo local : locals) {
        if (local.type != null) {
          map.put(slot, local.type);
        }
        slot += local.slotSize();
      }
      return Collections.unmodifiableMap(map);
    }
  }
}
