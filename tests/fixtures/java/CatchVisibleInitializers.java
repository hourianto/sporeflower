public class CatchVisibleInitializers {
    static int calls, failAt;

    static void touch() {
        if (++calls == failAt) throw new IllegalArgumentException();
    }

    static String branches(String input) {
        String result = "";
        try {
            touch();
            if (input.length() > 0) {
                touch();
                if (input.charAt(0) == 'a') touch();
                else touch();
            }
            result = input.substring(1);
            touch();
        } catch (RuntimeException expected) {
            touch();
        }
        return result;
    }

    static int loops(int count) {
        int current = 5;
        int saved = current;
        try {
            for (int i = 0; i < count; i++) {
                touch();
                saved = current;
                current = current * 3 + i;
                touch();
            }
        } catch (RuntimeException expected) {
            return current * 1000 + saved;
        }
        return current + saved;
    }

    static int catchUpdates(int count) {
        int current = 7;
        while (count-- > 0) {
            try {
                touch();
                current += 2;
                touch();
            } catch (RuntimeException expected) {
                current += 11;
            }
        }
        return current;
    }

    static String replacement(String value) {
        touch();
        return new String(value);
    }

    static String stringsInLoop(int count) {
        String current = "";
        String saved = current;
        String trace = "";
        for (int i = 0; i < count; i++) {
            try {
                touch();
                saved = current;
                current = replacement(current + i);
                touch();
            } catch (RuntimeException expected) {
                trace += ":" + current + "/" + saved + "/" + (current == saved);
                current = "caught";
            }
        }
        return trace + ":" + current + "/" + saved + "/" + (current == saved);
    }

    static String sharedBindings() {
        String first = "seed";
        String second = first;
        try {
            touch();
            first = replacement("fresh");
            touch();
            second = first;
            first = replacement("last");
            touch();
        } catch (RuntimeException expected) {
            return first + "/" + second + "/" + (first == second) + "/" + (second == "seed");
        }
        return first + "/" + second + "/" + (first == second);
    }

    static String pair(String before, String after) {
        return before + "/" + after + "/" + (before == after);
    }

    static String retainedValue() {
        String current = "seed";
        String result = "";
        try {
            touch();
            result = pair(current, current = replacement("next"));
            touch();
        } catch (RuntimeException expected) {
            return result + ":" + current + "/" + (current == "seed");
        }
        return result + ":" + current;
    }

    public static void main(String[] args) {
        for (failAt = 0; failAt < 12; failAt++) {
            calls = 0;
            System.out.println(branches("abc") + ":" + calls);
            calls = 0;
            System.out.println(branches("xyz") + ":" + calls);
            calls = 0;
            System.out.println(loops(3) + ":" + calls);
            calls = 0;
            System.out.println(catchUpdates(3) + ":" + calls);
            calls = 0;
            System.out.println(stringsInLoop(3) + ":" + calls);
            calls = 0;
            System.out.println(sharedBindings() + ":" + calls);
            calls = 0;
            System.out.println(retainedValue() + ":" + calls);
        }
    }
}
