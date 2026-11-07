public class RegionBoundaries {
    private static int trace;
    private int value;

    private static boolean probe(int input, int step) {
        trace = trace * 31 + step;
        if (input == step + 16) throw new IllegalArgumentException("p" + step);
        return (input & (1 << (step - 1))) != 0;
    }

    private static RegionBoundaries receiver(RegionBoundaries value, int input, int step) {
        probe(input, step);
        return value;
    }

    private static void caseTail(RegionBoundaries value, int input, int mode) {
        switch (mode) {
        case 0:
            write: {
                RegionBoundaries target;
                int result;
                if (probe(input, 1)) {
                    target = receiver(value, input, 3);
                    result = 11;
                } else {
                    if (!probe(input, 2)) break write;
                    target = receiver(value, input, 4);
                    result = 13;
                }
                target.value = result;
            }
            value.value += 17;
            break;
        case 1:
            value.value += 19;
            break;
        default:
            value.value += 23;
        }
    }

    private static void protectedTail(RegionBoundaries value, int input, int mode) {
        try {
            caseTail(value, input, mode);
            probe(input, 5);
        } catch (IllegalArgumentException failure) {
            trace = trace * 31 + 61;
            value.value += 29;
        }
        probe(input, 6);
    }

    private static void cleanupExits(RegionBoundaries value, int input) {
        for (int i = 0; i < 3; i++) {
            try {
                if (probe(input, 1)) continue;
                if (probe(input, 2)) break;
                value.value += 31;
            } finally {
                probe(input, 3);
                value.value += 37;
            }
            value.value += 41;
        }
        value.value += 43;
    }

    public static void main(String[] args) {
        for (int input = 0; input < 24; input++) {
            for (int mode = 0; mode < 4; mode++) {
                RegionBoundaries value = new RegionBoundaries();
                trace = 0;
                try {
                    protectedTail(value, input, mode);
                    cleanupExits(value, input);
                    System.out.println(value.value + ":" + trace);
                } catch (Throwable failure) {
                    System.out.println(failure.getClass().getName() + ":" + failure.getMessage() + ":" + value.value + ":" + trace);
                }
            }
        }
    }
}
