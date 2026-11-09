use crate::common::{compile, decompile, execute, source, sources};
use std::{fs, path::PathBuf};

#[test]
fn discarded_reads_keep_first_access_effects_and_release_source_expressions() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/DiscardedReads.java")],
        &original,
        None,
    );
    let expected = execute(&original, "DiscardedReads");
    decompile(&original, &output);
    let emitted = fs::read_to_string(output.join("DiscardedReads.java")).unwrap();
    let joined = source::method(&emitted, "joined");
    assert!(!joined.contains("StringBuffer"), "{joined}");
    assert_eq!(joined.matches("ReadHolder.VALUE").count(), 2, "{joined}");
    let array = source::method(&emitted, "array");
    assert!(array.contains("new String[] {"), "{array}");
    assert_eq!(array.matches("ReadHolder.VALUE").count(), 2, "{array}");
    let volatile = source::method(&emitted, "volatileReads");
    assert_eq!(
        volatile.matches("ReadHolder.VOLATILE").count(),
        2,
        "{volatile}"
    );
    compile(&sources(&output), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, "DiscardedReads"));
}
