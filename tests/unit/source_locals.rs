use super::*;
use crate::{classfile::JvmType, index::test_ids::*, java::syntax::Operator};

fn binding(local: LocalId) -> Binding {
    Binding {
        local,
        name: format!("v{local}"),
        ty: JvmType::Int,
        type_name: "int".into(),
        synthetic: true,
        canonical_boolean: false,
        storage: false,
    }
}
fn inputs() -> Declarations {
    [L0, L1]
        .into_iter()
        .map(|local| {
            Some(Declaration {
                binding: binding(local),
                initialize: false,
                entry: false,
            })
        })
        .collect()
}
fn value() -> Expr {
    Expr::integer(1, JvmType::Int, "1")
}
fn assign(local: LocalId, value: Expr) -> Statement {
    Statement::Simple(Simple::Assign {
        binding: binding(local),
        value: AssignmentValue::Value(value),
        declaration: false,
    })
}
fn returned(local: LocalId) -> Statement {
    Statement::Return(Some(binding(local).expression()))
}
fn condition(local: LocalId) -> Expr {
    Expr::binary(Operator::Less, binding(local).expression(), value())
}
fn declared(statement: &Statement) -> bool {
    matches!(
        statement,
        Statement::Simple(Simple::Assign {
            declaration: true,
            ..
        })
    )
}
fn standalone(statement: &Statement, local: LocalId, initialized: bool) -> bool {
    matches!(statement, Statement::Simple(Simple::Declare { binding, initialize })
        if binding.local == local && *initialize == initialized)
}

#[test]
fn lexical_scope_escape_and_source_occurrences_choose_one_binding() {
    let mut body = vec![
        Statement::Block {
            label: None,
            body: vec![assign(L0, value())],
        },
        returned(L0),
    ];
    place(&mut body, inputs());
    assert!(standalone(&body[0], L0, false));
    let mut body = vec![assign(L0, value()), assign(L0, value()), returned(L0)];
    place(&mut body, inputs());
    assert!(declared(&body[0]));
    assert!(!declared(&body[1]));
}

#[test]
fn initializer_reads_and_excluded_bindings_keep_outer_storage() {
    let mut body = vec![assign(L0, binding(L0).expression()), returned(L0)];
    place(&mut body, inputs());
    assert!(standalone(&body[0], L0, false));

    // Parameters, lexical catches and selected-out helper fields have no input.
    let mut excluded = inputs();
    excluded[L0] = None;
    let mut body = vec![assign(L0, value()), returned(L0)];
    place(&mut body, excluded);
    assert_eq!(body.len(), 2);
    assert!(!declared(&body[0]));
}

#[test]
fn entry_defaults_stay_outside_repetition_but_dead_entry_values_can_initialize() {
    let mut defaults = inputs();
    let declaration = defaults[L0].as_mut().unwrap();
    declaration.initialize = true;
    declaration.entry = true;
    let mut body = vec![Statement::Loop {
        label: None,
        header: LoopHeader::While(Expr::boolean(true)),
        body: vec![assign(L0, value()), returned(L0)],
    }];
    place(&mut body, defaults);
    assert!(standalone(&body[0], L0, true));
    let Statement::Loop { body, .. } = &body[1] else {
        panic!()
    };
    assert!(!declared(&body[0]));

    let mut defaults = inputs();
    defaults[L0].as_mut().unwrap().initialize = true;
    let mut body = vec![assign(L0, value()), returned(L0)];
    place(&mut body, defaults);
    assert_eq!(body.len(), 2);
    assert!(declared(&body[0]));
}

