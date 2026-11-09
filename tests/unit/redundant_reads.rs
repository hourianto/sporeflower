use super::*;
use crate::classfile::{ConstantPool, JvmType};
use crate::index::test_ids::*;
use crate::ir::lift::Event;
use crate::ir::{InvokeKind, IrBlock, Member, MethodIr, Statement, Terminator, Value};

fn field(receiver: Option<ValueId>) -> ValueKind {
    ValueKind::Field {
        field: Member {
            owner: "Holder".into(),
            name: "value".into(),
            descriptor: "I".into(),
        },
        receiver,
    }
}

fn fixture(kinds: Vec<ValueKind>, result: Option<ValueId>) -> RawIr {
    let ir = MethodIr {
        entry: B0,
        values: kinds
            .into_iter()
            .enumerate()
            .map(|(pc, kind)| Value {
                kind,
                pc: pc as u32,
                block: B0,
                ty: JvmType::Int,
                local: None,
            })
            .collect(),
        blocks: vec![IrBlock {
            caught_exception: None,
            pc: 0,
            statements: vec![],
            terminator: Terminator::Return(result),
            handlers: vec![],
        }]
        .into(),
        locals: IdVec::new(),
        parameters: vec![],
        return_type: JvmType::Int,
        initialization_sites: vec![],
    };
    let mut raw = RawIr::new(ir);
    raw.events[B0] = raw
        .ir
        .values
        .ids()
        .map(|value| Event {
            pc: raw.ir.values[value].pc,
            kind: EventKind::Define(value),
        })
        .collect();
    raw
}

fn run(raw: &mut RawIr, flags: Option<u16>) {
    let class = ClassFile {
        minor_version: 0,
        major_version: 48,
        access_flags: 0x21,
        name: "Probe".into(),
        super_name: Some("java/lang/Object".into()),
        interfaces: vec![],
        constant_pool: ConstantPool::default(),
        fields: vec![],
        methods: vec![],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    };
    eliminate(raw, &class, Some(&|_| flags));
}

fn retained(raw: &RawIr, value: ValueId) -> bool {
    raw.events
        .iter()
        .flatten()
        .any(|event| matches!(event.kind, EventKind::Define(candidate) if candidate == value))
}

#[test]
fn first_successful_read_stays_and_only_unused_nonvolatile_repeats_disappear() {
    for flags in [None, Some(0x0048), Some(0x0008)] {
        let mut raw = fixture(vec![field(None), field(None), field(None)], Some(V2));
        run(&mut raw, flags);
        assert!(
            retained(&raw, V0),
            "first read must still initialize/resolve"
        );
        assert_eq!(retained(&raw, V1), flags != Some(0x0008));
        assert!(retained(&raw, V2), "the consumed value may have changed");
    }
}

#[test]
fn earlier_read_only_proves_the_same_immutable_receiver() {
    let mut raw = fixture(
        vec![
            ValueKind::Parameter {
                slot: 0,
                index: Some(0),
            },
            ValueKind::Parameter {
                slot: 1,
                index: Some(1),
            },
            field(Some(V0)),
            field(Some(V1)),
            field(Some(V0)),
        ],
        Some(V2),
    );
    run(&mut raw, Some(0));
    assert!(retained(&raw, V3), "second receiver can be null");
    assert!(!retained(&raw, V4));
}

#[test]
fn receiver_effects_and_intervening_writes_survive_discarded_read_removal() {
    let mut raw = fixture(
        vec![
            ValueKind::Call {
                kind: InvokeKind::Static,
                method: Member {
                    owner: "Probe".into(),
                    name: "receiver".into(),
                    descriptor: "()LHolder;".into(),
                },
                receiver: None,
                args: vec![],
            },
            field(Some(V0)),
            field(Some(V0)),
        ],
        None,
    );
    raw.events[B0].insert(
        2,
        Event {
            pc: 1,
            kind: EventKind::Action(Statement::PutField {
                pc: None,
                field: Member {
                    owner: "Holder".into(),
                    name: "value".into(),
                    descriptor: "I".into(),
                },
                receiver: Some(V0),
                value: V1,
            }),
        },
    );
    run(&mut raw, Some(0));
    assert!(retained(&raw, V0));
    assert!(retained(&raw, V1));
    assert!(!retained(&raw, V2));
    assert!(
        raw.events[B0]
            .iter()
            .any(|event| matches!(event.kind, EventKind::Action(_)))
    );
}

#[test]
fn success_in_another_block_is_not_a_dominance_proof() {
    let mut raw = fixture(vec![field(None), field(None)], Some(V0));
    raw.ir.blocks.push(IrBlock {
        caught_exception: None,
        pc: 1,
        statements: vec![],
        terminator: Terminator::Return(None),
        handlers: vec![],
    });
    let event = raw.events[B0].pop().unwrap();
    raw.events.push(vec![event]);
    raw.ir.values[V1].block = B1;
    run(&mut raw, Some(8));
    assert!(retained(&raw, V1));
}
