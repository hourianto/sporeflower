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

    public Unsynchronized reference(Unsynchronized value) {
        return value;
    }

    public Unsynchronized specialReference(Unsynchronized value) {
        return this.reference(value);
    }

    public Unsynchronized virtualReference(Unsynchronized value) {
        return this.reference(value);
    }

    public static void main(String[] args) {
        SpecialDispatch instance = new SpecialChild();
        System.out.println(instance.special(instance, 1));
        System.out.println(instance.virtual(instance, 2));
        try { instance.special(null, ++effects); } catch (NullPointerException expected) {}
        System.out.println(effects);
        Unsynchronized reference = new Unsynchronized(7);
        System.out.println(instance.specialReference(reference).mark);
        System.out.println(instance.virtualReference(reference).mark);
    }
}

class SpecialChild extends SpecialDispatch {
    public int value(int amount) { return amount + 100; }
    public Unsynchronized reference(Unsynchronized value) {
        return new Unsynchronized(value.mark + 100);
    }
}

class Unsynchronized {
    public int mark;
    public Unsynchronized(int mark) { this.mark = mark; }
}
