public class QualityChoices {
    static int trace;
    static int cursor;
    static final byte[] stored = new byte[] { 2, 4, 6 };

    static int observe(int value) {
        trace = trace * 31 + value;
        return value;
    }

    static byte[] bytes(int value) {
        observe(value);
        return new byte[] { (byte) value, (byte) (value + 1) };
    }

    static int consume(byte[] data, int length) {
        observe(length);
        return data[0] + length;
    }

    static int advanceBoundary(int position, int limit) {
        if (position == -1) position = limit;
        else position++;
        observe(position);
        return position + limit;
    }

    static int adjustedIndex(boolean enabled, boolean first, boolean second, int input) {
        int result = input;
        if (enabled) {
            if (input >= 2 && first) result = input + 3;
            else if (input >= 4 && second) result = input + 7;
        }
        observe(result);
        return result * 31 + input;
    }

    static int selectedBuffer(boolean selected, int input) {
        int result;
        if (selected) {
            byte[] data = bytes(input);
            result = consume(data, data.length);
        } else {
            result = consume(stored, 1);
        }
        return result;
    }

    static int compoundRule(int style, boolean enabled, int group, int mode, int target, int first, int second) {
        int range;
        if (style == 1) range = 5;
        else if (enabled && group == 2 && mode == 0 && target >= 0) range = -1;
        else range = Math.max(first, second);
        observe(range);
        return range;
    }

    static int changedSelector(int position, int limit) {
        int result;
        if (position++ < limit) result = position;
        else result = limit;
        return observe(result) + position + limit;
    }

    static boolean checked(int input) {
        observe(input);
        if (input == 9) throw new IllegalArgumentException();
        return input > 0;
    }

    static int caughtDefault(boolean first, boolean second, int input) {
        int result = -17;
        try {
            if (first) result = input;
            else if (checked(input)) result = input + 2;
            else if (second) result = input;
            else result = input - 3;
        } catch (IllegalArgumentException caught) {
            observe(result);
        }
        return result + input;
    }

    static int changedDefault(boolean first, boolean second, int input) {
        int result;
        if (first) result = input;
        else if (input++ > 1) result = input + 2;
        else if (second) result = input;
        else result = input - 3;
        return result + input;
    }

    static int resetChoice(boolean first, boolean second, int input) {
        int result;
        if (first) {
            result = input + 1;
            if (second) result = input;
        } else {
            result = input;
        }
        return result;
    }

    static int widenedDefault(boolean first, boolean second, byte input) {
        int result = input;
        if (first && second) result = 150 * input / 100;
        observe(result);
        return result + input;
    }

    static String literalDefault(boolean first, boolean second, int input) {
        String result = "/items";
        if (!first && second) result += input;
        observe(result.length());
        return result;
    }

    static int materializedPredicates(boolean enabled, int input) {
        int result;
        if (enabled) {
            boolean below;
            if (input < 3) below = true;
            else below = false;
            if (below) result = 1;
            else {
                boolean matching;
                if (input == 5) matching = true;
                else matching = false;
                if (matching) result = 2;
                else result = 3;
            }
        } else result = 0;
        observe(result);
        return result;
    }

    static int narrowAlternative(short[] values, int input) {
        int result;
        if (input < 0) result = values[5];
        else result = input;
        observe(result);
        return result + input;
    }

    static int guardedChoice(boolean enabled, int first, int second, int result) {
        if (enabled) {
            if (first < second) result = -1;
            else result = 1;
        }
        observe(result);
        return result;
    }

    static int independentChoices(boolean enabled, int first, int second) {
        int result;
        if (enabled) {
            if (first < second) result = -1;
            else result = 1;
            if (first == second) result += 3;
            else result += 4;
        } else result = 0;
        observe(result);
        return result;
    }

