//! Static declarations retain the initializer's publication and effect order.
use super::support::{Fixture, assert_absent, assert_contains, compact};
use std::sync::OnceLock;

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        Fixture::build(
            "tests/fixtures/source_quality/static_initializers",
            "StaticInitializerDriver",
        )
    })
}

#[test]
fn arrays_and_reference_values_belong_to_their_field_declarations() {
    let source = fixture().source("StaticArrayDeclarations");
    for declaration in [
        "static final byte[] BYTES = new byte[] { Byte.MIN_VALUE, 0, Byte.MAX_VALUE };",
        "static final int[] SINGLE = new int[] { 7 };",
        "static final int[][] ROWS = new int[][] { { 1 }, { 2, 3 } };",
        "static final String[] WORDS = new String[] { \"first\", \"second\" };",
        "static final StringBuffer TEXT = new StringBuffer(\"ready\");",
    ] {
        assert_contains(source, declaration);
    }
    assert_absent(source, "static {");
    assert_absent(source, "[0] =");
    assert_absent(source, "[1] =");
}

#[test]
fn declaration_order_follows_publication_until_an_intervening_effect() {
    let source = fixture().source("StaticPublicationOrder");
    let source = compact(source);
    let first = source
        .find("staticintfirst=")
        .unwrap_or_else(|| panic!("{source}"));
    let second = source
        .find("staticintsecond=")
        .unwrap_or_else(|| panic!("{source}"));
    let block = source.find("static{").unwrap_or_else(|| panic!("{source}"));
    let observation = source
        .find(".observe(")
        .unwrap_or_else(|| panic!("{source}"));
    let last = source
        .find("last=StaticInitializerDriver.mark(3)")
        .unwrap_or_else(|| panic!("{source}"));
    assert!(
        first < second && second < block && block < observation && observation < last,
        "{source}"
    );
    assert_contains(&source, "static int last;");
    assert_eq!(
        source.matches("StaticInitializerDriver.mark(1)").count(),
        1,
        "{source}"
    );
    assert_eq!(
        source.matches("StaticInitializerDriver.mark(3)").count(),
        1,
        "{source}"
    );
}

#[test]
fn declaration_reads_keep_forward_and_self_references_qualified() {
    let source = fixture().source("StaticDefaultReads");
    assert_contains(source, "static int before = StaticDefaultReads.later;");
    assert_contains(
        source,
        "static int self = StaticInitializerDriver.mark(StaticDefaultReads.self + 1);",
    );
    assert_contains(
        source,
        "static int later = StaticInitializerDriver.mark(4);",
    );
    assert_absent(source, "static {");
}
