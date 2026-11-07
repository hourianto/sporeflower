public class NullStorageTypes {
    static int calls;
    static int failAt;
    static int observed;

    static class Resource {
        final int number;
        Resource(int number) { this.number = number; }
    }

    static int select(Object value) { return 1; }
    static int select(Resource value) { return 2; }

    static Resource open(int number) {
        if (++calls == failAt) throw new IllegalStateException();
        return new Resource(number);
    }

    static void use(Resource value) {
        observed = observed * 7 + value.number;
        if (++calls == failAt) throw new IllegalArgumentException();
    }

    static int run(int count) {
        Resource current = null;
        try {
            for (int index = 0; index < count; index++) {
                current = open(index + 1);
                use(current);
                current = null;
            }
            return select((Object) current) * 10 + select(current);
        } finally {
            observed = observed * 7 + (current == null ? 0 : current.number);
        }
    }

    public static void main(String[] args) {
        for (int count = 0; count < 5; count++) {
            for (failAt = 0; failAt < 11; failAt++) {
                calls = 0;
                observed = 0;
                try {
                    System.out.print(run(count));
                } catch (IllegalStateException expected) {
                    System.out.print("open");
                } catch (IllegalArgumentException expected) {
                    System.out.print("use");
                }
                System.out.print(":");
                System.out.print(calls);
                System.out.print(":");
                System.out.println(observed);
            }
        }
    }
}
