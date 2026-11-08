//! Cleanup owns the complete protected operation, including branches and catches.
use super::support::{Fixture, assert_absent, assert_contains, compact, method, word_count};
use sporeflower::{
    bytecode::{self, Operand},
    classfile::ClassFile,
};
use std::{fs, path::Path, sync::OnceLock};

fn share_null_cleanup(directory: &Path) {
    let path = directory.join("QualityCleanupOwnership.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let code = class
        .methods
        .iter()
        .find(|method| method.name == "find")
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let instructions = bytecode::decode(&code.bytes).unwrap();
    let handler = code
        .exception_table
        .iter()
        .find(|entry| entry.catch_type.as_deref() == Some("java/lang/Throwable"))
        .unwrap()
        .handler_pc;
    let handler_index = instructions
        .iter()
        .position(|instruction| instruction.pc == handler)
        .unwrap();
    let caught_return = &instructions[handler_index + 1..handler_index + 5];
    assert_eq!(caught_return[0].opcode, 0x01); // aconst_null
    assert_eq!(caught_return[1].opcode, 0x3a); // astore
    assert_eq!(caught_return[2].opcode, 0x19); // aload reader
    assert_eq!(caught_return[3].opcode, 0xc6); // ifnull
    let target = caught_return[2].pc;
    let mut optimized = code.bytes.clone();
    // The shared continuation returns the literal directly, as after ordinary
    // dead-store elimination. Keeping javac's pending-return local would add a
    // distinct value-provenance problem to the cleanup-ownership topology.
    let Operand::Branch(return_pc) = caught_return[3].operand else {
        panic!()
    };
    let return_index = instructions
        .iter()
        .position(|instruction| instruction.pc == return_pc)
        .unwrap();
    assert_eq!(instructions[return_index].opcode, 0x19);
    assert_eq!(instructions[return_index].operand, caught_return[1].operand);
    assert_eq!(instructions[return_index + 1].opcode, 0xb0); // areturn
    optimized[return_pc as usize..instructions[return_index + 1].pc as usize].fill(0x00);
    optimized[return_pc as usize] = 0x01;
    assert_eq!(instructions[handler_index].opcode, 0x3a);
    optimized[handler as usize..target as usize].fill(0x00);
    optimized[handler as usize] = 0x57; // pop unused caught exception
    let mut shared = 0;
    for window in instructions.windows(4) {
        if window[0].pc >= handler {
            break;
        }
        if window[0].opcode != 0x01
            || window[1].opcode != 0x3a
            || window[2].opcode != 0x19
            || window[3].opcode != 0xc6
        {
            continue;
        }
        // Deduplicate only return-null cleanup copies: both the preflight exit
        // and exhausted search enter the same cleanup as the caught failure.
        // The successful trim keeps its separate cleanup/return continuation.
        let Operand::Branch(end) = window[3].operand else {
            panic!()
        };
        let start = window[2].pc as usize;
        optimized[window[0].pc as usize..end as usize].fill(0x00);
        optimized[start] = 0xa7; // goto shared cleanup
        let displacement = i16::try_from(i64::from(target) - start as i64).unwrap();
        optimized[start + 1..start + 3].copy_from_slice(&displacement.to_be_bytes());
        shared += 1;
    }
    assert_eq!(shared, 2);
    let offsets: Vec<_> = bytes
        .windows(code.bytes.len())
        .enumerate()
        .filter_map(|(offset, candidate)| (candidate == code.bytes).then_some(offset))
        .collect();
    assert_eq!(offsets.len(), 1);
    bytes[offsets[0]..offsets[0] + optimized.len()].copy_from_slice(&optimized);
    // The changed joins use the legacy verifier instead of javac's stale maps.
    bytes[4..8].copy_from_slice(&[0, 3, 0, 45]);
    fs::write(path, bytes).unwrap();
}

fn source() -> &'static str {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            Fixture::build_transformed(
                "tests/fixtures/source_quality/cleanup_ownership",
                "QualityCleanupOwnershipDriver",
                "-g:none",
                share_null_cleanup,
            )
        })
        .source("QualityCleanupOwnership")
}