    static int maskAdjust(int direction, int mask, int x, int y) {
        for (int step = 0; step < 2; step++) {
            if (direction == 0) mask |= 1 << (y + step);
            else mask |= 1 << (x + step);
            if (direction == 0) mask &= ~(1 << (y + step + 1));
            else mask &= ~(1 << (x + step + 1));
        }
        return mask;
    }

    static int crcUpdate(byte value, int crc) {
        crc ^= value & 255;
        for (int bit = 0; bit < 8; bit++) {
            if ((crc & 1) == 1) crc = crc >>> 1 ^ -306674912;
            else crc >>>= 1;
        }
        return crc;
    }

    static String contentType(String name) {
        if (name.endsWith(".txt")) return "text/plain";
        if (name.endsWith(".html")) return "text/html";
        if (name.endsWith(".png")) return "image/png";
        if (name.endsWith(".jpg")) return "image/jpeg";
        if (name.endsWith(".aac")) return "audio/aac";
        if (name.endsWith(".css")) return "text/css";
        return "application/octet-stream";
    }

    static int maskScan(String input, int target) {
        if (target <= 0) return 0;
        int position = 0;
        int count = 0;
        do {
            if (position < input.length() && input.charAt(position) == '|') count++;
            position++;
        } while (((count == target ? -1 : 0) | (position > input.length() ? -1 : 0)) == 0);
        return position;
    }

    static boolean nextOne(byte[] input) {
        return cursor < input.length && input[cursor++] == 1;
    }

    static Boolean boxedToggle(Object input) {
        input = (Object[]) input;
        boolean selected = ((Boolean) ((Object[]) input)[0]).booleanValue();
        return new Boolean(!selected);
    }

    static String label(boolean selected) { return selected ? "ready" : "waiting"; }
    static int clamp(int value, int low, int high) { return value < low ? observe(low) : value > high ? observe(high) : observe(value); }
    static int calls(boolean first, boolean second) { return first && second ? observe(1) : observe(2); }

    public static void main(String[] args) {
        for (int flags = 0; flags < 8; flags++) {
            boolean enabled = (flags & 1) != 0;
            boolean first = (flags & 2) != 0;
            boolean second = (flags & 4) != 0;
            for (int input = -1; input < 10; input++) {
                trace = 0;
                System.out.println(advanceBoundary(input, 20));
                System.out.println(adjustedIndex(enabled, first, second, input));
                System.out.println(selectedBuffer(first, input));
                System.out.println(compoundRule(flags % 3, enabled, first ? 2 : 0, second ? 0 : 1, input, input + 2, 4));
                System.out.println(changedSelector(input, 3));
                System.out.println(caughtDefault(first, second, input));
                System.out.println(changedDefault(first, second, input));
                System.out.println(resetChoice(first, second, input));
                System.out.println(widenedDefault(first, second, (byte) input));
                System.out.println(literalDefault(first, second, input));
                System.out.println(materializedPredicates(enabled, input));
                System.out.println(narrowAlternative(new short[] { 0, 1, 2, 3, 4, -7 }, input));
                System.out.println(guardedChoice(enabled, input, flags, input));
                System.out.println(independentChoices(enabled, input, flags));
                System.out.println(maskAdjust(flags % 2, input, 2, 4));
                System.out.println(crcUpdate((byte) input, flags));
                System.out.println(contentType(new String[] { ".txt", ".html", ".png", ".jpg", ".aac", ".css", ".bin" }[flags % 7]));
                System.out.println(maskScan("a|b|", input));
                cursor = (input + 1) % 4;
                System.out.println(nextOne(new byte[] { 1, 2, 1 }));
                System.out.println(cursor);
                System.out.println(boxedToggle(new Object[] { new Boolean(first) }));
                System.out.println(label(enabled));
                System.out.println(clamp(input, 0, 5));
                System.out.println(calls(first, second));
                System.out.println(trace);
            }
        }
        System.out.println(advanceBoundary(Integer.MAX_VALUE, 20));
        System.out.println(changedSelector(Integer.MAX_VALUE, 3));
    }
}
