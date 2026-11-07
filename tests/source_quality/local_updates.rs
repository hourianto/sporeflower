//! Local updates own both their committed value and their old expression result.
use super::support::{Fixture, assert_absent, assert_contains, method};
use sporeflower::classfile::ClassFile;
use std::{fs, path::Path, sync::OnceLock};

/// Retain the old stack operand across the arithmetic/store, as CLDC
/// optimizers do. The final DUP/store retains javac's debug-local frame too.
fn stack_countdown(directory: &Path) {
    let path = directory.join("QualityLocalUpdates.class");
    let mut bytes = fs::read(&path).unwrap();
    replace_countdown(
        &mut bytes,
        "stackCountdown",
        &[0x1a, 0x3c, 0x1a, 0x04, 0x64, 0x3b, 0x1b],
        &[0x1a, 0x59, 0x04, 0x64, 0x3b, 0x59, 0x3c],
    );
    // The long parameter leaves JVM slot 1 unused. The decrement remains in
    // slot 2 while its compact frame index is 1, as in CLDC countdown loops.
    replace_countdown(
        &mut bytes,
        "wideSlotCountdown",
        &[0x1c, 0x36, 0x05, 0x1c, 0x04, 0x64, 0x3d, 0x15, 0x05],
        &[0x1c, 0x59, 0x04, 0x64, 0x3d, 0x59, 0x36, 0x05, 0x00],
    );
    fs::write(path, bytes).unwrap();
}

fn replace_countdown(bytes: &mut [u8], name: &str, old: &[u8], replacement: &[u8]) {
    assert_eq!(old.len(), replacement.len());
    let class = ClassFile::parse(bytes).unwrap();
    let code = &class
        .methods
        .iter()
        .find(|method| method.name == name)
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let at = code
        .windows(old.len())
        .position(|part| part == old)
        .unwrap();
    let starts: Vec<_> = bytes
        .windows(code.len())
        .enumerate()
        .filter_map(|(index, part)| (part == code).then_some(index))
        .collect();
    let [start] = starts.as_slice() else {
        panic!("stack countdown bytecode must be unique");
    };
    assert_eq!(
        u32::from_be_bytes(bytes[start - 4..*start].try_into().unwrap()) as usize,
        code.len()
    );
    let stack = u16::from_be_bytes(bytes[start - 8..start - 6].try_into().unwrap()).max(3);
    bytes[start - 8..start - 6].copy_from_slice(&stack.to_be_bytes());
    bytes[start + at..start + at + old.len()].copy_from_slice(replacement);
}

fn fixture(stripped: bool) -> &'static Fixture {
    static DEBUG: OnceLock<Fixture> = OnceLock::new();
    static STRIPPED: OnceLock<Fixture> = OnceLock::new();
    (if stripped { &STRIPPED } else { &DEBUG }).get_or_init(|| {
        let directory = "tests/fixtures/source_quality/local_updates";
        Fixture::build_transformed(
            directory,
            "QualityLocalUpdates",
            if stripped { "-g:none" } else { "-g" },
            stack_countdown,
        )
    })
}

#[test]
fn arithmetic_stores_and_iinc_share_postfix_loop_guards() {
    for stripped in [false, true] {
        for name in [
            "padding",
            "escapingCountdown",
            "iincCountdown",
            "addNegative",
            "stackCountdown",
        ] {
            let body = method(fixture(stripped).source("QualityLocalUpdates"), name);
            assert_contains(body, "-- > 0)");
            assert_absent(body, "while (true)");
            assert_absent(body, "break;");
        }
    }
}

#[test]
fn ordinary_argument_consumers_can_own_postfix_updates() {
    for stripped in [false, true] {
        for name in ["callOld", "updateBeforeFailure"] {
            let body = method(fixture(stripped).source("QualityLocalUpdates"), name);
            assert_contains(body, "++,");
            assert_absent(body, " + 1");
        }
    }
}

#[test]
fn repeated_old_values_and_earlier_new_value_observers_keep_snapshots() {
    for stripped in [false, true] {
        for name in [
            "repeatedOld",
            "oldAfterNew",
            "olderSnapshot",
            "handlerObserved",
        ] {
            let body = method(fixture(stripped).source("QualityLocalUpdates"), name);
            assert_absent(body, "++");
        }
    }
}

#[test]
fn independent_arithmetic_sentinels_do_not_become_mutations() {
    for stripped in [false, true] {
        let body = method(
            fixture(stripped).source("QualityLocalUpdates"),
            "convergenceSentinel",
        );
        assert_contains(body, " + 1");
        assert_eq!(
            body.matches("++").count(),
            1,
            "only the iteration counter is updated:\n{body}"
        );
        let neighbor = method(
            fixture(stripped).source("QualityLocalUpdates"),
            "independentLoopNeighbor",
        );
        assert_contains(neighbor, " + 1");
        assert_absent(neighbor, "++");
    }
}

#[test]
fn stack_countdowns_keep_slot_identity_after_wide_locals() {
    for stripped in [false, true] {
        let body = method(
            fixture(stripped).source("QualityLocalUpdates"),
            "wideSlotCountdown",
        );
        assert_contains(body, "do {");
        assert_contains(body, "-- >= 0)");
        assert_absent(body, "while (true)");
        assert_absent(body, "break;");
    }
}

#[test]
fn loop_carried_commits_own_delayed_arithmetic_updates() {
    for stripped in [false, true] {
        let source = fixture(stripped).source("QualityLocalUpdates");
        let countdown = method(source, "delayedCountdown");
        assert_contains(countdown, "-- >= 0)");
        assert_absent(countdown, "while (true)");
        assert_absent(countdown, "break;");
        let increment = method(source, "delayedIncrement");
        assert_contains(increment, "++)");
        assert_absent(increment, " + 1");
    }
}

#[test]
fn conditional_cursor_chains_keep_argument_postfix_updates() {
    let body = method(fixture(true).source("QualityLocalUpdates"), "chainedCursor");
    assert_contains(body, "for (");
    assert_contains(body, "++)");
    assert_absent(body, " + 1");
}
