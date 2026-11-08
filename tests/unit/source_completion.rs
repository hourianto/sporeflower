use super::*;
use crate::java::{
    source::{Catch, bound::Throw},
    syntax::Expr,
};

#[test]
fn pure_loop_transfers_follow_the_combined_try_catch_cleanup() {
    for transfer in [
        Statement::Continue(Some("outer".into())),
        Statement::Break(None),
    ] {
        let result = with_cleanup(vec![caught(Vec::new()), transfer], Vec::new());
        assert!(matches!(
            result.as_slice(),
            [
                Statement::Try {
                    cleanup: Some(_),
                    ..
                },
                Statement::Continue(_) | Statement::Break(_)
            ]
        ));
    }
}

fn caught(body: Body) -> Statement {
    Statement::Try {
        body: Vec::new(),
        catches: vec![Catch {
            catch_type: "java/io/IOException".into(),
            ty: "IOException".into(),
            name: "e".into(),
            local: None,
            body,
        }],
        cleanup: None,
    }
}

#[test]
fn bare_return_follows_the_combined_try_catch_cleanup() {
    let result = with_cleanup(
        vec![
            caught(vec![Statement::Return(None)]),
            Statement::Return(None),
        ],
        Vec::new(),
    );
    let [
        Statement::Try {
            catches, cleanup, ..
        },
        Statement::Return(None),
    ] = result.as_slice()
    else {
        panic!("bare completion must follow one combined try");
    };
    assert_eq!(catches.len(), 1);
    assert!(matches!(
        catches[0].body.as_slice(),
        [Statement::Return(None)]
    ));
    assert!(cleanup.is_some());
    assert!(!can_complete(&result));
}

#[test]
fn abrupt_cleanup_still_overrides_the_pending_return() {
    for cleanup in [
        Statement::Return(None),
        Statement::Throw(Throw {
            value: Expr::atom("problem"),
            effects: Default::default(),
        }),
    ] {
        let result = with_cleanup(
            vec![caught(Vec::new()), Statement::Return(None)],
            vec![cleanup],
        );
        assert!(matches!(
            result.as_slice(),
            [Statement::Try { cleanup: Some(cleanup), .. }] if cleanup.len() == 1
        ));
        assert!(!can_complete(&result));
    }
}

#[test]
fn value_and_throwing_continuations_keep_their_exception_scope() {
    for suffix in [
        Statement::Return(Some(Expr::boolean(true))),
        Statement::Throw(Throw {
            value: Expr::atom("problem"),
            effects: Default::default(),
        }),
    ] {
        let result = with_cleanup(vec![caught(Vec::new()), suffix], Vec::new());
        assert!(matches!(
            result.as_slice(),
            [Statement::Try { body, catches, .. }]
                if catches.is_empty()
                    && matches!(body.as_slice(), [Statement::Try { cleanup: None, .. }, _])
        ));
    }
}

#[test]
fn existing_cleanup_keeps_its_own_scope() {
    let inner = with_cleanup(vec![caught(Vec::new())], Vec::new());
    let result = with_cleanup(inner, Vec::new());
    assert!(matches!(
        result.as_slice(),
        [Statement::Try { body, catches, .. }]
            if catches.is_empty()
                && matches!(body.as_slice(), [Statement::Try { cleanup: Some(_), .. }])
    ));
}
