//! Static declarations retain the initializer's publication and effect order.
use super::support::{Fixture, assert_absent, assert_contains, compact};
use sporeflower::{
    bytecode::{self, Operand},
    classfile::ClassFile,
};
use std::{fs, path::Path, sync::OnceLock};

fn initializer_early_return(directory: &Path) {
    let path = directory.join("StaticEarlyExit.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let code = &class
        .methods
        .iter()
        .find(|method| method.name == "<clinit>")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let calls: Vec<_> = bytecode::decode(code)
        .unwrap()
        .into_iter()
        .filter_map(|instruction| {
            let Operand::Constant(index) = instruction.operand else {
                return None;
            };
            if instruction.opcode != 0xb8 {
                return None;
            }
            let member = class.constant_pool.member_ref(index).unwrap();
            (member.owner == "StaticInitializerDriver"
                && member.name == "earlyExit"
                && member.descriptor == "()V")
                .then_some(instruction.pc as usize)
        })
        .collect();
    let [call] = calls.as_slice() else {
        panic!("one early-exit marker required")
    };
    let offsets: Vec<_> = bytes
        .windows(code.len())
        .enumerate()
        .filter_map(|(offset, part)| (part == code).then_some(offset))
        .collect();
    let [offset] = offsets.as_slice() else {
        panic!("initializer bytecode must be unique")
    };
    // Keep instruction lengths and the following branch-target frame intact.
    // Java cannot spell this early return directly in a static initializer.
    bytes[offset + call..offset + call + 3].copy_from_slice(&[0x00, 0x00, 0xb1]);
    fs::write(path, bytes).unwrap();
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        Fixture::build_transformed(
            "tests/fixtures/source_quality/static_initializers",
            "StaticInitializerDriver",
            "-g",
            initializer_early_return,
        )
    })
}

fn assert_ordered(source: &str, snippets: &[&str]) {
    let source = compact(source);
    let mut rest = source.as_str();
    for snippet in snippets {
        let snippet = compact(snippet);
        let start = rest
            .find(&snippet)
            .unwrap_or_else(|| panic!("missing ordered fragment {snippet:?}:\n{source}"));
        rest = &rest[start + snippet.len()..];
    }
}

#[test]
fn arrays_and_reference_values_belong_to_their_field_declarations() {
    let source = fixture().source("StaticArrayDeclarations");
    for declaration in [
        "static final byte[] BYTES = new byte[] { -128, 0, 127 };",
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
fn declaration_order_continues_after_an_intervening_effect() {
    let source = fixture().source("StaticPublicationOrder");
    assert_ordered(
        source,
        &[
            "static int first =",
            "static int second =",
            "static {",
            ".observe(",
            "} static int last = StaticInitializerDriver.mark(3);",
        ],
    );
    assert_contains(source, "StaticPublicationOrder.last");
    assert_absent(source, "static int last;");
    for value in [1, 3] {
        assert_eq!(
            source
                .matches(&format!("StaticInitializerDriver.mark({value})"))
                .count(),
            1,
            "{source}"
        );
    }
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
    assert_ordered(
        source,
        &[
            "static int before =",
            "static {",
            ".observe(",
            "StaticDefaultReads.self, StaticDefaultReads.later",
            "}",
            "static int self =",
            "static int later =",
        ],
    );
}

#[test]
fn declarations_resume_after_loops_and_retain_array_element_effects() {
    let source = fixture().source("StaticLoopBoundary");
    assert_ordered(
        source,
        &[
            "static int[] first =",
            "static {",
            ".observe(",
            "static int afterLoop = StaticInitializerDriver.mark(21);",
            "static {",
            ".observe(",
            "} static int[] last = new int[] { StaticInitializerDriver.mark(22), StaticInitializerDriver.mark(23) };",
        ],
    );
    assert_contains(source, "StaticLoopBoundary.afterLoop");
    assert_eq!(compact(source).matches("static{").count(), 2, "{source}");
    assert_absent(source, "last[0] =");
    assert_absent(source, "last[1] =");
}

#[test]
fn dead_locals_allow_boundaries_but_shared_locals_keep_their_block() {
    let source = fixture().source("StaticLocalBoundaries");
    assert_ordered(
        source,
        &[
            "static {",
            "int expired =",
            ".observe(",
            "} static int afterDead = StaticInitializerDriver.mark(9);",
            "static {",
            "int shared =",
            ".observe(",
            "sharedPublication = StaticInitializerDriver.mark(11);",
            ".observe(",
            "} static int tail = StaticInitializerDriver.mark(12);",
        ],
    );
    assert_contains(source, "static int sharedPublication;");
    assert_absent(source, "static int sharedPublication =");
    assert_eq!(compact(source).matches("static{").count(), 2, "{source}");
}

#[test]
fn repeated_writes_remain_ordered_before_a_later_declaration() {
    let source = fixture().source("StaticRepeatedWrites");
    assert_ordered(
        source,
        &[
            "repeated = StaticInitializerDriver.mark(30);",
            ".observe(",
            "repeated = StaticInitializerDriver.mark(31);",
            "static int later = StaticInitializerDriver.mark(32);",
        ],
    );
    for value in [30, 31, 32] {
        assert_eq!(
            source
                .matches(&format!("StaticInitializerDriver.mark({value})"))
                .count(),
            1,
            "{source}"
        );
    }
    assert_eq!(
        compact(source).matches("staticintrepeated").count(),
        1,
        "{source}"
    );
}

#[test]
fn early_initializer_return_keeps_the_following_write_conditional() {
    let source = fixture().source("StaticEarlyExit");
    assert_contains(source, "static int tail;");
    assert_contains(source, "tail = StaticInitializerDriver.mark(41);");
    assert_absent(source, "static int tail =");
    assert_absent(source, "earlyExit()");
}

#[test]
fn failure_stays_after_publication_and_the_intervening_observation() {
    let source = fixture().source("StaticFailedInitialization");
    assert_ordered(
        source,
        &[
            "static int published = StaticInitializerDriver.mark(6);",
            "static {",
            ".observe(",
            "} static int failure = StaticInitializerDriver.fail();",
            "static int unreached = StaticInitializerDriver.mark(7);",
        ],
    );
}
