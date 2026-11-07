package typesupport;

public class Token {
    public static int number = 11;
    public int value;

    public Token(int value) { this.value = value; }
    public static int read() { return number; }
}
