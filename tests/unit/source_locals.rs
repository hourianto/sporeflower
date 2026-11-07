use super::*;
use crate::classfile::JvmType;
use crate::index::test_ids::*;
use crate::index::{LocalId, ValueId};
use crate::ir::ValueUse;
use crate::ir::{IrBlock, Literal, Local, Terminator, Value, ValueKind};

fn fixture() -> MethodIr {
    {
        let mut ir = MethodIr {
            entry: B0,
            values: vec![
                Value {
                    local: None,
                    ty: JvmType::Int,
                    kind: ValueKind::Literal(Literal::Int(1)),
                    pc: 0,
                    block: B0,
                },
                Value {
                    local: None,
                    ty: JvmType::Int,
                    kind: ValueKind::Phi {
                        inputs: Vec::new(),
                        slot: Some(0),
                    },
                    pc: 1,
                    block: B1,
                },
            ]
            .into(),
            blocks: vec![
                IrBlock {
                    caught_exception: None,
                    pc: 0,
                    statements: vec![crate::ir::Statement::Assign {
                        local: L0,
                        value: ValueUse::Reference(V0),
                    }],
                    terminator: Terminator::Goto(B1),
                    handlers: Vec::new(),
                },
                IrBlock {
                    caught_exception: None,
                    pc: 1,
                    statements: Vec::new(),
                    terminator: Terminator::Return(Some(V1)),
                    handlers: Vec::new(),
                },
            ]
            .into(),
            locals: vec![Local {
                name_hint: None,
                name: "v0".into(),
                ty: JvmType::Int,
                parameter: None,
                is_this: false,
                needs_default: false,
            }]
            .into(),
            parameters: Vec::new(),
            return_type: JvmType::Int,
            initialization_sites: Vec::new(),
        };
        for (entity, metadata) in ir.values.iter_mut().zip(vec![None, Some(0)]) {
            entity.local = metadata.map(LocalId::from_index);
        }
        ir
    }
}
fn assign(local: usize, value: usize) -> Statement {
    Statement::Simple(Simple::Assign {
        local: LocalId::from_index(local),
        value: Expression::Value(ValueUse::Reference(ValueId::from_index(value))),
        declaration: false,
    })
}
fn returned() -> Statement {
    Statement::Return(Some(Expression::Value(ValueUse::Reference(V1))))
}
fn prepare(ir: &MethodIr, body: &mut Body) {
    place(ir, body, &HashSet::new(), None, false);
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

#[test]
fn lexical_scope_escape_and_source_occurrences_choose_one_binding() {
    let ir = fixture();
    let mut body = vec![
        Statement::Block {
            label: None,
            body: vec![assign(0, 0)],
        },
        returned(),
    ];
    prepare(&ir, &mut body);
    assert!(matches!(
        body[0],
        Statement::Simple(Simple::Declare { local: L0, .. })
    ));
    let mut body = vec![assign(0, 0), assign(0, 0), returned()];
    prepare(&ir, &mut body);
    assert!(declared(&body[0]));
    assert!(!declared(&body[1]));
}

#[test]
fn initializer_reads_and_shared_helper_fields_keep_outer_bindings() {
    let mut ir = fixture();
    let mut body = vec![assign(0, 0), returned()];
    prepare(&ir, &mut body);
    assert!(declared(&body[0]));
    ir.locals[L0].needs_default = true;
    let mut body = vec![assign(0, 0), returned()];
    prepare(&ir, &mut body);
    assert!(declared(&body[0]));
    let mut body = vec![assign(0, 0), returned()];
    place(
        &ir,
        &mut body,
        &HashSet::new(),
        Some(&BTreeSet::new()),
        false,
    );
    assert!(!declared(&body[0]));
    ir.locals[L0].needs_default = false;
    ir.values[V0].kind = ValueKind::Alias(V1);
    let mut body = vec![assign(0, 0), returned()];
    prepare(&ir, &mut body);
    assert!(matches!(
        body[0],
        Statement::Simple(Simple::Declare { local: L0, .. })
    ));
}

#[test]
fn default_elision_observes_store_boundaries_before_exceptional_reads() {
    let mut ir = fixture();
    ir.locals[L0].needs_default = true;
    ir.blocks[B0].handlers.push(crate::cfg::HandlerEdge {
        target: B1,
        catch_type: None,
        table_index: 0,
    });
    ir.values.push(Value {
        local: None,
        ty: JvmType::Int,
        kind: ValueKind::Call {
            kind: crate::ir::InvokeKind::Static,
            method: crate::ir::Member {
                owner: "Input".into(),
                name: "read".into(),
                descriptor: "()I".into(),
            },
            receiver: None,
            args: Vec::new(),
        },
        pc: 0,
        block: B0,
    });

    ir.blocks[B0].statements[0] = crate::ir::Statement::Assign {
        local: L0,
        value: ValueUse::Reference(V2),
    };
    let mut body = vec![assign(0, 2), returned()];
    prepare(&ir, &mut body);
    assert!(matches!(
        body[0],
        Statement::Simple(Simple::Declare {
            initialize: true,
            ..
        })
    ));
    ir.blocks[B0].statements = vec![
        crate::ir::Statement::Assign {
            local: L0,
            value: ValueUse::Reference(V0),
        },
        crate::ir::Statement::Eval(V2),
    ];
    let mut body = vec![assign(0, 0), returned()];
    prepare(&ir, &mut body);
    assert!(declared(&body[0]));
}

#[test]
fn for_update_accesses_place_body_assignments_outside_the_header() {
    let mut ir = fixture();
    ir.locals[L0].parameter = Some(0);
    ir.locals.push(Local {
        name_hint: None,
        name: "step".into(),
        ty: JvmType::Int,
        parameter: None,
        is_this: false,
        needs_default: false,
    });
    let mut body = vec![Statement::Loop {
        label: None,
        header: LoopHeader::For {
            initializer: None,
            condition: Expression::Boolean(true),
            update: Box::new(Simple::Scheduled(crate::ir::Statement::Assign {
                local: L1,
                value: ValueUse::Reference(V1),
            })),
        },
        body: vec![assign(1, 0), Statement::Continue(None)],
    }];
    prepare(&ir, &mut body);
    assert!(matches!(
        body[0],
        Statement::Simple(Simple::Declare { local: L1, .. })
    ));
    let Statement::Loop { body, .. } = &body[1] else {
        panic!()
    };
    assert!(!declared(&body[0]));
}

#[test]
fn final_branches_hoist_to_their_common_source_scope_and_drop_erased_bindings() {
    let ir = fixture();
    let mut body = vec![Statement::Block {
        label: None,
        body: vec![
            Statement::If {
                condition: Expression::Boolean(true),
                yes: vec![assign(0, 0)],
                no: vec![assign(0, 0)],
            },
            returned(),
        ],
    }];
    prepare(&ir, &mut body);
    let [Statement::Block { body, .. }] = body.as_slice() else {
        panic!()
    };
    assert!(matches!(
        body[0],
        Statement::Simple(Simple::Declare { local: L0, .. })
    ));
    let mut body = Vec::new();
    prepare(&ir, &mut body);
    assert!(body.is_empty());
}
