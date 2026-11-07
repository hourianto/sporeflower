public class SwitchDefaults {
    static int trace;

    static int selector(int value) {
        trace = trace * 3 + 1;
        if (value == -7) {
            throw new IllegalArgumentException();
        }
        return value;
    }

    static int consume(boolean wide) {
        trace = trace * 7 + (wide ? 2 : 1);
        return wide ? 101 : 17;
    }

    static int simple(int choice) {
        boolean wide = false;
        int selected = selector(choice);
        switch (selected) {
            case 32:
                wide = true;
            case 17:
                return consume(wide);
            default:
                return -selected;
        }
    }

    static int repeated(int[] choices) {
        int total = 0;
        for (int i = 0; i < choices.length; i++) {
            boolean wide = false;
            int selected = selector(choices[i]);
            switch (selected) {
                case 32:
                    wide = true;
                case 17:
                    total += consume(wide);
                    break;
                default:
                    total += 3;
                    break;
            }
        }
        return total;
    }

    static int observed(int choice) {
        boolean flag = true;
        try {
            int selected = selector(choice);
            flag = false;
            switch (selected) {
                case 32:
                    flag = true;
                case 17:
                    return consume(flag);
                default:
                    return -selected;
            }
        } catch (IllegalArgumentException failure) {
            return flag ? 73 : 29;
        }
    }

    public static void main(String[] args) {
        for (int choice = -8; choice <= 33; choice++) {
            trace = choice;
            int answer;
            try {
                answer = simple(choice);
            } catch (IllegalArgumentException failure) {
                answer = 41;
            }
            System.out.print(answer);
            System.out.print(':');
            System.out.println(trace);
            trace = choice;
            System.out.print(observed(choice));
            System.out.print(':');
            System.out.println(trace);
        }
        for (int count = 0; count < 8; count++) {
            int[] choices = new int[count];
            for (int i = 0; i < count; i++) {
                choices[i] = i % 3 == 0 ? 32 : (i % 3 == 1 ? 17 : 9);
            }
            trace = count;
            System.out.print(repeated(choices));
            System.out.print(':');
            System.out.println(trace);
        }
    }
}
