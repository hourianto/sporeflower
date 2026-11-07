public class QualitySearchDriver {
    static void result(int value) {
        System.out.println(value);
        System.out.println(QualitySearch.trace);
        QualitySearch.trace = 0;
        QualitySearch.selected = -999;
    }

    public static void main(String[] args) {
        for (int mask = -1; mask < 8; mask++) {
            try {
                QualitySearch.oneShotChoice(mask);
                result(0);
                QualitySearch.exhaustionEffects(mask);
                result(1);
                QualitySearch.wrappedChoice(mask);
                result(2);
                QualitySearch.observedCounter(mask);
                result(3);
                QualitySearch.twoTrips(mask);
                result(4);
                QualitySearch.protectedRepeat(mask);
                result(5);
                QualitySearch.effectfulBound(mask);
                result(6);
            } catch (IllegalArgumentException expected) {
                result(-1002);
            }
        }
        int[][] scans = {{}, {7}, {3, 7, 5}, {3, 5}, null};
        for (int first = 0; first < scans.length; first++) {
            for (int second = 0; second < scans.length; second++) {
                for (int wanted = 3; wanted < 9; wanted += 2) {
                    try {
                        result(QualitySearch.fallbackSearch(scans[first], scans[second], wanted));
                    } catch (NullPointerException expected) {
                        result(-1000);
                    }
                    for (int third = 0; third < scans.length; third++) {
                        try {
                            result(QualitySearch.chainedFallback(scans[first], scans[second], scans[third], wanted));
                        } catch (NullPointerException expected) {
                            result(-1001);
                        }
                    }
                }
            }
        }
        int[][] preferred = {{7}, {}, {3, 5}, {3, 7, 5}};
        int[][] fallback = {null, {7}, {5, 7}, null};
        result(QualitySearch.nestedFallback(preferred, fallback, 7));
        result(QualitySearch.nestedFallback(new int[0][], null, 7));
    }
}
