//! Cleanup keeps lexical ownership and allocation facts across inner catches.
use super::support::{Fixture, assert_absent, assert_contains, compact, method};
use sporeflower::{bytecode, classfile::ClassFile};
use std::{fs, path::Path, sync::OnceLock};

fn specialize_normal_cleanup(directory: &Path) {
    let path = directory.join("CleanupQuality.class");
    let mut bytes = fs::read(&path).unwrap();
    let class = ClassFile::parse(&bytes).unwrap();
    let code = class
        .methods
        .iter()
        .find(|method| method.name == "load")
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    // The inner catch occurs before the ordinary cleanup. The first handler
    // of the outer protected range bounds its allocation-specialized copy.
    let handler = code
        .exception_table
        .iter()
        .min_by_key(|entry| (entry.start_pc, entry.handler_pc))
        .unwrap()
        .handler_pc;
    let instructions = bytecode::decode(&code.bytes).unwrap();
    let mut optimized = code.bytes.clone();
    let mut removed = 0;
    for pair in instructions.windows(2) {
        if pair[1].pc >= handler {
            break;
        }
        if pair[1].opcode == 0xc6 {
            assert_eq!(pair[0].opcode, 0x19); // decoder canonicalizes compact aload forms
            optimized[pair[0].pc as usize..pair[1].pc as usize + 3].fill(0);
            removed += 1;
        }
    }
    assert_eq!(
        removed, 2,
        "both successful allocations must specialize their guards"
    );
    let offsets: Vec<_> = bytes
        .windows(code.bytes.len())
        .enumerate()
        .filter_map(|(offset, candidate)| (candidate == code.bytes).then_some(offset))
        .collect();
    assert_eq!(offsets.len(), 1);
    bytes[offsets[0]..offsets[0] + optimized.len()].copy_from_slice(&optimized);
    fs::write(path, bytes).unwrap();
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        Fixture::build_transformed(
            "tests/fixtures/source_quality/cleanup",
            "CleanupQuality",
            "-g:none",
            specialize_normal_cleanup,
        )
    })
}

fn body() -> &'static str {
    method(fixture().source("CleanupQuality"), "load")
}

#[test]
fn optimized_resource_guards_recover_one_cleanup_scope() {
    let body = body();
    assert_eq!(compact(body).matches("finally{").count(), 1, "{body}");
    assert_eq!(body.matches(".close()").count(), 2, "{body}");
    assert_absent(body, "$sf$rethrow");
    assert_absent(body, "catch (Throwable");
    assert_absent(body, "label");
}

#[test]
fn cleanup_uses_one_catch_for_ordered_stream_closes() {
    let body = body();
    assert_contains(body, "finally { try {");
    let cleanup = body.split("finally").nth(1).unwrap();
    assert_eq!(
        compact(cleanup).matches("catch(IOException").count(),
        1,
        "{body}"
    );
    assert_contains(cleanup, ".getMessage()");
    assert_absent(cleanup, "(IOException)");
}

#[test]
fn loop_tail_catches_fall_through_their_cleanup() {
    let body = method(fixture().source("CleanupQuality"), "repeatedCleanup");
    assert_eq!(compact(body).matches("try{").count(), 1, "{body}");
    assert_eq!(compact(body).matches("finally{").count(), 1, "{body}");
    assert_contains(body, ".getMessage()");
    assert_absent(body, "(IOException)");
    assert_absent(body, "continue;");
}

#[test]
fn throwable_catch_and_synchronized_body_share_the_enclosing_cleanup() {
    let fixture = Fixture::build_stripped(
        "tests/fixtures/source_quality/shadowed_cleanup",
        "ShadowedCleanupQuality",
    );
    let body = method(fixture.source("ShadowedCleanupQuality"), "playback");
    assert_eq!(compact(body).matches("finally{").count(), 1, "{body}");
    assert_eq!(body.matches(".close()").count(), 1, "{body}");
    assert_contains(body, "synchronized (");
    assert_absent(body, "$sf$rethrow");
    assert_absent(body, "label");
    let (protected, cleanup) = body.split_once("finally").unwrap();
    assert_eq!(
        compact(protected).matches("catch(Throwable").count(),
        1,
        "{body}"
    );
    assert_contains(cleanup, ".close();");
}
