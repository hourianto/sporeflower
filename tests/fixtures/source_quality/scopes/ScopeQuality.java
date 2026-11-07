public class ScopeQuality extends ScopeParent {
    static int trace;
    static final Object lock = new Object();
    static int total = 40;
    final int value;

    ScopeQuality(int value) {
        super(value + 1);
        this.value = value;
    }

    ScopeQuality() {
        this(5);
    }

    static int mark(int digit) {
        trace = trace * 31 + digit;
        return digit;
    }

    static int checkedValue(int value) throws java.io.IOException {
        mark(1);
        if (value < 0) throw new java.io.IOException("negative");
        return value;
    }

    static Object acquire() {
        mark(2);
        return lock;
    }

    static int returnFinally(int value) throws java.io.IOException {
        try {
            return checkedValue(value);
        } finally {
            mark(9);
        }
    }

    static void combinedCleanup(int value) {
        try {
            checkedValue(value);
        } catch (java.io.IOException problem) {
            mark(problem.getMessage().length());
        } finally {
            mark(8);
        }
    }

    static void rethrowChecked(int value) throws java.io.IOException {
        try {
            checkedValue(value);
        } catch (java.io.IOException problem) {
            mark(7);
            throw problem;
        }
    }

    static void monitorExpression(int value) {
        synchronized (acquire()) {
            mark(value);
        }
    }

    static void monitorCleanup(int value) {
        synchronized (lock) {
            try {
                checkedValue(value);
            } catch (java.io.IOException problem) {
                mark(problem.getMessage().length());
            } finally {
                mark(6);
            }
        }
    }

    static void retainedMonitor() {
        Object saved = acquire();
        synchronized (saved) {
            saved.notifyAll();
            mark(3);
        }
    }

    static int declarationNearUse(int value) {
        mark(1);
        int measured = mark(value);
        return measured + measured;
    }

    static int branchLocal(boolean ready, int value) {
        mark(1);
        if (ready) {
            int measured = mark(value);
            mark(measured);
            return measured + 1;
        }
        return 0;
    }

    static int siblingLocals(boolean first) {
        if (first) {
            int width = mark(3);
            return width * width;
        } else {
            String title = String.valueOf(mark(4));
            return title.length() + title.length();
        }
    }

    static int catchLocal(int value) {
        try {
            return checkedValue(value);
        } catch (java.io.IOException problem) {
            String message = problem.getMessage();
            return message.length() + message.length();
        }
    }

    static int preservedCommit(int value) {
        int committed = 11;
        try {
            committed = checkedValue(value);
        } catch (java.io.IOException problem) {
            mark(committed);
        }
        return committed;
    }

    static int allocatedValue(int value) {
        return new ScopeProduct(mark(value)).value;
    }

    static int nestedAllocation(int value) {
        return new ScopeEnvelope(new ScopeProduct(mark(value)), mark(4)).read();
    }

    static int[] initializedArray(int value) {
        return new int[] {mark(value), mark(2), mark(3)};
    }

    static ScopeProduct[] initializedObjects(int value) {
        return new ScopeProduct[] {new ScopeProduct(mark(value)), new ScopeProduct(mark(2))};
    }

    static int[][] initializedRows(int value) {
        return new int[][] {{mark(value), mark(2)}, {mark(3)}};
    }

    int shadowedMembers(int value, int total) {
        return this.value + value + ScopeQuality.total + total;
    }

    int inheritedBinding() {
        return super.value + this.value + super.read();
    }

    int read() {
        return value;
    }

    static void print(int value) {
        System.out.println(value + ":" + trace);
        trace = 0;
    }

    public static void main(String[] args) throws java.io.IOException {
        for (int value = -1; value <= 1; value++) {
            try {
                print(returnFinally(value));
            } catch (java.io.IOException problem) {
                print(problem.getMessage().length());
            }
            combinedCleanup(value);
            print(0);
            try {
                rethrowChecked(value);
                print(1);
            } catch (java.io.IOException problem) {
                print(problem.getMessage().length());
            }
            monitorExpression(value);
            print(0);
            monitorCleanup(value);
            print(0);
            retainedMonitor();
            print(0);
            print(declarationNearUse(value));
            print(branchLocal(true, value));
            print(branchLocal(false, value));
            print(siblingLocals(true));
            print(siblingLocals(false));
            print(catchLocal(value));
            print(preservedCommit(value));
            print(allocatedValue(value));
            print(nestedAllocation(value));
            int[] values = initializedArray(value);
            print(values[0] + values[1] + values[2]);
            ScopeProduct[] products = initializedObjects(value);
            print(products[0].value + products[1].value);
            int[][] rows = initializedRows(value);
            print(rows[0][0] + rows[0][1] + rows[1][0]);
            ScopeQuality instance = new ScopeQuality(value);
            print(instance.shadowedMembers(value, value));
            print(instance.inheritedBinding());
        }
        print(new ScopeQuality().read());
    }
}

class ScopeParent {
    final int value;
    ScopeParent(int value) {
        this.value = value;
    }
    int read() {
        return value;
    }
}

class ScopeProduct {
    static int initialized = ScopeQuality.mark(7);
    final int value;
    ScopeProduct(int value) {
        this.value = value;
    }
}

class ScopeEnvelope {
    final ScopeProduct product;
    final int offset;
    ScopeEnvelope(ScopeProduct product, int offset) {
        this.product = product;
        this.offset = offset;
    }
    int read() {
        return product.value + offset;
    }
}
