public class DiscardedReads {
    static int calls;

    static String joined() {
        return new StringBuffer()
            .append(ReadHolder.VALUE.text(ReadHolder.VALUE.FIRST))
            .append(":")
            .append(ReadHolder.VALUE.text(ReadHolder.VALUE.SECOND)).toString();
    }

    static String[] array() {
        return new String[] {
            ReadHolder.VALUE.text(ReadHolder.VALUE.FIRST),
            ReadHolder.VALUE.text(ReadHolder.VALUE.SECOND)
        };
    }

    static int volatileReads() {
        ReadText result = ReadHolder.VOLATILE;
        ReadText ignored = ReadHolder.VOLATILE;
        return result.number;
    }

    static int separateReceivers(ReadText first, ReadText second) {
        int result = first.number;
        int ignored = second.number;
        return result;
    }

    static int sameReceiver(ReadText value) {
        int result = value.number;
        int ignored = value.number;
        return result;
    }

    static ReadText effectfulReceiver() {
        calls++;
        return null;
    }

    static void receiverEffect() {
        int ignored = effectfulReceiver().number;
    }

    static int firstAccessFails() {
        int ignored = FailedReadHolder.VALUE;
        int repeated = FailedReadHolder.VALUE;
        return 1;
    }

    static int initialization() {
        System.out.println("initialize");
        return 9;
    }

    static int failInitialization() {
        System.out.println("fail initialize");
        throw new RuntimeException("init");
    }

    public static void main(String[] args) {
        System.out.println(joined());
        String[] strings = array();
        System.out.println(strings[0] + ":" + strings[1]);
        System.out.println(volatileReads());
        System.out.println(sameReceiver(ReadHolder.VALUE));
        try { sameReceiver(null); }
        catch (NullPointerException expected) { System.out.println("first null"); }
        try { separateReceivers(ReadHolder.VALUE, null); }
        catch (NullPointerException expected) { System.out.println("second null"); }
        try { receiverEffect(); }
        catch (NullPointerException expected) { System.out.println("receiver:" + calls); }
        try { firstAccessFails(); }
        catch (ExceptionInInitializerError expected) { System.out.println("init failed"); }
    }
}

class ReadHolder {
    static ReadText VALUE = new ReadText(DiscardedReads.initialization());
    static volatile ReadText VOLATILE = VALUE;
}

class FailedReadHolder {
    static int VALUE = DiscardedReads.failInitialization();
}

class ReadText {
    static final int FIRST = 1;
    static final int SECOND = 2;
    int number;

    ReadText(int number) { this.number = number; }
    String text(int index) { return Integer.toString(number + index); }
}
