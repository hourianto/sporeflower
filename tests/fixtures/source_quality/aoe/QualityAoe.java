public final class QualityAoe {
    private static int trace;

    private static void observe(int value) {
        trace = trace * 31 + value;
    }

    public static int menuAction(int action, int frame, int frameCount) {
        switch (action) {
            case 1:
                observe(1);
                action = 30;
                break;
            case 2:
                if (frame >= frameCount) {
                    observe(2);
                    action = 17;
                } else if (++frame == frameCount) {
                    observe(3);
                }
                break;
            case 3:
                observe(4);
                action = 57;
                break;
            default:
                break;
        }
        observe(action);
        return action;
    }

    public static void objectiveFlags(int[] objective, int available, boolean enemyDefeated) {
        boolean complete = objective[1] == 1;
        boolean failed = objective[1] == 2;
        if (complete) {
            return;
        }
        switch (objective[0]) {
            case 0:
                if (available >= objective[2]) {
                    complete = true;
                }
                break;
            case 1:
                if (enemyDefeated) {
                    failed = true;
                }
                break;
            case 2:
                complete = true;
                failed = true;
                break;
            default:
                break;
        }
        if (complete) {
            objective[1] = 1;
            observe(1);
        }
        if (failed) {
            objective[1] = 2;
            observe(2);
        }
    }

    public static int freshResult(int mode) {
        int result;
        switch (mode) {
            case 0:
                observe(10);
                result = 0;
                break;
            case 1:
                observe(11);
                result = 7;
                break;
            default:
                result = -1;
                break;
        }
        observe(result);
        return result;
    }

    public static int searchResult(int[] values, int wanted) {
        int index;
        for (index = 0; true; index++) {
            if (index >= values.length) {
                index = -1;
                break;
            }
            observe(values[index]);
            if (values[index] == wanted) {
                break;
            }
        }
        observe(index);
        return index;
    }

    public static int freshCopiedResult(int mode, int fallback) {
        {
            int previous = fallback;
            observe(previous);
        }
        int result;
        switch (mode) {
            case 0:
                observe(20);
                result = 0;
                break;
            case 1:
                observe(21);
                result = fallback;
                break;
            default:
                result = -1;
                break;
        }
        observe(result);
        return result;
    }

    public static boolean retainedFlag(boolean failed, int[] states) {
        for (int index = 0; index < states.length; index++) {
            if (states[index] == 2) {
                failed = true;
                break;
            }
        }
        observe(failed ? 1 : 0);
        return failed;
    }

    public static void main(String[] arguments) {
        for (int action = 0; action < 5; action++) {
            for (int frame = 0; frame < 4; frame++) {
                System.out.println(menuAction(action, frame, 2) + ":" + trace);
            }
        }
        for (int kind = 0; kind < 4; kind++) {
            for (int state = 0; state < 3; state++) {
                for (int available = 0; available < 2; available++) {
                    for (int enemy = 0; enemy < 2; enemy++) {
                        int[] objective = {kind, state, 1};
                        objectiveFlags(objective, available, enemy != 0);
                        System.out.println(objective[1] + ":" + trace);
                    }
                }
            }
        }
        for (int mode = -1; mode < 3; mode++) {
            System.out.println(freshResult(mode) + ":" + trace);
            System.out.println(freshCopiedResult(mode, 42) + ":" + trace);
        }
        int[][] searches = {new int[0], {1}, {2, 1, 3}, {3, 4}};
        for (int row = 0; row < searches.length; row++) {
            for (int wanted = 0; wanted < 5; wanted++) {
                System.out.println(searchResult(searches[row], wanted) + ":" + trace);
            }
            System.out.println(retainedFlag(false, searches[row]) + ":" + trace);
            System.out.println(retainedFlag(true, searches[row]) + ":" + trace);
        }
    }
}
