public class ReturnChoices {
    static int trace;
    final int width;
    final int height;

    ReturnChoices(int width, int height) {
        this.width = width;
        this.height = height;
    }

    static int effect(int value) {
        trace = trace * 31 + value;
        return value;
    }

    static int guardedReturns(int value) {
        if (effect(value) < 10) return effect(1);
        if (effect(value) < 100) return effect(2);
        return effect(3);
    }

    static int nestedReturns(int value) {
        if (value < 100) {
            if (value >= 10) return 2;
            if (value <= 0) return 0;
            return 1;
        } else if (value >= 10000) {
            return 5;
        } else if (value >= 1000) {
            return 4;
        } else {
            return 3;
        }
    }

    static String referenceReturns(int value) {
        if (value < 0) return "negative";
        if (value > 0) return "positive";
        return "zero";
    }

    static int joinedChoice(int value) {
        return effect(value) < 10 ? effect(1) : effect(value) < 100 ? effect(2) : effect(3);
    }

    static int singleChoice(boolean selected) {
        if (selected) return effect(1);
        return effect(2);
    }

    static ReturnChoices largeReturns(boolean vertical, int value) {
        if (vertical) {
            return new ReturnChoices(
                Math.max(effect(value), effect(value + 1) * 2),
                Math.max(effect(value + 2), effect(value + 3) / 2));
        }
        return new ReturnChoices(
            Math.max(effect(value), effect(value + 1) / 2),
            Math.max(effect(value + 2), effect(value + 3) * 2));
    }

    static ReturnChoices largeJoined(boolean vertical, int value) {
        return vertical
            ? new ReturnChoices(
                Math.max(effect(value), effect(value + 1) * 2),
                Math.max(effect(value + 2), effect(value + 3) / 2))
            : new ReturnChoices(
                Math.max(effect(value), effect(value + 1) / 2),
                Math.max(effect(value + 2), effect(value + 3) * 2));
    }

    static int mixedChoice(int value) {
        if (value < 0) return effect(0);
        return value > 10 ? effect(1) : effect(2);
    }

    static boolean booleanReturns(int value) {
        if (effect(value) < 0) return false;
        if (effect(value) > 10) return true;
        return effect(value) == 5;
    }

    public static void main(String[] args) {
        int[] values = {-1, 0, 1, 5, 9, 10, 11, 99, 100, 999, 1000, 9999, 10000};
        for (int i = 0; i < values.length; i++) {
            int value = values[i];
            trace = 0;
            ReturnChoices returned = largeReturns(value % 2 == 0, value);
            ReturnChoices joined = largeJoined(value % 2 == 0, value);
            System.out.println(guardedReturns(value) + ":" + nestedReturns(value)
                + ":" + referenceReturns(value) + ":" + joinedChoice(value)
                + ":" + singleChoice(value > 0) + ":" + mixedChoice(value)
                + ":" + returned.width + ":" + returned.height
                + ":" + joined.width + ":" + joined.height
                + ":" + booleanReturns(value) + ":" + trace);
        }
    }
}