/// Return the lexical block following a construct, without depending on locals.
fn block_after<'a>(source: &'a str, construct: &str) -> &'a str {
    let start = source
        .find(construct)
        .unwrap_or_else(|| panic!("missing {construct}:\n{source}"));
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return &source[open + 1..open + offset];
        }
    }
    panic!("unclosed {construct}:\n{source}")
}

#[test]
fn synchronized_loop_exits_reset_the_flag_in_one_enclosing_finally() {
    let body = method(source(), "pump");
    assert_contains(body, "while (QualityCleanupOwnership.next())");
    assert_contains(block_after(body, "try"), "synchronized (");
    assert_eq!(word_count(body, "finally"), 1, "{body}");
    assert_eq!(compact(body).matches("busy=false;").count(), 1, "{body}");
    assert_contains(block_after(body, "finally"), "busy = false;");
    assert_absent(body, "catch (Throwable");
    assert_absent(body, "$sf$rethrow");
}

#[test]
fn shared_failure_cleanup_keeps_the_search_and_trim_in_one_try() {
    let body = method(source(), "find");
    assert_eq!(word_count(body, "finally"), 1, "{body}");
    let protected = block_after(body, "try");
    assert_contains(protected, ".ready()");
    let search = block_after(protected, "for (");
    assert_eq!(compact(search).matches(".read()").count(), 2, "{body}");
    assert_contains(search, ".equals(");
    assert!(
        search.lines().any(|line| {
            let line = line.trim();
            line.starts_with("return ") && line.ends_with(".trim();")
        }),
        "the matching branch must return the trimmed value directly:\n{body}"
    );
    assert_eq!(compact(body).matches(".close()").count(), 1, "{body}");
    assert_eq!(word_count(body, "try"), 2, "{body}");
    assert_absent(body, "label");
}

#[test]
fn catch_classification_and_reporting_share_the_entire_cleanup_scope() {
    let body = method(source(), "transfer");
    let caught = block_after(body, "catch (Throwable");
    assert_eq!(word_count(body, "finally"), 1, "{body}");
    let (protected, cleanup) = if word_count(caught, "finally") == 1 {
        // A specialized successful cleanup can leave a catch-owned finally.
        (block_after(caught, "try"), block_after(caught, "finally"))
    } else {
        // Retaining the ownership flag also permits the original outer scope.
        (caught, block_after(body, "finally"))
    };
    assert_contains(protected, ".delete()");
    assert_contains(
        protected,
        "instanceof QualityCleanupOwnership$ClassifiedFailure",
    );
    assert_contains(protected, "switch (");
    assert_contains(protected, "QualityCleanupOwnership.report(");
    assert_eq!(
        compact(cleanup)
            .matches("QualityCleanupOwnership.close(")
            .count(),
        2,
        "{body}"
    );
    assert_contains(cleanup, "QualityCleanupOwnership.release();");
    assert_contains(cleanup, "QualityCleanupOwnership.abandon();");
    assert_absent(caught, "QualityCleanupOwnership.accept()");
    assert_contains(body, "QualityCleanupOwnership.accept();");
    assert_absent(body, "$sf$rethrow");
}

#[test]
fn connection_creation_keeps_its_branch_and_protected_owner() {
    let body = method(source(), "acquire");
    let protected = block_after(body, "try");
    assert_contains(protected, ".exists())");
    assert_contains(protected, ".create();");
    assert_eq!(word_count(body, "try"), 1, "{body}");
    assert_eq!(compact(body).matches(".create()").count(), 1, "{body}");
}

