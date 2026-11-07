public final class QualitySharedTails {
    private static int trace;

    private static void observe(int value, int failure) {
        trace = trace * 31 + value;
        if (value == failure) throw new IllegalArgumentException();
    }

    private static final class Cell {
        int value;
    }

    // AoE's spiral carries a shared length copy through three direction leaves.
    public static int spiral(boolean clockwise, int direction, int rounds) {
        int length = 1;
        int remaining = 1;
        int turns = 0;
        int total = 0;
        for (int step = 0; step < rounds; step++) {
            remaining--;
            if (remaining <= 0) {
                turns++;
                if (turns == 2) {
                    turns = 0;
                    length++;
                }
                remaining = length;
                if (clockwise) {
                    direction = (direction + 2) % 8;
                } else {
                    direction -= 2;
                    if (direction < 0) direction += 8;
                }
            }
            total = total * 7 + direction + remaining;
        }
        return total;
    }

    // Snake's direction cases carry the unchanged other coordinate to one join.
    public static int wrap(int direction, int x, int y) {
        switch (direction) {
            case 1:
                x--;
                if (x < 0) x = 7;
                break;
            case 2:
                x++;
                if (x == 8) x = 0;
                break;
            case 3:
                y--;
                if (y < 0) y = 7;
                break;
            default:
                break;
        }
        return x * 8 + y;
    }

    // MP3Tools's scan index survives a conditional operation that can throw.
    public static int protectedScan(boolean skip, int offset, int size, int failure) {
        int position = offset;
        int total = 0;
        try {
            if (skip) {
                size -= 2;
                observe(size, failure);
            }
            do {
                observe(position, failure);
                total += position;
                position++;
            } while (position < size);
            return total;
        } catch (IllegalArgumentException ex) {
            return position * 100 + total;
        }
    }

    // Tetris's resource decoder shares a constructed result across setup arms.
    public static int decode(int kind, boolean empty, int seed, int failure) {
        Object result;
        switch (kind) {
            case 0:
                Cell created = new Cell();
                result = created;
                if (empty) {
                    observe(seed, failure);
                } else {
                    observe(seed, failure);
                    created.value = seed;
                }
                break;
            case 1:
                result = "plain";
                break;
            default:
                result = null;
        }
        return result instanceof Cell ? ((Cell) result).value : result == null ? -1 : -2;
    }

    // Different values and an abrupt arm must never become a common store.
    public static int interrupted(boolean first, boolean second, int seed, int failure) {
        int result = seed;
        try {
            if (first) {
                observe(1, failure);
                if (second) return result;
                result = seed + 1;
            } else {
                observe(2, failure);
                result = seed - 1;
            }
            observe(result, failure);
        } catch (IllegalArgumentException ex) {
            observe(result, -100);
        }
        return result;
    }

    public static void main(String[] args) {
        for (int clockwise = 0; clockwise < 2; clockwise++) {
            for (int direction = 0; direction < 8; direction++) {
                for (int rounds = 0; rounds < 15; rounds++) {
                    System.out.println(spiral(clockwise != 0, direction, rounds));
                }
            }
        }
        for (int direction = 0; direction < 5; direction++) {
            for (int x = 0; x < 8; x++) {
                for (int y = 0; y < 8; y++) System.out.println(wrap(direction, x, y));
            }
        }
        for (int mode = 0; mode < 4; mode++) {
            for (int seed = 0; seed < 4; seed++) {
                for (int failure = -1; failure < 7; failure++) {
                    System.out.println(protectedScan(mode != 0, seed, 6, failure) + ":" + trace);
                    System.out.println(interrupted((mode & 1) != 0, (mode & 2) != 0, seed, failure) + ":" + trace);
                    try {
                        System.out.println(decode(mode & 1, (mode & 2) != 0, seed, failure) + ":" + trace);
                    } catch (IllegalArgumentException ex) {
                        System.out.println("decode:" + trace);
                    }
                }
            }
        }
    }
}
