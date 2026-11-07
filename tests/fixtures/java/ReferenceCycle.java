public class ReferenceCycle {
    interface Face {}
    static class Left implements Face {}
    static class Right implements Face {}

    public static Face rotate(Face seed, int count) {
        Face left = new Left();
        Face right = new Right();
        Face third = seed;
        while (count-- > 0) {
            Face saved = left;
            left = right;
            right = third;
            third = saved;
        }
        return left;
    }

    public static Face[] rotateArrays(Face[] seed, int count) {
        Face[] left = new Left[] { new Left() };
        Face[] right = new Right[] { new Right() };
        Face[] third = seed;
        while (count-- > 0) {
            Face[] saved = left;
            left = right;
            right = third;
            third = saved;
        }
        return left;
    }

    static int tag(Face result, Face seed) {
        if (result == null) return 0;
        if (result == seed) return 3;
        return result instanceof Left ? 1 : 2;
    }

    public static void main(String[] args) {
        for (int mode = 0; mode < 3; mode++) {
            Face seed = mode == 0 ? null : mode == 1 ? (Face) new Left() : new Right();
            Face[] array = mode == 0 ? null : new Face[] { seed };
            for (int count = 0; count < 12; count++) {
                Face result = rotate(seed, count);
                Face[] values = rotateArrays(array, count);
                System.out.println(mode + ":" + count + ":" + tag(result, seed) + ":" + (values == null ? 0 : values == array ? 3 : tag(values[0], seed)));
            }
        }
    }
}