#[test]
fn for_update_accesses_place_body_assignments_outside_the_header() {
    let mut declarations = inputs();
    declarations[L0] = None;
    let mut body = vec![Statement::Loop {
        label: None,
        header: LoopHeader::For {
            initializer: None,
            condition: Expr::boolean(true),
            update: Box::new(Simple::Assign {
                binding: binding(L1),
                value: AssignmentValue::Compound {
                    operator: Operator::Add,
                    value: binding(L0).expression(),
                },
                declaration: false,
            }),
        },
        body: vec![assign(L1, value()), Statement::Continue(None)],
    }];
    place(&mut body, declarations);
    assert!(standalone(&body[0], L1, false));
    let Statement::Loop { body, .. } = &body[1] else {
        panic!()
    };
    assert!(!declared(&body[0]));
}

#[test]
fn final_branches_hoist_to_their_common_source_scope_and_drop_erased_bindings() {
    let mut body = vec![Statement::Block {
        label: None,
        body: vec![
            Statement::If {
                condition: Expr::boolean(true),
                yes: vec![assign(L0, value())],
                no: vec![assign(L0, value())],
            },
            returned(L0),
        ],
    }];
    place(&mut body, inputs());
    let [Statement::Block { body, .. }] = body.as_slice() else {
        panic!()
    };
    assert!(standalone(&body[0], L0, false));
    let mut body = Vec::new();
    place(&mut body, inputs());
    assert!(body.is_empty());
}

#[test]
fn final_choice_initializer_stays_after_preceding_effects() {
    let call = |number| {
        Expr::atom("mark").invoke(vec![Expr::integer(
            number,
            JvmType::Int,
            number.to_string(),
        )])
    };
    let mut body = vec![
        Statement::Simple(Simple::Expression(call(7))),
        assign(
            L0,
            Expr::conditional(Expr::local(L1, "flag", &JvmType::Boolean), call(1), call(2)),
        ),
        Statement::Return(Some(Expr::binary(
            Operator::Add,
            binding(L0).expression(),
            binding(L0).expression(),
        ))),
    ];
    let mut declarations = inputs();
    declarations[L1] = None;
    place(&mut body, declarations);
    assert_eq!(body.len(), 3);
    assert!(matches!(body[0], Statement::Simple(Simple::Expression(_))));
    assert!(declared(&body[1]));
}

#[test]
fn for_initializer_scope_follows_final_counter_uses() {
    let loop_statement = || {
        let Statement::Simple(initializer) = assign(L0, value()) else {
            unreachable!()
        };
        Statement::Loop {
            label: None,
            header: LoopHeader::For {
                initializer: Some(Box::new(initializer)),
                condition: condition(L0),
                update: Box::new(Simple::Assign {
                    binding: binding(L0),
                    value: AssignmentValue::Increment { increment: true },
                    declaration: false,
                }),
            },
            body: vec![returned(L0)],
        }
    };
    let mut body = vec![loop_statement()];
    place(&mut body, inputs());
    let Statement::Loop {
        header:
            LoopHeader::For {
                initializer: Some(initializer),
                ..
            },
        ..
    } = &body[0]
    else {
        panic!()
    };
    assert!(matches!(
        initializer.as_ref(),
        Simple::Assign {
            declaration: true,
            ..
        }
    ));

    let mut body = vec![loop_statement(), returned(L0)];
    place(&mut body, inputs());
    assert!(standalone(&body[0], L0, false));
    let Statement::Loop {
        header:
            LoopHeader::For {
                initializer: Some(initializer),
                ..
            },
        ..
    } = &body[1]
    else {
        panic!()
    };
    assert!(matches!(
        initializer.as_ref(),
        Simple::Assign {
            declaration: false,
            ..
        }
    ));
}

#[test]
fn post_test_condition_reads_outside_the_loop_body_scope() {
    let mut body = vec![Statement::Loop {
        label: None,
        header: LoopHeader::DoWhile(condition(L0)),
        body: vec![assign(L0, value())],
    }];
    place(&mut body, inputs());
    assert!(standalone(&body[0], L0, false));
    let Statement::Loop { body, .. } = &body[1] else {
        panic!()
    };
    assert!(!declared(&body[0]));
}
