public class QualityCleanupOwnershipDriver {
    static void require(boolean condition) {
        if (!condition) throw new Error(QualityCleanupOwnership.trace);
    }

    static void pump() {
        for (int limit = 0; limit <= 3; limit++) {
            for (int failure = -3; failure <= 3; failure++) {
                QualityCleanupOwnership.trace = "";
                QualityCleanupOwnership.busy = false;
                QualityCleanupOwnership.turn = 0;
                QualityCleanupOwnership.limit = limit;
                Throwable thrown = null;
                try { QualityCleanupOwnership.pump(failure); }
                catch (Throwable problem) { thrown = problem; }
                boolean fatal = failure < 0 && -failure <= limit;
                require(thrown == (fatal ? QualityCleanupOwnership.fatal : null));
                require(!QualityCleanupOwnership.busy);
                require(QualityCleanupOwnership.turn == (fatal ? -failure : limit));
                System.out.println("pump:" + limit + ":" + failure + ":" + QualityCleanupOwnership.trace);
            }
        }
    }

    static void search() {
        String[] entries = { "a", " alpha ", "b", " beta " };
        String[] keys = { "a", "b", "missing" };
        for (int ready = 0; ready < 2; ready++) {
            for (int failure = -2; failure <= 4; failure++) {
                for (int key = 0; key < keys.length; key++) {
                    QualityCleanupOwnership.trace = "";
                    QualityCleanupOwnership.Reader reader = new QualityCleanupOwnership.Reader(entries, ready != 0, failure);
                    String result = null;
                    Throwable thrown = null;
                    try { result = QualityCleanupOwnership.find(reader, keys[key]); }
                    catch (Throwable problem) { thrown = problem; }
                    require(reader.closes == 1);
                    require(thrown == (failure == -2 ? QualityCleanupOwnership.fatal : null));
                    int reads = key == 0 ? 2 : 4;
                    boolean found = ready != 0 && key != 2 && !(failure > 0 && failure <= reads) && failure != -2;
                    require(found ? (key == 0 ? "alpha" : "beta").equals(result) : result == null);
                    System.out.println("find:" + ready + ":" + failure + ":" + key + ":" + result + ":" + QualityCleanupOwnership.trace);
                }
            }
        }
    }

    static void transfer() {
        for (int present = 0; present < 2; present++) {
            for (int failure = 0; failure < 4; failure++) {
                for (int deletion = 0; deletion < 3; deletion++) {
                    for (int reporting = 0; reporting < 2; reporting++) {
                        QualityCleanupOwnership.trace = "";
                        QualityCleanupOwnership.Connection source = new QualityCleanupOwnership.Connection("s", present != 0, 0);
                        QualityCleanupOwnership.Connection destination = new QualityCleanupOwnership.Connection("d", true, deletion == 0 ? 0 : deletion + 3);
                        Throwable thrown = null;
                        try { QualityCleanupOwnership.transfer(source, destination, failure, reporting != 0); }
                        catch (Throwable problem) { thrown = problem; }
                        boolean caught = present != 0 && failure != 0;
                        boolean fatal = caught && (deletion == 2 || reporting != 0);
                        require(thrown == (fatal ? QualityCleanupOwnership.fatal : null));
                        require(source.closes == 1 && destination.closes == 1);
                        boolean success = present != 0 && failure == 0;
                        require(QualityCleanupOwnership.trace.endsWith(success ? "releaseaccept" : "releaseabandon"));
                        System.out.println("transfer:" + present + ":" + failure + ":" + deletion + ":" + reporting + ":" + QualityCleanupOwnership.trace);
                    }
                }
            }
        }
    }

    static void acquire() {
        for (int present = 0; present < 2; present++) {
            for (int failure = 0; failure < 4; failure++) {
                QualityCleanupOwnership.trace = "";
                QualityCleanupOwnership.Connection current = new QualityCleanupOwnership.Connection("c", false, failure);
                QualityCleanupOwnership.Connection backup = new QualityCleanupOwnership.Connection("b", present != 0, failure);
                QualityCleanupOwnership.Connection metadata = new QualityCleanupOwnership.Connection("m", true, 0);
                QualityCleanupOwnership.Connection result = null;
                Throwable thrown = null;
                try { result = QualityCleanupOwnership.acquire(current, backup, metadata); }
                catch (Throwable problem) { thrown = problem; }
                boolean failed = failure == 1 || (present == 0 ? failure == 2 : failure == 3);
                require(thrown == (failed ? QualityCleanupOwnership.io : null));
                require(result == (failed ? null : present == 0 ? current : backup));
                require(metadata.closes == 1);
                require(backup.closes == (!failed && present != 0 ? 0 : 1));
                if (result != null) require(result.closes == 0);
                System.out.println("acquire:" + present + ":" + failure + ":" + QualityCleanupOwnership.trace);
            }
        }
    }

