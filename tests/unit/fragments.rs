use super::*;
use crate::classfile::JvmType;
use crate::index::{IdVec, test_ids::*};
use crate::ir::{BinaryOp, InvokeKind, Literal, Local, Terminator, Value};

fn method() -> MethodIr {
    MethodIr {
        entry: B0,
        values: IdVec::new(),
        blocks: IdVec::new(),
        locals: IdVec::new(),
        parameters: Vec::new(),
        return_type: JvmType::Int,
        initialization_sites: Vec::new(),
    }
}

fn local(ir: &mut MethodIr) -> LocalId {
    ir.locals.push(Local {
        name_hint: None,
        name: format!("v{}", ir.locals.len()),
        ty: JvmType::Int,
        parameter: None,
        is_this: false,
        needs_default: false,
    })
}

fn value(ir: &mut MethodIr, kind: ValueKind, local: Option<LocalId>) -> ValueId {
    ir.values.push(Value {
        kind,
        local,
        ty: JvmType::Int,
        pc: 17,
        block: B0,
    })
}

fn call() -> ValueKind {
    ValueKind::Call {
        kind: InvokeKind::Static,
        method: Member {
            owner: "Input".into(),
            name: "next".into(),
            descriptor: "()I".into(),
        },
        receiver: None,
        args: Vec::new(),
    }
}

fn binding(copy: &mut Rebinder<'_>, target: &mut MethodIr, source: LocalId) -> ValueId {
    let local = local(target);
    let reference = value(
        target,
        ValueKind::Phi {
            inputs: vec![],
            slot: None,
        },
        Some(local),
    );
    copy.bind_local(source, local, reference);
    reference
}

#[test]
fn copied_definition_evaluates_once_while_assignments_to_other_storage_read_it() {
    let mut source = method();
    let first = local(&mut source);
    let second = local(&mut source);
    let producer = value(&mut source, call(), Some(first));
    value(&mut source, call(), None); // An unselected operation must not enter the helper.
    let mut target = method();
    local(&mut target); // Force different storage identities.
    let mut copy = Rebinder::new(&source, B1);
    let reference = binding(&mut copy, &mut target, first);
    binding(&mut copy, &mut target, second);

    let read = copy
        .statement(
            &mut target,
            &Statement::Assign {
                local: second,
                value: ValueUse::Reference(producer),
            },
        )
        .unwrap();
    assert!(
        matches!(read, Statement::Assign { local: L2, value } if value == ValueUse::Reference(reference))
    );
    assert_eq!(target.values.len(), 2);
    let define = copy
        .statement(
            &mut target,
            &Statement::Assign {
                local: first,
                value: ValueUse::Definition(producer),
            },
        )
        .unwrap();
    let Statement::Assign {
        local: L1,
        value: ValueUse::Definition(definition),
    } = define
    else {
        panic!("{define:?}")
    };
    assert_ne!(definition, reference);
    assert!(matches!(
        target.values[definition].kind,
        ValueKind::Call { .. }
    ));
    assert_eq!(target.values[definition].pc, 17);
    assert_eq!(target.values[definition].block, B1);
    assert_eq!(
        copy.value(&mut target, ValueUse::Definition(producer))
            .unwrap(),
        definition
    );
    assert_eq!(
        copy.value(&mut target, ValueUse::Reference(producer))
            .unwrap(),
        reference
    );
    assert_eq!(target.values.len(), 3);

    // A coalesced self-copy remains a storage read during fragment extraction.
    let self_copy = copy
        .statement(
            &mut target,
            &Statement::Assign {
                local: first,
                value: ValueUse::Reference(producer),
            },
        )
        .unwrap();
    assert!(
        matches!(self_copy, Statement::Assign { local: L1, value: ValueUse::Reference(value) } if value == reference)
    );
    assert_eq!(target.values.len(), 3);
}

