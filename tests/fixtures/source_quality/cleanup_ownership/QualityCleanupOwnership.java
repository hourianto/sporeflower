import java.io.IOException;

public class QualityCleanupOwnership {
    static String trace;
    static boolean busy;
    static int turn;
    static int limit;
    static final Object lock = new Object();
    static final IOException io = new IOException("read");
    static final Error fatal = new Error("fatal");

    static boolean next() {
        if (busy) throw new Error("cleanup did not precede the loop guard");
        trace += "g";
        return turn < limit;
    }

    static void pump(int failure) {
        while (next()) {
            try {
                synchronized (lock) {
                    busy = true;
                    turn++;
                    trace += "w";
                    if (failure == turn) throw io;
                    if (failure == -turn) throw fatal;
                }
            } catch (IOException caught) {
                trace += "c";
                continue;
            } finally {
                busy = false;
            }
        }
    }

    static class Reader {
        String[] entries;
        boolean ready;
        int failure;
        int position;
        int closes;

        Reader(String[] entries, boolean ready, int failure) {
            this.entries = entries;
            this.ready = ready;
            this.failure = failure;
        }

        boolean ready() { trace += "p"; return ready; }
        int count() { trace += "n"; return entries.length / 2; }
        String read() throws IOException {
            trace += "r";
            if (++position == failure) throw io;
            return entries[position - 1];
        }
        void close() throws IOException {
            closes++;
            trace += "x";
            if (failure == -1) throw io;
            if (failure == -2) throw fatal;
        }
    }

    static String find(Reader reader, String key) {
        try {
            if (!reader.ready()) return null;
            for (int remaining = reader.count(); remaining > 0; remaining--) {
                String candidate = reader.read();
                String value = reader.read();
                if (candidate.equals(key)) return value.trim();
            }
            return null;
        } catch (Throwable problem) {
            return null;
        } finally {
            if (reader != null) {
                try { reader.close(); } catch (Exception ignored) { }
            }
        }
    }

    static class Connection {
        String name;
        boolean present;
        int failure;
        int closes;

        Connection(String name, boolean present, int failure) {
            this.name = name;
            this.present = present;
            this.failure = failure;
        }

        boolean exists() throws IOException {
            trace += name + "?";
            if (failure == 1) throw io;
            return present;
        }
        void create() throws IOException {
            trace += name + "+";
            if (present) throw new Error("creating an existing connection");
            if (failure == 2) throw io;
            present = true;
        }
        void rename() throws IOException {
            trace += name + "m";
            if (failure == 3) throw io;
        }
        void delete() throws IOException {
            trace += name + "-";
            if (failure == 4) throw io;
            if (failure == 5) throw fatal;
        }
        void close() { trace += name + "x"; closes++; }
    }

    static void close(Connection connection) {
        if (connection != null) connection.close();
    }

    static void copy(Connection source, int failure) throws IOException {
        trace += "copy";
        if (failure == 1) throw io;
        if (failure == 2) throw fatal;
        if (failure == 3) throw new ClassifiedFailure(7);
    }

    static class ClassifiedFailure extends IOException {
        int category;
        ClassifiedFailure(int category) { this.category = category; }
    }

    static void report(int category, Throwable problem, boolean fail) {
        trace += "report" + category;
        if (fail) throw fatal;
    }

    static void release() { trace += "release"; }
    static void abandon() { trace += "abandon"; }
    static void accept() { trace += "accept"; }

    static void transfer(Connection source, Connection destination, int failure, boolean reportFailure) {
        boolean success = false;
        try {
            if (!source.exists()) return;
            copy(source, failure);
            success = true;
        } catch (Throwable problem) {
            if (destination != null) {
                try { destination.delete(); } catch (Exception ignored) { }
            }
            int category = 4;
            if (problem instanceof ClassifiedFailure) {
                category = ((ClassifiedFailure) problem).category;
            } else if (problem instanceof IOException) {
                category = 2;
            }
            switch (category) {
                case 2: category = destination == null ? 3 : 5; break;
                case 7: category = 8; break;
                default: break;
            }
            report(category, problem, reportFailure);
            return;
        } finally {
            close(destination);
            close(source);
            release();
            if (!success) abandon();
        }
        accept();
    }

    static Connection acquire(Connection current, Connection backup, Connection metadata) throws IOException {
        boolean retained = false;
        try {
            if (!backup.exists()) {
                current.create();
                return current;
            }
            current.close();
            backup.rename();
            metadata.rename();
            retained = true;
            return backup;
        } finally {
            if (!retained) close(backup);
            close(metadata);
        }
    }

    static void finish(boolean fail) {
        trace += "tail";
        if (fail) throw fatal;
    }

    static void ordinaryContinuation(int failure, boolean finishFailure) {
        try { copy(null, failure); } catch (Throwable ignored) { }
        finish(finishFailure);
    }

    static void closeOutput() { trace += "output"; }

    static void diagnostic(int failure) throws IOException {
        trace += "diagnostic";
        if (failure == 1) throw io;
        if (failure == 2) throw fatal;
    }

    static void cleanupDiagnostic(Connection resource, int failure, int diagnosticFailure, boolean handlerFailure) {
        try {
            copy(resource, failure);
        } catch (Throwable problem) {
            trace += "caught";
            if (handlerFailure) throw fatal;
        } finally {
            closeOutput();
            try { diagnostic(diagnosticFailure); } catch (Throwable ignored) { }
            close(resource);
            release();
        }
    }

    static int branchCleanup(Connection resource, boolean batch, int option, int failure) throws IOException {
        boolean retained = false;
        try {
            if (batch) {
                if (option == 0) return 10;
                copy(resource, failure);
                if (option == 1) return 20;
                copy(resource, 0);
                return 30;
            }
            copy(resource, failure);
            retained = true;
            return 40;
        } finally {
            if (!retained) close(resource);
            release();
        }
    }

    static void prepareResource(int failure) throws IOException {
        trace += "create";
        if (failure == 1) throw io;
        if (failure == -1) throw fatal;
    }

    static Reader openReader(Reader supplied, int failure) throws IOException {
        trace += "open";
        if (failure == 2) throw io;
        if (failure == -2) throw fatal;
        return supplied;
    }

    static void useReader(Reader resource, int failure) throws IOException {
        trace += "use";
        if (failure == 3) throw io;
        if (failure == -3) throw fatal;
        resource.read();
    }

    static boolean createAndRead(Reader supplied, int failure) {
        Reader resource = null;
        try {
            try { prepareResource(failure); } catch (Exception ignored) { return false; }
            resource = openReader(supplied, failure);
            useReader(resource, failure);
            return true;
        } catch (Throwable ignored) { }
        if (resource != null) {
            try { resource.close(); } catch (IOException ignored) { }
        }
        return false;
    }

    static class OpenFailure extends Exception {
        final Throwable original;
        OpenFailure(Throwable original) { this.original = original; }
    }

    static Reader retainOnSuccess(Reader supplied, int failure) throws OpenFailure {
        Reader resource = null;
        boolean failed = false;
        try {
            resource = openReader(supplied, failure);
            useReader(resource, failure);
            return resource;
        } catch (IOException problem) {
            failed = true;
            throw new OpenFailure(problem);
        } finally {
            if (failed && resource != null) {
                try { resource.close(); }
                catch (IOException ignored) { trace += "close-io"; }
                catch (Error ignored) { trace += "close-error"; }
            }
        }
    }
}
