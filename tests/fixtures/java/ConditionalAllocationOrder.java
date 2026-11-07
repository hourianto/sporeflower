public class ConditionalAllocationOrder {
    static int trace;
    static int failure;
    static final RuntimeException marker = new RuntimeException("effect");

    static int effect(int value) {
        trace = trace * 10 + value;
        if (failure == value) throw marker;
        return value;
    }

    static Object beforeShared(boolean selected) {
        Object result;
        if (selected) {
            int value = effect(1);
            result = new ConditionalAllocated(value, value);
        } else {
            result = null;
        }
        return result;
    }

    static Object beforeSingle(boolean selected) {
        Object result;
        if (selected) {
            int value = effect(1);
            result = new ConditionalAllocated(value, 0);
        } else {
            result = null;
        }
        return result;
    }

    static Object beforeNested(boolean selected) {
        Object result;
        if (selected) {
            int value = effect(1);
            result = new ConditionalOuter(new ConditionalAllocated(value, value));
        } else {
            result = null;
        }
        return result;
    }

    static Object insideShared(boolean selected) {
        int value;
        return selected ? new ConditionalAllocated(value = effect(1), value) : null;
    }

    static Object combine(int value, Object created) {
        effect(6);
        return created;
    }

    static Object reorderedAllocation(boolean selected) {
        Object result;
        if (selected) {
            int value;
            ConditionalAllocated created = new ConditionalAllocated(value = effect(1), 0);
            result = combine(value, created);
        } else {
            result = null;
        }
        return result;
    }

    public static void main(String[] args) {
        failure = Integer.parseInt(args[1]);
        boolean selected = args[2].equals("true");
        try {
            Object value;
            if (args[0].equals("shared")) value = beforeShared(selected);
            else if (args[0].equals("single")) value = beforeSingle(selected);
            else if (args[0].equals("nested")) value = beforeNested(selected);
            else if (args[0].equals("reordered")) value = reorderedAllocation(selected);
            else value = insideShared(selected);
            System.out.println(value == null ? "empty" : "created");
        } catch (RuntimeException caught) {
            System.out.println(caught == marker ? "effect" : "other");
        } catch (Error caught) {
            System.out.println("initialization");
        }
        System.out.println(trace);
    }
}

class ConditionalAllocated {
    static int initialized = ConditionalAllocationOrder.effect(2);
    ConditionalAllocated(int first, int second) { ConditionalAllocationOrder.effect(3); }
}

class ConditionalOuter {
    static int initialized = ConditionalAllocationOrder.effect(4);
    ConditionalOuter(Object value) { ConditionalAllocationOrder.effect(5); }
}
