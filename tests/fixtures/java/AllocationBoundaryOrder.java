public class AllocationBoundaryOrder {
    static int trace;
    static int failure;
    static final RuntimeException marker = new RuntimeException("effect");

    static int effect(int value) {
        trace = trace * 10 + value;
        if (failure == value) throw marker;
        return value;
    }

    static String text(int value) { return String.valueOf(effect(value)); }

    static Object beforeObject(boolean selected) {
        int value = selected ? effect(1) : effect(4);
        return new BoundaryNode(value);
    }

    static Object insideObject(boolean selected) {
        return new BoundaryNode(selected ? effect(1) : effect(4));
    }

    static Object beforeArray(boolean selected) {
        String value = selected ? text(1) : text(4);
        return new String[] { value, "tail" };
    }

    static Object insideArray(boolean selected) {
        return new String[] { selected ? text(1) : text(4), "tail" };
    }

    static Object arrayInChoice(boolean selected) {
        return selected ? new String[] { text(1), text(4) } : new String[] { text(5) };
    }

    public static void main(String[] args) {
        failure = Integer.parseInt(args[1]);
        boolean selected = args[2].equals("true");
        try {
            Object result;
            if (args[0].equals("beforeObject")) result = beforeObject(selected);
            else if (args[0].equals("insideObject")) result = insideObject(selected);
            else if (args[0].equals("beforeArray")) result = beforeArray(selected);
            else if (args[0].equals("insideArray")) result = insideArray(selected);
            else result = arrayInChoice(selected);
            System.out.println(result == null ? "empty" : "created");
        } catch (RuntimeException caught) {
            System.out.println(caught == marker ? "effect" : "other");
        } catch (Error caught) {
            System.out.println("initialization");
        }
        System.out.println(trace);
    }
}

class BoundaryNode {
    static int initialized = AllocationBoundaryOrder.effect(2);
    BoundaryNode(int value) { AllocationBoundaryOrder.effect(3); }
}
