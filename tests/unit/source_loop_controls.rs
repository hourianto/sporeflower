use super::*;
use crate::java::source::completion;

fn overridden(transfer: Source) -> Body {
    vec![Source::Try {
        body: vec![transfer],
        catches: Vec::new(),
        cleanup: Some(vec![Source::Return(None)]),
    }]
}

#[test]
fn an_overriding_cleanup_does_not_remove_lexical_transfer_ownership() {
    let body = overridden(Source::Break(None));
    assert!(!completion::breaks_loop(&body, &None));
    assert!(completion::targets_loop(&body, &None, false));
    let body = overridden(Source::Continue(None));
    assert!(completion::targets_loop(&body, &None, true));
    assert!(!completion::targets_loop(&body, &None, false));
}

#[test]
fn nested_loops_own_bare_transfers_but_not_outer_labels() {
    let label = Some("outer".to_owned());
    for continuing in [false, true] {
        let transfer = |target| {
            if continuing {
                Source::Continue(target)
            } else {
                Source::Break(target)
            }
        };
        let nested = |body| {
            vec![Source::Loop {
                label: None,
                header: LoopHeader::While(Expr::boolean(true)),
                body,
            }]
        };
        assert!(!completion::targets_loop(
            &nested(overridden(transfer(None))),
            &label,
            continuing
        ));
        assert!(completion::targets_loop(
            &nested(overridden(transfer(label.clone()))),
            &label,
            continuing
        ));
    }
}

#[test]
fn tail_continue_removal_keeps_finally_overrides_and_nested_targets() {
    let label = Some("outer".to_owned());
    let mut body = vec![Source::Try {
        body: vec![Source::Continue(label.clone())],
        catches: Vec::new(),
        cleanup: Some(vec![Source::Continue(label.clone())]),
    }];
    remove_tail_continues(&mut body, &label);
    let Source::Try { body, cleanup, .. } = &body[0] else {
        panic!()
    };
    assert!(body.is_empty());
    assert!(matches!(cleanup.as_deref(), Some([Source::Continue(_)])));

    let mut body = vec![Source::Loop {
        label: None,
        header: LoopHeader::While(Expr::boolean(true)),
        body: vec![Source::Continue(label.clone())],
    }];
    remove_tail_continues(&mut body, &label);
    let Source::Loop { body, .. } = &body[0] else {
        panic!()
    };
    assert!(matches!(body.as_slice(), [Source::Continue(_)]));
}