#[test]
fn postfix_copy_rebinds_its_old_read_and_committed_definition_separately() {
    let mut source = method();
    let stored = local(&mut source);
    let old = value(&mut source, call(), Some(stored));
    let one = value(&mut source, ValueKind::Literal(Literal::Int(1)), None);
    let committed = value(
        &mut source,
        ValueKind::Binary {
            op: BinaryOp::Add,
            left: old,
            right: one,
        },
        Some(stored),
    );
    let update = value(
        &mut source,
        ValueKind::LocalUpdate {
            local: stored,
            value: committed,
            increment: true,
            postfix: true,
        },
        None,
    );
    let mut target = method();
    local(&mut target);
    let mut copy = Rebinder::new(&source, B0);
    let reference = binding(&mut copy, &mut target, stored);
    let copied = copy
        .value(&mut target, ValueUse::Reference(update))
        .unwrap();
    let ValueKind::LocalUpdate {
        local,
        value: rhs,
        postfix: true,
        ..
    } = target.values[copied].kind
    else {
        panic!()
    };
    assert_eq!(local, L1);
    assert_ne!(rhs, reference);
    assert!(matches!(target.values[rhs].kind, ValueKind::Binary { left, .. } if left == reference));
    assert_eq!(target.values[rhs].local, Some(L1));
    assert!(
        !target
            .values
            .iter()
            .any(|value| matches!(value.kind, ValueKind::Call { .. }))
    );
}

#[test]
fn transported_reads_use_the_storage_type_while_definitions_keep_the_producer_type() {
    let mut source = method();
    let stored = local(&mut source);
    source.locals[stored].ty = JvmType::Byte;
    let producer = value(
        &mut source,
        ValueKind::Literal(Literal::Int(5)),
        Some(stored),
    );
    let mut target = method();
    let mut copy = Rebinder::new(&source, B0);
    binding(&mut copy, &mut target, stored);
    copy.bind_field(
        stored,
        Member {
            owner: "State".into(),
            name: "narrow".into(),
            descriptor: "B".into(),
        },
        None,
    );
    let read = copy
        .value(&mut target, ValueUse::Reference(producer))
        .unwrap();
    let definition = copy
        .value(&mut target, ValueUse::Definition(producer))
        .unwrap();
    assert_eq!(target.values[read].ty, JvmType::Byte);
    assert_eq!(target.values[definition].ty, JvmType::Int);
    assert_eq!(target.values[definition].local, None);
}

#[test]
fn caught_binding_and_field_transport_follow_the_copied_handler() {
    let mut source = method();
    let stored = local(&mut source);
    let caught = value(
        &mut source,
        ValueKind::CaughtException { handler: B1 },
        Some(stored),
    );
    source.values[caught].block = B1;
    let mut target = method();
    let mut copy = Rebinder::new(&source, B0);
    binding(&mut copy, &mut target, stored);
    copy.bind_field(
        stored,
        Member {
            owner: "State".into(),
            name: "caught".into(),
            descriptor: "I".into(),
        },
        None,
    );
    copy.blocks.insert(B1, B0);
    copy.bind_catch(&mut target, caught).unwrap();
    let body = copy
        .block(
            &mut target,
            B1,
            &IrBlock {
                caught_exception: Some(caught),
                pc: 17,
                statements: vec![],
                terminator: Terminator::Return(Some(caught)),
                handlers: vec![],
            },
        )
        .unwrap();
    let copied = body.caught_exception.unwrap();
    assert!(matches!(
        target.values[copied].kind,
        ValueKind::CaughtException { handler: B0 }
    ));
    assert!(
        matches!(body.statements.as_slice(), [Statement::PutField { value, .. }] if *value == copied)
    );
    let Terminator::Return(Some(read)) = body.terminator else {
        panic!()
    };
    assert!(matches!(target.values[read].kind, ValueKind::Field { .. }));
    assert_eq!(target.values[read].block, B0);
}
