public class DebugNames extends DebugNamesBase {
    static int trace;
    static final int resultConst;
    final int saved;

    static {
        int computed = step(3);
        resultConst = computed * computed;
    }

    DebugNames(int seed) {
        super(seed > 0 ? step(seed) : step(-seed), seed);
        saved = seed;
    }

    static int step(int value) {
        trace = trace * 31 + value;
        return value;
    }

    static long wide(String resource, long timeout, double scale, int retries) {
        return resource.length() + timeout + (long) scale + retries;
    }

    static int switching(int selector) {
        int result;
        switch (selector) {
            case 0: {
                int pageCount = step(4);
                result = pageCount * pageCount;
                break;
            }
            case 1: {
                int pageCount = step(5);
                result = pageCount * pageCount;
                break;
            }
            default:
                result = 3;
        }
        return result;
    }

    static int derivedLoop(int footprint, int maxTileX) {
        int total = 0;
        for (footprint = maxTileX - footprint + 1; footprint < maxTileX; footprint++) {
            total += step(footprint);
        }
        return total;
    }

    static int callResult(int height) {
        height = step(height);
        return height * height;
    }

    static int qualifiers(String System, String java) {
        return System.length() + java.length();
    }

    static int duplicateParameters(int first, int second) { return first * 10 + second; }
    static int invalidParameter(int good) { return good + 1; }
    static int ambiguousParameter(int good) { return good + 2; }
    static int helperParameter(int $state) { return $state + 3; }
    static int legacyReservedParameters(int enclosing, int cached) { return enclosing + cached; }

    public static void main(String[] arguments) {
        for (int index = -2; index < 5; index++) {
            DebugNames item = new DebugNames(index);
            System.out.println(item.saved + item.first + item.second);
            System.out.println(switching(index));
            System.out.println(derivedLoop(index + 3, 7));
            System.out.println(callResult(index));
        }
        System.out.println(wide("name", 30L, 7.0, 2));
        System.out.println(qualifiers("one", "two"));
        System.out.println(duplicateParameters(4, 7));
        System.out.println(invalidParameter(4));
        System.out.println(ambiguousParameter(4));
        System.out.println(helperParameter(4));
        System.out.println(legacyReservedParameters(4, 7));
        System.out.println(trace);
        System.out.println(resultConst);
    }
}

class DebugNamesBase {
    final int first;
    final int second;
    DebugNamesBase(int first, int second) {
        this.first = first;
        this.second = second;
    }
}
