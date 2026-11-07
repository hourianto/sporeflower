//! Null has no concrete reference type. A physical local whose every definition
//! is null can use a connected storage type without changing its runtime value.
//! This refines types only; the ordinary interference proof still owns merging.
use crate::classfile::JvmType;
use crate::index::IdVec;
use crate::ir::{Literal, LocalId, MethodIr, Statement, ValueKind, ValueUse, local_effects};
use std::collections::{HashSet, VecDeque};

pub(super) fn refine(ir: &mut MethodIr) {
    let mutable_reference = |local: LocalId| {
        let local = &ir.locals[local];
        local.ty.is_reference() && !local.is_this && local.parameter.is_none()
    };
    let mut edges: IdVec<LocalId, _> = vec![Vec::new(); ir.locals.len()].into();
    let mut differs = false;
    for block in &ir.blocks {
        for statement in &block.statements {
            let Statement::Assign { local, value } = *statement else {
                continue;
            };
            let Some(source) = value.local(ir) else {
                continue;
            };
            if local != source
                && mutable_reference(local)
                && mutable_reference(source)
                && super::hints_compatible(ir, Some(local), Some(source))
            {
                differs |= ir.locals[local].ty != ir.locals[source].ty;
                edges[local].push(source);
                edges[source].push(local);
            }
        }
    }
    if !differs {
        return;
    }

    let null = null_locals(ir);
    let mut seen: IdVec<LocalId, _> = vec![false; ir.locals.len()].into();
    for start in ir.locals.ids() {
        if !null[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut pending = vec![start];
        let mut group = Vec::new();
        let mut anchors = HashSet::new();
        while let Some(local) = pending.pop() {
            group.push(local);
            for &neighbor in &edges[local] {
                if null[neighbor] {
                    if !seen[neighbor] {
                        seen[neighbor] = true;
                        pending.push(neighbor);
                    }
                } else {
                    anchors.insert(ir.locals[neighbor].ty.clone());
                }
            }
        }
        // No hierarchy guesses: conflicting concrete types remain distinct.
        if anchors.len() != 1 {
            continue;
        }
        let ty = anchors.into_iter().next().unwrap();
        for local in group {
            if super::hint(ir, local)
                .is_some_and(|hint| JvmType::parse(&hint.descriptor).ok().as_ref() != Some(&ty))
            {
                continue;
            }
            ir.locals[local].ty = ty.clone();
        }
    }
}

#[derive(Default)]
struct Definitions {
    writes: usize,
    invalid: bool,
    seed: bool,
    dependencies: Vec<LocalId>,
}

fn null_locals(ir: &MethodIr) -> IdVec<LocalId, bool> {
    let mut definitions: IdVec<LocalId, _> =
        (ir.locals.ids()).map(|_| Definitions::default()).collect();
    let mut observe = |local: LocalId, value: ValueUse| {
        let definition = &mut definitions[local];
        definition.writes += 1;
        if let Some((dependencies, seed)) = requirements(ir, value, &mut HashSet::new()) {
            definition.dependencies.extend(dependencies);
            definition.seed |= seed;
        } else {
            definition.invalid = true;
        }
    };
    for block in &ir.blocks {
        for statement in &block.statements {
            local_effects::statement(ir, statement, false).assignments(&mut observe);
        }
        local_effects::terminator(ir, &block.terminator, false).assignments(&mut observe);
    }
    for value in ir
        .blocks
        .iter()
        .filter_map(|block| block.caught_exception.as_ref())
    {
        if let Some(local) = ir.values[*value].local {
            // A catch definition supplies an actual exception, and occurs only
            // on exceptional edges even when the handler also has normal entry.
            definitions[local].invalid = true;
        }
    }
    let mut users: IdVec<LocalId, _> = vec![Vec::new(); ir.locals.len()].into();
    for (local, definition) in definitions.iter_enumerated_mut() {
        definition.dependencies.sort_unstable();
        definition.dependencies.dedup();
        for &dependency in &definition.dependencies {
            users[dependency].push(local);
        }
    }
    let mut possible: IdVec<LocalId, _> = definitions
        .iter()
        .zip(&ir.locals)
        .map(|(definition, local)| {
            definition.writes != 0
                && !definition.invalid
                && local.ty.is_reference()
                && !local.is_this
                && local.parameter.is_none()
        })
        .collect();
    remove_unknown_inputs(&mut possible, &users);

    // Null-seeded copy cycles are valid, but a cycle with no concrete incoming
    // definition is not evidence. Reject it and all locals depending on it.
    let mut anchored: IdVec<LocalId, _> = definitions
        .iter()
        .zip(&possible)
        .map(|(definition, possible)| *possible && definition.seed)
        .collect();
    let mut pending: VecDeque<_> = anchored
        .iter_enumerated()
        .filter_map(|(local, &seed)| seed.then_some(local))
        .collect();
    while let Some(local) = pending.pop_front() {
        for &user in &users[local] {
            if possible[user] && !anchored[user] {
                anchored[user] = true;
                pending.push_back(user);
            }
        }
    }
    for (possible, anchored) in possible.iter_mut().zip(anchored) {
        *possible &= anchored;
    }
    remove_unknown_inputs(&mut possible, &users);
    possible
}

fn remove_unknown_inputs(
    possible: &mut IdVec<LocalId, bool>,
    users: &IdVec<LocalId, Vec<LocalId>>,
) {
    let mut pending: VecDeque<_> = possible
        .iter_enumerated()
        .filter_map(|(local, &possible)| (!possible).then_some(local))
        .collect();
    while let Some(local) = pending.pop_front() {
        for &user in &users[local] {
            if possible[user] {
                possible[user] = false;
                pending.push_back(user);
            }
        }
    }
}

fn requirements(
    ir: &MethodIr,
    usage: ValueUse,
    visiting: &mut HashSet<ValueUse>,
) -> Option<(Vec<LocalId>, bool)> {
    if let Some(local) = usage.local(ir) {
        return Some((vec![local], false));
    }
    let value = usage.value();
    if !ir.values[value].ty.is_reference() || !visiting.insert(usage) {
        return None;
    }
    let result = match &ir.values[value].kind {
        ValueKind::Literal(Literal::Null) => Some((vec![], true)),
        ValueKind::Alias(value) | ValueKind::Cast { value, .. } => {
            requirements(ir, ValueUse::Reference(*value), visiting)
        }
        ValueKind::InlineAssign { value, .. } => {
            requirements(ir, ValueUse::Definition(*value), visiting)
        }
        ValueKind::Select {
            then_value,
            else_value,
            ..
        } => {
            match (
                requirements(ir, ValueUse::Reference(*then_value), visiting),
                requirements(ir, ValueUse::Reference(*else_value), visiting),
            ) {
                (Some((mut left, left_seed)), Some((right, right_seed))) => {
                    left.extend(right);
                    Some((left, left_seed || right_seed))
                }
                _ => None,
            }
        }
        _ => None,
    };
    visiting.remove(&usage);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classfile::LocalVariable;
    use crate::index::test_ids::*;
    use crate::ir::{CompareOp, Condition, InvokeKind, IrBlock, Local, Member, Terminator, Value};

    fn fixture(statements: Vec<Statement>) -> MethodIr {
        let object = JvmType::Object("java/lang/Object".into());
        let string = JvmType::Object("java/lang/String".into());
        let kinds = vec![
            ValueKind::Literal(Literal::Null),
            ValueKind::Phi {
                inputs: vec![],
                slot: Some(0),
            },
            ValueKind::Call {
                kind: InvokeKind::Static,
                method: Member {
                    owner: "Probe".into(),
                    name: "text".into(),
                    descriptor: "()Ljava/lang/String;".into(),
                },
                receiver: None,
                args: vec![],
            },
            ValueKind::Phi {
                inputs: vec![],
                slot: Some(2),
            },
            ValueKind::InlineAssign {
                local: L0,
                value: V2,
            },
            ValueKind::InlineAssign {
                local: L0,
                value: V0,
            },
            ValueKind::Select {
                condition: Condition {
                    op: CompareOp::Eq,
                    left: V0,
                    right: V0,
                },
                then_value: V5,
                else_value: V0,
            },
            ValueKind::Cast {
                value: V1,
                ty: string.clone(),
            },
            ValueKind::CaughtException { handler: B0 },
        ];
        {
            let mut ir = MethodIr {
                entry: B0,
                values: kinds
                    .into_iter()
                    .enumerate()
                    .map(|(id, kind)| Value {
                        local: None,
                        kind,
                        ty: if id == 2 || id == 7 {
                            string.clone()
                        } else {
                            object.clone()
                        },
                        pc: 0,
                        block: B0,
                    })
                    .collect(),
                blocks: vec![IrBlock {
                    caught_exception: None,
                    pc: 0,
                    statements,
                    terminator: Terminator::Return(Some(V1)),
                    handlers: vec![],
                }]
                .into(),
                locals: (0..3)
                    .map(|index| Local {
                        name_hint: None,
                        name: format!("v{index}"),
                        ty: if index == 1 {
                            string.clone()
                        } else {
                            object.clone()
                        },
                        parameter: None,
                        is_this: false,
                        needs_default: false,
                    })
                    .collect(),
                parameters: vec![],
                return_type: object,
                initialization_sites: vec![],
            };
            for (entity, metadata) in ir.values.iter_mut().zip(vec![
                None,
                Some(0),
                Some(1),
                Some(2),
                None,
                None,
                None,
                None,
                Some(0),
            ]) {
                entity.local = metadata.map(LocalId::from_index);
            }
            ir
        }
    }

    fn assign(local: usize, value: ValueUse) -> Statement {
        Statement::Assign {
            local: LocalId::from_index(local),
            value,
        }
    }

    #[test]
    fn null_storage_refines_without_changing_values_or_overriding_interference() {
        let ir = fixture(vec![
            assign(0, ValueUse::Reference(V0)),
            assign(1, ValueUse::Definition(V2)),
            assign(1, ValueUse::Reference(V1)),
        ]);
        let original_value_type = ir.values[V1].ty.clone();
        let mut raw = crate::ir::lift::RawIr::new(ir);
        assert_eq!(super::super::run(&mut raw), 0);
        raw.assert_statement_origins();
        let ir = raw.ir;
        assert_eq!(ir.locals[L0].ty, ir.locals[L1].ty);
        assert_eq!(ir.values[V1].ty, original_value_type);
        // The null remains live while local1 receives a non-null/unknown call
        // result. Equal declared types do not make those contents equivalent.
        assert_ne!(ir.values[V1].local, ir.values[V2].local);
    }

    #[test]
    fn conditional_and_terminal_inline_writes_are_part_of_the_null_proof() {
        let mut ir = fixture(vec![
            assign(0, ValueUse::Reference(V0)),
            assign(1, ValueUse::Definition(V2)),
            assign(1, ValueUse::Reference(V1)),
        ]);
        ir.blocks[B0].terminator = Terminator::Return(Some(V6));
        assert!(null_locals(&ir)[L0]);
        // The untaken branch is still a possible definition of physical local0.
        ir.values[V6].kind = ValueKind::Select {
            condition: Condition {
                op: CompareOp::Eq,
                left: V0,
                right: V0,
            },
            then_value: V4,
            else_value: V0,
        };
        assert!(!null_locals(&ir)[L0]);
    }

    #[test]
    fn implicit_catches_parameters_and_mutable_literal_bindings_are_not_null_evidence() {
        let sample = || {
            fixture(vec![
                assign(0, ValueUse::Reference(V0)),
                assign(1, ValueUse::Definition(V2)),
                assign(1, ValueUse::Reference(V1)),
            ])
        };
        let mut caught = sample();
        caught.blocks[B0].caught_exception = Some(V8);
        assert!(!null_locals(&caught)[L0]);
        let mut parameter = sample();
        parameter.locals[L0].parameter = Some(0);
        assert!(!null_locals(&parameter)[L0]);
        let mut reused = sample();
        reused.values[V0].local = Some(L1);
        assert!(!null_locals(&reused)[L0]);
    }

    #[test]
    fn copy_cycles_need_a_real_null_seed_and_no_unknown_external_input() {
        let mut ir = fixture(vec![
            assign(0, ValueUse::Reference(V0)),
            assign(2, ValueUse::Reference(V1)),
            assign(0, ValueUse::Reference(V3)),
            assign(1, ValueUse::Definition(V2)),
            assign(1, ValueUse::Reference(V1)),
        ]);
        assert!(null_locals(&ir)[L0] && null_locals(&ir)[L2]);
        ir.blocks[B0].statements.remove(0);
        assert!(!null_locals(&ir)[L0] && !null_locals(&ir)[L2]);
        ir.blocks[B0]
            .statements
            .insert(0, assign(0, ValueUse::Reference(V0)));
        ir.blocks[B0]
            .statements
            .push(assign(2, ValueUse::Reference(V2)));
        assert!(!null_locals(&ir)[L0] && !null_locals(&ir)[L2]);
    }

    #[test]
    fn concrete_anchor_conflicts_and_debug_declared_types_prevent_refinement() {
        let sample = || {
            fixture(vec![
                assign(0, ValueUse::Reference(V0)),
                assign(1, ValueUse::Definition(V2)),
                assign(1, ValueUse::Reference(V1)),
            ])
        };
        let mut conflict = sample();
        conflict.blocks[B0].statements.extend([
            assign(2, ValueUse::Reference(V2)),
            assign(2, ValueUse::Reference(V1)),
        ]);
        let original = conflict.locals[L0].ty.clone();
        refine(&mut conflict);
        assert_eq!(conflict.locals[L0].ty, original);
        let mut debug = sample();
        debug.locals[L0].name_hint = Some(LocalVariable {
            index: 0,
            start_pc: 0,
            length: 10,
            name: "declaredObject".into(),
            descriptor: "Ljava/lang/Object;".into(),
        });
        refine(&mut debug);
        assert_eq!(debug.locals[L0].ty, original);
    }

    #[test]
    fn a_null_reference_cast_preserves_bottom_contents() {
        let mut ir = fixture(vec![
            assign(0, ValueUse::Reference(V0)),
            assign(2, ValueUse::Reference(V7)),
            assign(1, ValueUse::Definition(V2)),
            assign(1, ValueUse::Reference(V3)),
        ]);
        assert!(null_locals(&ir)[L0] && null_locals(&ir)[L2]);
        refine(&mut ir);
        assert_eq!(ir.locals[L2].ty, ir.locals[L1].ty);
    }
}