    static void continuations() {
        for (int failure = 0; failure < 4; failure++) {
            for (int finishing = 0; finishing < 2; finishing++) {
                QualityCleanupOwnership.trace = "";
                Throwable thrown = null;
                try { QualityCleanupOwnership.ordinaryContinuation(failure, finishing != 0); }
                catch (Throwable problem) { thrown = problem; }
                require(thrown == (finishing != 0 ? QualityCleanupOwnership.fatal : null));
                require(QualityCleanupOwnership.trace.equals("copytail"));
                System.out.println("continuation:" + failure + ":" + finishing + ":" + QualityCleanupOwnership.trace);
            }
            for (int diagnostic = 0; diagnostic < 3; diagnostic++) {
                for (int handling = 0; handling < 2; handling++) {
                    QualityCleanupOwnership.trace = "";
                    QualityCleanupOwnership.Connection resource = new QualityCleanupOwnership.Connection("r", true, 0);
                    Throwable thrown = null;
                    try { QualityCleanupOwnership.cleanupDiagnostic(resource, failure, diagnostic, handling != 0); }
                    catch (Throwable problem) { thrown = problem; }
                    require(thrown == (failure != 0 && handling != 0 ? QualityCleanupOwnership.fatal : null));
                    require(resource.closes == 1);
                    require(QualityCleanupOwnership.trace.equals("copy" + (failure == 0 ? "" : "caught") + "outputdiagnosticrxrelease"));
                    System.out.println("diagnostic:" + failure + ":" + diagnostic + ":" + handling + ":" + QualityCleanupOwnership.trace);
                }
            }
        }
    }

    static void branchCleanup() {
        for (int batch = 0; batch < 2; batch++) {
            for (int option = 0; option < 3; option++) {
                for (int failure = 0; failure < 4; failure++) {
                    QualityCleanupOwnership.trace = "";
                    QualityCleanupOwnership.Connection resource = new QualityCleanupOwnership.Connection("r", true, 0);
                    int result = -1;
                    Throwable thrown = null;
                    try { result = QualityCleanupOwnership.branchCleanup(resource, batch != 0, option, failure); }
                    catch (Throwable problem) { thrown = problem; }
                    boolean worked = batch == 0 || option != 0;
                    boolean failed = worked && failure != 0;
                    if (!failed) require(thrown == null);
                    else if (failure == 1) require(thrown == QualityCleanupOwnership.io);
                    else if (failure == 2) require(thrown == QualityCleanupOwnership.fatal);
                    else require(thrown instanceof QualityCleanupOwnership.ClassifiedFailure);
                    require(result == (failed ? -1 : batch == 0 ? 40 : (option + 1) * 10));
                    require(resource.closes == (failed || batch != 0 ? 1 : 0));
                    String expected = worked ? "copy" : "";
                    if (!failed && batch != 0 && option == 2) expected += "copy";
                    if (failed || batch != 0) expected += "rx";
                    require(QualityCleanupOwnership.trace.equals(expected + "release"));
                    System.out.println("branch:" + batch + ":" + option + ":" + failure + ":" + QualityCleanupOwnership.trace);
                }
            }
        }
    }

    static void unopenedResource() {
        for (int failure = -3; failure <= 3; failure++) {
            for (int closing = -2; closing <= 0; closing++) {
                QualityCleanupOwnership.trace = "";
                QualityCleanupOwnership.Reader reader = new QualityCleanupOwnership.Reader(new String[] { "value" }, true, closing);
                boolean result = false;
                Throwable thrown = null;
                try { result = QualityCleanupOwnership.createAndRead(reader, failure); }
                catch (Throwable problem) { thrown = problem; }
                boolean opened = failure == 0 || failure == -3 || failure == 3;
                boolean closed = opened && failure != 0;
                require(thrown == (closed && closing == -2 ? QualityCleanupOwnership.fatal : null));
                require(result == (failure == 0));
                require(reader.closes == (closed ? 1 : 0));
                String expected = "create";
                if (failure != -1 && failure != 1) expected += "open";
                if (opened) expected += "use";
                if (failure == 0) expected += "r";
                if (closed) expected += "x";
                require(QualityCleanupOwnership.trace.equals(expected));
                System.out.println("unopened:" + failure + ":" + closing + ":" + result + ":" + QualityCleanupOwnership.trace);
            }
        }
    }

    static void retainedResource() {
        int[] failures = { 0, 2, 3, -2, -3 };
        for (int index = 0; index < failures.length; index++) {
            int failure = failures[index];
            for (int closing = -2; closing <= 0; closing++) {
                QualityCleanupOwnership.trace = "";
                QualityCleanupOwnership.Reader reader = new QualityCleanupOwnership.Reader(new String[] { "value" }, true, closing);
                QualityCleanupOwnership.Reader result = null;
                Throwable thrown = null;
                try { result = QualityCleanupOwnership.retainOnSuccess(reader, failure); }
                catch (Throwable problem) { thrown = problem; }
                if (failure > 0) {
                    require(thrown instanceof QualityCleanupOwnership.OpenFailure);
                    require(((QualityCleanupOwnership.OpenFailure) thrown).original == QualityCleanupOwnership.io);
                } else {
                    require(thrown == (failure < 0 ? QualityCleanupOwnership.fatal : null));
                }
                require(result == (failure == 0 ? reader : null));
                require(reader.closes == (failure == 3 ? 1 : 0));
                String expected = "open";
                if (failure != 2 && failure != -2) expected += "use";
                if (failure == 0) expected += "r";
                if (failure == 3) {
                    expected += "x";
                    if (closing == -1) expected += "close-io";
                    if (closing == -2) expected += "close-error";
                }
                require(QualityCleanupOwnership.trace.equals(expected));
                System.out.println("retained:" + failure + ":" + closing + ":" + QualityCleanupOwnership.trace);
            }
        }
    }

    public static void main(String[] arguments) {
        pump();
        search();
        transfer();
        acquire();
        continuations();
        branchCleanup();
        unopenedResource();
        retainedResource();
    }
}
