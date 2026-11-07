import pkg.TestConsecutiveReads;
import pkg.TestDuplicatedRandomSeed;
import pkg.TestStackSavedAccumulator;
import pkg.TestAllocationIdentity;
import pkg.TestArrayPostIncrementArgument;

public class LegacyDriver {
    public static void main(String[] args) {
        TestConsecutiveReads.calls = 0;
        System.out.println(TestConsecutiveReads.distinct());
        System.out.println(TestConsecutiveReads.calls);
        TestConsecutiveReads.calls = 0;
        System.out.println(TestConsecutiveReads.shared());
        System.out.println(TestConsecutiveReads.calls);
        TestDuplicatedRandomSeed.state = new TestDuplicatedRandomSeed();
        for (int i = -3; i < 7; i++) {
            TestDuplicatedRandomSeed.state.seed = i * 1010101;
            System.out.println(TestDuplicatedRandomSeed.next((i & 1) != 0));
            System.out.println(TestDuplicatedRandomSeed.state.seed);
        }
        TestStackSavedAccumulator[] records = new TestStackSavedAccumulator[5];
        for (int i = 0; i < records.length; i++) records[i] = new TestStackSavedAccumulator(i * 19);
        int[] result = TestStackSavedAccumulator.accumulate(records, 11);
        System.out.println(result[0]);
        System.out.println(result[1]);
        for (int i = 0; i < records.length; i++) System.out.println(records[i].value);
        for (int i = 0; i < 8; i++) {
            System.out.println(TestAllocationIdentity.aliases(i));
            System.out.println(TestAllocationIdentity.alternatives(i));
            System.out.println(TestAllocationIdentity.branchAliases(i));
            System.out.println(TestAllocationIdentity.overwrittenAlias(i));
            System.out.println(TestAllocationIdentity.distinct(i));
            System.out.println(TestAllocationIdentity.loop(i));
            System.out.println(TestAllocationIdentity.protectedArguments(i));
            System.out.println(TestAllocationIdentity.guardedArguments(i));
        }
        TestArrayPostIncrementArgument a = new TestArrayPostIncrementArgument();
        a.values = new byte[] {0, 0, 0, 127, -128, 4};
        for (int i = 0; i < 3; i++) {
            a.draw(i, true);
            System.out.println(a.observed);
            System.out.println(a.stored);
        }
        System.out.println(TestArrayPostIncrementArgument.twice(new byte[] {-128}));
        int[] values = new int[] {42};
        System.out.println(TestArrayPostIncrementArgument.aroundMutation(values));
        System.out.println(values[0]);
    }
}
