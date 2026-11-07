public class ConditionalArrayTypes {
    Object value;

    void store(boolean condition, ChoiceLeft[] left, ChoiceRight[] right, int index) {
        value = condition ? left[index] : right[index + 1];
    }

    static Object returned(boolean condition, ChoiceLeft[] left, ChoiceRight[] right, int index) {
        return condition ? left[index] : right[index + 1];
    }

    static Object mixed(boolean condition, ChoiceLeft[] left, int[][] right, int index) {
        return condition ? left[index] : right[index + 1];
    }

    static Object[] arrays(boolean condition, ChoiceLeft[][] left, ChoiceRight[][] right, int index) {
        return condition ? left[index] : right[index + 1];
    }

    static ChoiceBase related(boolean condition, ChoiceLeft[] left, ChoiceBase[] right, int index) {
        return condition ? left[index] : right[index + 1];
    }

    static Object nullable(boolean condition, ChoiceLeft[] left, int index) {
        return (condition ? left : null)[index];
    }

    static int invoke(boolean condition, ChoiceLeft[] left, ChoiceRight[] right, int index) {
        return overload(condition ? (Object) left[index] : right[index + 1]);
    }

    static int captured(boolean condition, ChoiceLeft[] left, ChoiceRight[] right, int index) {
        Object saved = condition ? left[index] : right[index + 1];
        return overload(saved) + overload(saved);
    }

    static int overload(Object value) { return 1; }
    static int overload(ChoiceBase value) { return 2; }
    static int overload(ChoiceLeft value) { return 3; }
    static int overload(ChoiceRight value) { return 4; }

    public static void main(String[] args) {
        ChoiceLeft[] left = {new ChoiceLeft(), null};
        ChoiceRight[] right = {null, new ChoiceRight(), null};
        int[][] integers = {null, new int[] {7}, null};
        ConditionalArrayTypes target = new ConditionalArrayTypes();
        for (int selected = 0; selected < 2; selected++) {
            boolean condition = selected != 0;
            for (int index = 0; index < 2; index++) {
                target.store(condition, left, right, index);
                Object expected = condition ? left[index] : right[index + 1];
                System.out.println(target.value == expected);
                System.out.println(returned(condition, left, right, index) == expected);
                System.out.println(mixed(condition, left, integers, index) == (condition ? left[index] : integers[index + 1]));
                System.out.println(related(condition, left, right, index) == expected);
                System.out.println(invoke(condition, left, right, index));
                System.out.println(captured(condition, left, right, index));
                try {
                    System.out.println(nullable(condition, left, index) == left[index]);
                } catch (NullPointerException failure) {
                    System.out.println("null array");
                }
            }
            System.out.println(arrays(condition, new ChoiceLeft[][] {left}, new ChoiceRight[][] {null, right}, 0) == (condition ? left : right));
        }
    }
}

class ChoiceBase {}
final class ChoiceLeft extends ChoiceBase {}
final class ChoiceRight extends ChoiceBase {}