#[test]
fn swallowed_failure_keeps_its_ordinary_continuation_after_the_catch() {
    let body = method(source(), "ordinaryContinuation");
    assert_eq!(word_count(body, "finally"), 0, "{body}");
    assert_eq!(word_count(body, "try"), 1, "{body}");
    assert_contains(block_after(body, "try"), "QualityCleanupOwnership.copy(");
    let caught = block_after(body, "catch (Throwable");
    assert!(caught.trim().is_empty(), "{body}");
    assert_contains(
        body.rsplit_once('}').unwrap().1,
        "QualityCleanupOwnership.finish(",
    );
    assert_absent(body, "label");
}

#[test]
fn swallowed_diagnostic_preserves_the_complete_enclosing_finally() {
    let body = method(source(), "cleanupDiagnostic");
    assert_eq!(word_count(body, "finally"), 1, "{body}");
    let cleanup = block_after(body, "finally");
    assert_contains(cleanup, "QualityCleanupOwnership.closeOutput();");
    assert_contains(
        block_after(cleanup, "try"),
        "QualityCleanupOwnership.diagnostic(",
    );
    assert_contains(
        cleanup.rsplit_once('}').unwrap().1,
        "QualityCleanupOwnership.close(",
    );
    assert_contains(cleanup, "QualityCleanupOwnership.release();");
    for effect in ["closeOutput(", "diagnostic(", "close(", "release("] {
        assert_eq!(compact(body).matches(effect).count(), 1, "{body}");
    }
    assert_absent(body, "$sf$rethrow");
}

#[test]
fn multiple_exits_inside_a_branch_share_their_own_cleanup_scope() {
    let body = method(source(), "branchCleanup");
    assert_eq!(word_count(body, "finally"), 1, "{body}");
    let finally = body.find("finally").unwrap();
    let last_try = body[..finally].rfind("try {").unwrap();
    let protected = block_after(&body[last_try..], "try");
    assert_contains(protected, "return 20;");
    assert_contains(protected, "return 30;");
    assert!(
        compact(protected)
            .matches("QualityCleanupOwnership.copy(")
            .count()
            >= 2,
        "{body}"
    );
    let cleanup = block_after(body, "finally");
    assert_contains(cleanup, "QualityCleanupOwnership.close(");
    assert_contains(cleanup, "QualityCleanupOwnership.release();");
    assert!(
        compact(body)
            .matches("QualityCleanupOwnership.close(")
            .count()
            <= 3,
        "{body}"
    );
}

#[test]
fn failure_before_opening_a_resource_returns_without_a_cleanup_scaffold() {
    let body = method(source(), "createAndRead");
    assert_contains(
        block_after(body, "try"),
        "QualityCleanupOwnership.prepareResource(",
    );
    let caught = block_after(body, "catch (Exception");
    assert_eq!(compact(caught), "returnfalse;", "{body}");
    assert_eq!(word_count(body, "finally"), 0, "{body}");
    assert_eq!(compact(body).matches(".close()").count(), 1, "{body}");
    assert_contains(body, "QualityCleanupOwnership.openReader(");
    assert_contains(body, "QualityCleanupOwnership.useReader(");
}

#[test]
fn failure_only_cleanup_keeps_its_finally_and_successful_resource_ownership() {
    let body = method(source(), "retainOnSuccess");
    assert_eq!(word_count(body, "finally"), 1, "{body}");
    let caught = block_after(body, "catch (IOException");
    assert_contains(caught, "= true;");
    assert_contains(caught, "throw new QualityCleanupOwnership$OpenFailure(");
    let cleanup = block_after(body, "finally");
    assert_contains(cleanup, "&&");
    assert_contains(cleanup, "!= null)");
    assert_contains(cleanup, ".close();");
    assert_contains(cleanup, "catch (IOException");
    assert_contains(cleanup, "catch (Error");
    assert_eq!(compact(body).matches(".close()").count(), 1, "{body}");
    assert_absent(body, "catch (Throwable");
    assert_absent(body, "$sf$rethrow");
}
