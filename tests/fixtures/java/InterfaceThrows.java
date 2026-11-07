import java.io.IOException;

interface WideThrower { void run() throws IOException; }
interface CombinedThrower extends WideThrower, Runnable {}

public class InterfaceThrows implements CombinedThrower {
    public static IOException payload = new IOException("same");
    public static class Payload {
        public Payload() throws IOException { throw payload; }
    }
    public void run() {
        try { Payload.class.newInstance(); }
        catch (InstantiationException failure) { throw new RuntimeException(); }
        catch (IllegalAccessException failure) { throw new RuntimeException(); }
    }
    public static void invoke(CombinedThrower value) {
        try {
            if (false) throw new IOException();
            value.run();
        } catch (IOException failure) {
            System.out.println(failure == payload);
        }
    }
    public static void main(String[] args) { invoke(new InterfaceThrows()); }
}
