public class QualityControlDriver {
    static void result(int value) {
        System.out.println(value);
        System.out.println(QualityControl.trace);
        QualityControl.trace = 0;
    }

    public static void main(String[] args) {
        int[] values = { 3, -2, 0, 5, 5 };
        for (int end = 0; end <= values.length; end++) {
            for (int start = 0; start <= end; start++) {
                result(QualityControl.strideSum(values, start, end));
            }
            QualityControl.countDownByTwo(end);
            result(QualityControl.reverseSum(values));
        }
        result(QualityControl.reverseSum(new int[0]));
        String[] texts = { "", ":", "abc", "ab:cd", "::" };
        for (int text = 0; text < texts.length; text++) {
            for (int offset = 0; offset <= texts[text].length(); offset++) {
                result(QualityControl.scanSeparator(texts[text], offset));
            }
        }
        for (int wanted = -3; wanted <= 6; wanted++) {
            result(QualityControl.reverseFind(values, wanted));
            result(QualityControl.reverseFind(new int[0], wanted));
        }
        int[] numbers = { 0, 1, -1, 9, 10, -10, 100, 2147483647, -2147483648 };
        for (int index = 0; index < numbers.length; index++) {
            result(QualityControl.decimalDigits(numbers[index]));
            result(QualityControl.shiftUntilClear(numbers[index], 15));
        }
        int[][][] matrices = { {}, { {} }, { {1, 2}, {3}, {} }, { {1, -1, 9}, {-1}, {2, 3} } };
        for (int index = 0; index < matrices.length; index++) {
            result(QualityControl.matrixSum(matrices[index]));
            result(QualityControl.prefixRows(matrices[index]));
        }
        String[] guardedTexts = { null, "", "abc" };
        for (int text = 0; text < guardedTexts.length; text++) {
            for (int limit = -1; limit <= 4; limit++) {
                result(QualityControl.guardedLength(guardedTexts[text], limit));
            }
        }
        for (int start = -1; start <= 2; start++) {
            for (int end = 0; end <= 2; end++) {
                try {
                    QualityControl.checkedRange(start, end);
                    result(0);
                } catch (IllegalArgumentException expected) {
                    result(-1);
                }
            }
        }
        int[] commands = { -100, -1, 0, 1, 2, 3, 5, 7, 9, 1000 };
        for (int index = 0; index < commands.length; index++) {
            int command = commands[index];
            QualityControl.classify(command);
            result(0);
            result(QualityControl.denseCommand(command));
            result(QualityControl.sparseCommand(command));
            QualityControl.groupedCommands(command);
            result(0);
            QualityControl.fallthroughCommand(command);
            result(0);
            for (int count = 0; count <= 3; count++) {
                QualityControl.loopCommand(command, count);
                result(0);
            }
            for (int flag = 0; flag < 2; flag++) {
                QualityControl.nestedDecision(flag != 0, command);
                result(0);
                result(QualityControl.guardedSwitch(command, flag != 0));
            }
        }
        QualityControl.dispatchLoop(commands);
        result(0);
        QualityControl.dispatchLoop(new int[0]);
        result(0);
    }
}
