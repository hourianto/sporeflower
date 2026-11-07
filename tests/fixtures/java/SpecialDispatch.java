public class SpecialDispatch {
    static int effects;

    public synchronized int value(int amount) {
        if (!Thread.holdsLock(this)) throw new AssertionError();
        effects += amount;
        return amount + 10;
    }

    public int special(SpecialDispatch receiver, int amount) {
        return receiver.value(amount);
    }

    public int virtual(SpecialDispatch receiver, int amount) {
        return receiver.value(amount);
    }

    public static void main(String[] args) {
        SpecialDispatch instance = new SpecialChild();
        System.out.println(instance.special(instance, 1));
        System.out.println(instance.virtual(instance, 2));
        try { instance.special(null, ++effects); } catch (NullPointerException expected) {}
        System.out.println(effects);
    }
}

class SpecialChild extends SpecialDispatch {
    public int value(int amount) { return amount + 100; }
}
