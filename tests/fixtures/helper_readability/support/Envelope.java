package support;
public final class Envelope {
    static {
        Calls.mark("envelope-init");
        if (Calls.mode == 1) throw new RuntimeException();
    }
    public Envelope(String title, Base[] bases, Face[] faces, Base[][] nested) {
        Calls.mark("envelope");
        if (Calls.mode == 6) throw new RuntimeException();
        Calls.mark(title);
        Calls.mark(nested[0].getClass().getName());
        Calls.mark(bases[0] == null ? "null" : "value");
        Calls.mark(bases[1] == null ? "null" : "value");
        Calls.mark(faces[0] == nested[0][0] ? "same" : "different");
    }
}
