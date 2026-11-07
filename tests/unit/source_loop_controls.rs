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
                header: LoopHeader::While(Expression::Boolean(true)),
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
