use super::*;

fn empty_ir() -> MethodIr {
    MethodIr {
        entry: crate::ir::BlockId::from_index(0),
        values: Default::default(),
        blocks: Default::default(),
        locals: Default::default(),
        parameters: vec![],
        return_type: crate::classfile::JvmType::Void,
        initialization_sites: vec![],
    }
}

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
            [Statement::Return(Some(Expression::Boolean(value)))] => return *value,
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
    group(&empty_ir(), &mut cases);
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
            body: vec![Statement::Return(Some(Expression::Boolean(true)))],
        },
        Case {
            keys: vec![3],
            default: false,
            scoped: false,
            body: vec![Statement::Return(Some(Expression::Boolean(false)))],
        },
        Case {
            keys: vec![],
            default: true,
            scoped: false,
            body: vec![Statement::Return(Some(Expression::Boolean(true)))],
        },
    ];
    let selectors = [1, 2, 3, 42];
    let before = selectors.map(|selector| returns(&cases, selector));
    group(&empty_ir(), &mut cases);
    assert_eq!(selectors.map(|selector| returns(&cases, selector)), before);
    assert_eq!(cases.len(), 2);
    assert!(cases[0].default);
    assert!(cases[0].keys.is_empty());
    assert_eq!(cases[1].keys, [3]);
}
