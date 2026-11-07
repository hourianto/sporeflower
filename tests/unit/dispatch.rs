use super::*;
use crate::index::BlockId;
use crate::index::test_ids::*;

fn edge(target: usize, catch_type: Option<&str>) -> HandlerEdge {
    HandlerEdge {
        target: BlockId::from_index(target),
        catch_type: catch_type.map(String::from),
        table_index: target,
    }
}

#[test]
fn first_match_dispatch_skips_repeated_types_and_entries_after_catch_all() {
    for catch_all in [None, Some("java/lang/Throwable")] {
        let table = vec![
            edge(1, Some("java/io/IOException")),
            edge(2, Some("java/io/IOException")),
            edge(3, Some("java/lang/Exception")),
            edge(4, catch_all),
            edge(5, None),
            edge(6, Some("java/lang/Error")),
        ];
        assert_eq!(
            effective(&table)
                .map(|handler| handler.target)
                .collect::<Vec<_>>(),
            [1, 3, 4].map(BlockId::from_index)
        );
    }
}

#[test]
fn range_identity_does_not_change_dispatch() {
    let left = edge(1, Some("java/lang/Exception"));
    let mut right = left.clone();
    right.table_index = 42;
    assert!(same_dispatch(&[left], &[right]));
}

#[test]
fn hierarchy_evidence_excludes_shadowed_subtypes() {
    let table = vec![
        edge(1, Some("java/lang/Exception")),
        edge(2, Some("java/io/IOException")),
        edge(3, Some("java/lang/Error")),
        edge(4, None),
        edge(5, Some("java/lang/Throwable")),
    ];
    assert_eq!(
        effective(&table)
            .map(|edge| edge.target)
            .collect::<Vec<_>>(),
        [B1, B2, B3, B4]
    );
    assert_eq!(
        effective_with(&table, |later, earlier| {
            later == "java/io/IOException" && earlier == "java/lang/Exception"
        })
        .map(|edge| edge.target)
        .collect::<Vec<_>>(),
        [B1, B3, B4]
    );
}
