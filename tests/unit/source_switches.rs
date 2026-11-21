use super::*;
use crate::java::syntax::Expr;

fn returns(cases: &[Case], selector: i32) -> bool {
    let entry = cases
        .iter()
        .position(|case| case.keys.contains(&selector))
        .or_else(|| cases.iter().position(|case| case.default))
        .unwrap();
    for case in &cases[entry..] {
        match case.body.as_slice() {
            [] => {}
            [Statement::Return(None)] => return true,
            [Statement::Return(Some(value))] => return value.boolean_constant().unwrap(),
            [Statement::Break(None)] => return false,
            _ => panic!("unexpected test arm"),
        }
    }
    false
}

#[test]
fn empty_fallthrough_labels_share_the_following_default_body() {
    let mut cases = vec![
        Case {
            keys: vec![5],
            default: false,
            scoped: true,
            body: vec![],
        },
        Case {
            keys: vec![],
            default: true,
            scoped: false,
            body: vec![],
        },
        Case {
            keys: vec![7],
            default: false,
            scoped: false,
            body: vec![Statement::Return(None)],
        },
        Case {
            keys: vec![9],
            default: false,
            scoped: false,
            body: vec![Statement::Break(None)],
        },
    ];
    let selectors = [5, 7, 9, 42];
    let before = selectors.map(|selector| returns(&cases, selector));
    group(&mut cases);
    assert_eq!(selectors.map(|selector| returns(&cases, selector)), before);
    assert_eq!(cases.len(), 2);
    assert!(cases[0].default);
    assert!(cases[0].keys.is_empty());
    assert_eq!(cases[1].keys, [9]);
}

#[test]
fn empty_entry_is_absorbed_after_terminal_merge_introduces_default() {
    let mut cases = vec![
        Case {
            keys: vec![1],
            default: false,
            scoped: false,
            body: vec![],
        },
        Case {
            keys: vec![2],
            default: false,
            scoped: false,
            body: vec![Statement::Return(Some(Expr::boolean(true)))],
        },
        Case {
            keys: vec![3],
            default: false,
            scoped: false,
            body: vec![Statement::Return(Some(Expr::boolean(false)))],
        },
        Case {
            keys: vec![],
            default: true,
            scoped: false,
            body: vec![Statement::Return(Some(Expr::boolean(true)))],
        },
    ];
    let selectors = [1, 2, 3, 42];
    let before = selectors.map(|selector| returns(&cases, selector));
    group(&mut cases);
    assert_eq!(selectors.map(|selector| returns(&cases, selector)), before);
    assert_eq!(cases.len(), 2);
    assert!(cases[0].default);
    assert!(cases[0].keys.is_empty());
    assert_eq!(cases[1].keys, [3]);
}

#[test]
fn equal_integral_values_share_cases_after_symbolic_binding() {
    let constant = |owner, name, value| {
        Expr::atom(owner)
            .member(name)
            .with_type(Some(&crate::classfile::JvmType::Int))
            .with_integer_value(value)
    };
    let mut cases: Vec<_> = [
        constant("Frames", "CANNON", 8),
        constant("OtherFrames", "GUN", 8),
        constant("Frames", "CREW", 9),
    ]
    .into_iter()
    .enumerate()
    .map(|(key, value)| Case {
        keys: vec![key as i32],
        default: false,
        scoped: false,
        body: vec![Statement::Return(Some(value))],
    })
    .collect();
    group(&mut cases);
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].keys, [0, 1]);
    assert_eq!(cases[1].keys, [2]);
}

#[test]
fn sorting_keeps_fallthrough_chains_and_the_implicit_final_exit_in_place() {
    let arm = |keys: &[i32], body| Case {
        keys: keys.to_vec(),
        default: keys.is_empty(),
        scoped: false,
        body,
    };
    let mut cases = vec![
        arm(&[11], vec![Statement::Return(None)]),
        arm(&[7, 2], vec![Statement::Break(None)]),
        arm(&[9], vec![]),
        arm(&[3], vec![Statement::Return(None)]),
        arm(&[], vec![Statement::Break(None)]),
        arm(&[1], vec![Statement::Return(None)]),
        arm(&[-1], vec![]),
    ];
    let selectors = [11, 7, 2, 9, 3, 20, 1, -1];
    let before = selectors.map(|selector| returns(&cases, selector));
    order_independent(&mut cases);
    assert_eq!(selectors.map(|selector| returns(&cases, selector)), before);
    let labels: Vec<_> = cases.iter().map(|case| case.keys.as_slice()).collect();
    assert_eq!(labels, [&[2, 7][..], &[11], &[9], &[3], &[1], &[], &[-1]]);
}
