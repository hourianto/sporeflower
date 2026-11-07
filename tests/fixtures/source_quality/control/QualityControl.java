/** Small Java 1.3-era collection, parser and command-dispatch shapes. */
public class QualityControl {
    static int trace;

    static void mark(int value) {
        trace = trace * 31 + value;
    }

    static int strideSum(int[] values, int start, int end) {
        int total = 0;
        for (int index = start; index < end; index += 2) {
            total += values[index];
        }
        return total;
    }

    static int reverseSum(int[] values) {
        int total = 0;
        for (int index = values.length - 1; index >= 0; index--) {
            total += values[index];
        }
        return total;
    }

    static void countDownByTwo(int remaining) {
        while (remaining > 0) {
            mark(remaining);
            remaining -= 2;
        }
    }

    static int scanSeparator(String text, int offset) {
        while (offset < text.length() && text.charAt(offset) != ':') {
            offset++;
        }
        return offset;
    }

    static int reverseFind(int[] values, int wanted) {
        int index = values.length - 1;
        while (index >= 0 && values[index] != wanted) {
            index--;
        }
        return index;
    }

    static int decimalDigits(int value) {
        int digits = 0;
        do {
            digits++;
            value /= 10;
        } while (value != 0);
        return digits;
    }

    static int shiftUntilClear(int value, int mask) {
        int shifts = 0;
        do {
            mark(value);
            value >>>= 1;
            shifts++;
        } while ((value & mask) != 0);
        return shifts;
    }

    static int matrixSum(int[][] rows) {
        int total = 0;
        for (int row = 0; row < rows.length; row++) {
            for (int column = 0; column < rows[row].length; column++) {
                total += rows[row][column];
            }
        }
        return total;
    }

    static int prefixRows(int[][] rows) {
        int total = 0;
        for (int row = 0; row < rows.length; row++) {
            for (int column = 0; column < rows[row].length; column++) {
                if (rows[row][column] < 0) break;
                total += rows[row][column];
            }
            mark(total);
        }
        return total;
    }

    static int guardedLength(String text, int limit) {
        if (text == null) return -1;
        if (limit < 0) return -2;
        if (text.length() > limit) return -3;
        mark(text.length());
        return text.length();
    }

    static void checkedRange(int start, int end) {
        if (start < 0 || start > end) throw new IllegalArgumentException();
        mark(start);
        mark(end);
    }

    static void classify(int value) {
        if (value < 0) {
            mark(10);
            mark(11);
        } else if (value == 0) {
            mark(20);
            mark(21);
        } else {
            mark(30);
            mark(31);
        }
        mark(40);
    }

    static void nestedDecision(boolean enabled, int mode) {
        if (enabled) {
            mark(1);
            if (mode == 0) {
                mark(2);
            } else {
                mark(3);
            }
            mark(4);
        } else {
            mark(5);
        }
        mark(6);
    }

    static int denseCommand(int command) {
        switch (command) {
            case 1: return 10;
            case 2: return 20;
            case 3: return 30;
            default: return -1;
        }
    }

    static int sparseCommand(int command) {
        switch (command) {
            case -100: return 7;
            case 9: return 11;
            case 1000: return 13;
            default: return -1;
        }
    }

    static void groupedCommands(int command) {
        switch (command) {
            case 2:
            case 5:
                mark(10);
                mark(11);
                break;
            case 7:
                mark(20);
                break;
            default:
                mark(30);
        }
        mark(40);
    }

    static void fallthroughCommand(int command) {
        switch (command) {
            case 0:
                mark(10);
            case 1:
                mark(11);
                break;
            case 2:
                mark(20);
                break;
            default:
                mark(30);
        }
        mark(40);
    }

    static void loopCommand(int command, int count) {
        switch (command) {
            case 1:
                for (int index = 0; index < count; index++) mark(10);
                break;
            case 2:
                do {
                    mark(20);
                } while (--count > 0);
                break;
            default:
                if (count > 0) mark(30);
                break;
        }
        mark(40);
    }

    static void dispatchLoop(int[] commands) {
        for (int index = 0; index < commands.length; index++) {
            switch (commands[index]) {
                case 0: mark(10); break;
                case 1: mark(20); break;
                default: mark(30);
            }
            mark(40);
        }
    }

    static int guardedSwitch(int command, boolean enabled) {
        if (!enabled) return -1;
        mark(1);
        switch (command) {
            case 1: mark(10); break;
            case 2: mark(20); break;
            default: mark(30);
        }
        mark(40);
        return 0;
    }
}
