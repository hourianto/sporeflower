//! Converge mutually exclusive publications of one constructor final field.
//!
//! Java's definite-assignment rules lose the JVM's exact exception boundaries
//! when separate final writes appear in sibling catches. Capture each RHS at
//! its original position and publish once at the immediate common join.
use crate::classfile::JvmType;
use crate::index::{BlockId, IdVec, ValueId};
use crate::ir::{
    IrBlock, Local, Member, MethodIr, PhiInput, Statement, Terminator, Value, ValueKind,
};
use crate::program::{MemberId, Program};

pub(super) fn converge(program: &Program<'_>, owner: usize, ir: &mut MethodIr) {
    let count = ir.blocks.len();
    let mut reachable: IdVec<BlockId, _> = vec![false; count].into();
    let mut pending = vec![ir.entry];
    while let Some(block) = pending.pop() {
        if std::mem::replace(&mut reachable[block], true) {
            continue;
        }
        pending.extend(ir.blocks[block].terminator.successors());
        pending.extend(ir.blocks[block].handlers.iter().map(|edge| edge.target));
    }
    let mut predecessors: IdVec<BlockId, _> = vec![vec![]; count].into();
    let mut exceptional_entry: IdVec<BlockId, _> = vec![false; count].into();
    for (source, block) in ir.blocks.iter_enumerated() {
        if !reachable[source] {
            continue;
        }
        for target in block.terminator.successors() {
            predecessors[target].push(source);
        }
        for handler in &block.handlers {
            exceptional_entry[handler.target] = true;
        }
    }
    for join in (0..count).map(BlockId::from_index) {
        let incoming = &predecessors[join];
        if incoming.len() < 2 || exceptional_entry[join] || join == ir.entry {
            continue;
        }
        let Some(stores): Option<Vec<_>> = incoming
            .iter()
            .map(|&block| publication_block(ir, block, join, &predecessors, &exceptional_entry))
            .collect()
        else {
            continue;
        };
        let Some((field_id, field, receiver)) = candidate(program, owner, ir, stores[0]) else {
            continue;
        };
        if !stores.iter().all(|&block| {
            candidate(program, owner, ir, block)
                .is_some_and(|(id, _, other_receiver)| id == field_id && other_receiver == receiver)
        }) || returns_to_predecessor(ir, join, incoming)
        {
            continue;
        }

        let ty = JvmType::parse(&field.descriptor)
            .expect("field descriptor already validated by lifting");
        let local = ir.locals.next_id();
        ir.locals.push(Local {
            name_hint: None,
            name: format!("v{local}"),
            ty: ty.clone(),
            parameter: None,
            is_this: false,
            needs_default: false,
        });
        let mut inputs = Vec::with_capacity(incoming.len());
        let pc = match ir.blocks[stores[0]].statements.last().unwrap() {
            Statement::PutField { pc, .. } => *pc,
            _ => unreachable!(),
        };
        for (&block, &store) in incoming.iter().zip(&stores) {
            let statement = ir.blocks[store].statements.last_mut().unwrap();
            let Statement::PutField { value, .. } = statement else {
                unreachable!()
            };
            inputs.push(PhiInput {
                block,
                throw_pc: None,
                value: *value,
            });
            *statement = Statement::Assign {
                local,
                value: crate::ir::ValueUse::Reference(*value),
            };
        }
        // Publication is outside the predecessors' RHS exception ranges. A
        // join can also begin a new try; keep this nonthrowing write outside
        // that range so Java's catch entry still sees an assigned final.
        let publication = if ir.blocks[join].handlers.is_empty() {
            join
        } else {
            let bridge = ir.blocks.push(IrBlock {
                caught_exception: None,
                pc: ir.blocks[join].pc,
                statements: vec![],
                terminator: Terminator::Goto(join),
                handlers: vec![],
            });
            for &predecessor in incoming {
                ir.blocks[predecessor].terminator.redirect(join, bridge);
            }
            for value in &mut ir.values {
                if value.block == join && matches!(value.kind, ValueKind::Phi { .. }) {
                    value.block = bridge;
                }
            }
            bridge
        };
        let value = ir.values.next_id();
        ir.values.push(Value {
            local: Some(local),
            kind: ValueKind::Phi { inputs, slot: None },
            ty,
            block: publication,
            pc: ir.blocks[join].pc,
        });
        ir.blocks[publication].statements.insert(
            0,
            Statement::PutField {
                pc,
                field,
                receiver: Some(receiver),
                value,
            },
        );
    }
}

fn candidate(
    program: &Program<'_>,
    owner: usize,
    ir: &MethodIr,
    block: BlockId,
) -> Option<(MemberId, Member, ValueId)> {
    let Statement::PutField {
        field,
        receiver: Some(receiver),
        ..
    } = ir.blocks[block].statements.last()?
    else {
        return None;
    };
    // This exact initialized source receiver is non-null, and the field is
    // resolved in its own class. The publication itself has no modeled effect
    // that can throw; only RHS evaluation stays under the predecessor handlers.
    if !ir.is_this(*receiver) {
        return None;
    }
    let id = program.resolve_field(&field.owner, &field.name, &field.descriptor)?;
    let declaration = program.field(id);
    (id.class == owner
        && declaration.access_flags & 0x0018 == 0x0010
        && program.initialization_plan().permits_final(id))
    .then(|| (id, field.clone(), *receiver))
}

/// Exception-range boundaries can leave empty Goto blocks between publication
/// and the join. Walk only a unique normal-entry corridor containing no source
/// operation or catch binding, so field visibility cannot be observed there.
fn publication_block(
    ir: &MethodIr,
    mut block: BlockId,
    mut next: BlockId,
    predecessors: &IdVec<BlockId, Vec<BlockId>>,
    exceptional_entry: &IdVec<BlockId, bool>,
) -> Option<BlockId> {
    for _ in 0..ir.blocks.len() {
        if !matches!(ir.blocks[block].terminator, Terminator::Goto(target) if target == next) {
            return None;
        }
        if !ir.blocks[block].statements.is_empty() {
            return Some(block);
        }
        if exceptional_entry[block] || predecessors[block].len() != 1 {
            return None;
        }
        next = block;
        block = predecessors[block][0];
    }
    None
}

fn returns_to_predecessor(ir: &MethodIr, join: BlockId, predecessors: &[BlockId]) -> bool {
    let mut visited: IdVec<BlockId, _> = vec![false; ir.blocks.len()].into();
    let mut pending = vec![join];
    while let Some(block) = pending.pop() {
        if predecessors.contains(&block) {
            return true;
        }
        if std::mem::replace(&mut visited[block], true) {
            continue;
        }
        pending.extend(ir.blocks[block].terminator.successors());
        pending.extend(
            ir.blocks[block]
                .handlers
                .iter()
                .map(|handler| handler.target),
        );
    }
    false
}
