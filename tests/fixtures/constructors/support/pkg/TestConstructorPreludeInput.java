package pkg;

import java.util.ArrayList;
import java.util.List;

public class TestConstructorPreludeInput {
  public static final List<String> events = new ArrayList<String>();
  public static TestConstructorPreludeInput active;
  public static int constructions;

  public byte[] data;
  public int position;
  public int remaining;

  public TestConstructorPreludeInput(byte[] data) {
    this.data = data;
    this.remaining = data.length;
    active = this;
    constructions = 0;
    events.clear();
  }

  public final int readInt() {
    events.add("int@" + position);
    int value = (data[position] & 255) | (data[position + 1] & 255) << 8
      | (data[position + 2] & 255) << 16 | data[position + 3] << 24;
    position += 4;
    remaining -= 4;
    return value;
  }

  public final String readText() {
    events.add("text@" + position);
    int length = readInt();
    char[] chars = new char[length];
    for (int i = 0; i < length; i++) {
      chars[i] = (char)((data[position] & 255) | (data[position + 1] & 255) << 8);
      position += 2;
      remaining -= 2;
    }
    return new String(chars);
  }

  public final TestConstructorPreludeInput advance(int length) {
    events.add("advance@" + position + ":" + length);
    position += length;
    remaining -= length;
    return this;
  }

  public static String decode(byte[] data, int start, int end, boolean enabled) {
    events.add("decode@" + start + ":" + end);
    if (!enabled) {
      throw new IllegalArgumentException();
    }
    // The codec is reduced to ASCII; its data/range arguments and lack of cursor
    // mutation are the properties relevant to constructor argument evaluation.
    char[] chars = new char[end - start];
    for (int i = 0; i < chars.length; i++) {
      chars[i] = (char)(data[start + i] & 255);
    }
    return new String(chars);
  }

  public static void constructed() {
    constructions++;
    events.add("construct@" + active.position);
  }
}
