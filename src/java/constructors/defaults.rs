//! Complete partially written JVM finals on paths retaining their VM default.
//! The original publication stays in place. Only a still-unassigned edge gets
//! a default store, after super and before it merges with an assigned path.
use super::*;
use crate::ir::{IrBlock, Literal, ValueUse, scheduled};
use std::collections::{HashMap, VecDeque};

pub(super) fn complete(program: &Program<'_>, owner: usize, ir: &mut MethodIr) {
    if ir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| {
            matches!(statement,
        Statement::ConstructorCall { method, .. } if method.owner == program.class(owner).name)
        })
    {
        return;
    }
    let mut fields: HashMap<usize, Vec<(BlockId, Member, ValueId)>> = HashMap::new();
    for (block, body) in ir.blocks.iter_enumerated() {
        for statement in &body.statements {
            let Statement::PutField {
                field,
                receiver: Some(receiver),
                ..
            } = statement
            else {
                continue;
            };
            let Some(id) = program.resolve_field(&field.owner, &field.name, &field.descriptor)
            else {
                continue;
            };
            if id.class == owner
                && program.field(id).access_flags & 0x0018 == 0x0010
                && program.initialization_plan().permits_final(id)
                && is_this(ir, *receiver, &mut HashSet::new())
            {
                fields
                    .entry(id.index)
                    .or_default()
                    .push((block, field.clone(), *receiver));
            }
        }
    }
    let mut fields: Vec<_> = fields
        .into_iter()
        .filter_map(|(id, writes)| {
            (writes.len() == 1).then(|| (id, writes.into_iter().next().unwrap()))
        })
        .collect();
    fields.sort_by_key(|(id, _)| *id);
    for (index, (store, field, receiver)) in fields {
        let id = MemberId {
            class: owner,
            index,
        };
        let Some(plan) = plan(program, ir, store, id) else {
            continue;
        };
        if plan.edges.is_empty() && plan.returns.is_empty() && plan.default_reads.is_empty() {
            continue;
        }
        let ty = JvmType::parse(&field.descriptor).expect("lifted field type");
        let literal = match ty {
            JvmType::Object(_) | JvmType::Array(_) => Literal::Null,
            JvmType::Long => Literal::Long(0),
            JvmType::Float => Literal::Float(0.0),
            JvmType::Double => Literal::Double(0.0),
            _ => Literal::Int(0),
        };
        let value = add_value(ir, ValueKind::Literal(literal), ty, store, None);
        for read in plan.default_reads {
            ir.values[read].kind = ir.values[value].kind.clone();
        }
        let assignment = Statement::PutField {
            pc: None,
            field,
            receiver: Some(receiver),
            value,
        };
        for block in plan.returns {
            ir.blocks[block].statements.push(assignment.clone());
        }
        for (source, target) in plan.edges {
            let bridge = ir.blocks.next_id();
            ir.blocks.push(IrBlock {
                caught_exception: None,
                pc: ir.blocks[target].pc,
                statements: vec![assignment.clone()],
                terminator: Terminator::Goto(target),
                handlers: vec![],
            });
            for value in &mut ir.values {
                if value.block == target
                    && let ValueKind::Phi { inputs, .. } = &mut value.kind
                {
                    for input in inputs {
                        if input.block == source && input.throw_pc.is_none() {
                            input.block = bridge;
                        }
                    }
                }
            }
            match &mut ir.blocks[source].terminator {
                Terminator::Goto(next) => {
                    assert_eq!(*next, target);
                    *next = bridge;
                }
                Terminator::Branch {
                    then_target,
                    else_target,
                    ..
                } => {
                    if *then_target == target {
                        *then_target = bridge;
                    }
                    if *else_target == target {
                        *else_target = bridge;
                    }
                }
                Terminator::Switch { cases, default, .. } => {
                    if *default == target {
                        *default = bridge;
                    }
                    for (_, next) in cases {
                        if *next == target {
                            *next = bridge;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

struct Plan {
    edges: Vec<(BlockId, BlockId)>,
    returns: Vec<BlockId>,
    default_reads: BTreeSet<ValueId>,
}

fn plan(program: &Program<'_>, ir: &MethodIr, store: BlockId, field: MemberId) -> Option<Plan> {
    let reachable = reachable(ir, ir.entry, None);
    if !reachable.contains(&store)
        || reachable
            .iter()
            .any(|&block| !ir.blocks[block].handlers.is_empty())
    {
        return None;
    }
    let mut predecessors: IdVec<BlockId, _> = vec![Vec::new(); ir.blocks.len()].into();
    for &block in &reachable {
        for target in ir.blocks[block].terminator.successors() {
            predecessors[target].push(block);
        }
    }
    let mut indegree: IdVec<BlockId, _> = predecessors.iter().map(Vec::len).collect();
    let mut ready: VecDeque<_> = reachable
        .iter()
        .copied()
        .filter(|&block| indegree[block] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(block) = ready.pop_front() {
        order.push(block);
        for target in ir.blocks[block].terminator.successors() {
            indegree[target] -= 1;
            if indegree[target] == 0 {
                ready.push_back(target);
            }
        }
    }
    if order.len() != reachable.len() {
        return None;
    }
    let reads: IdVec<BlockId, Vec<BTreeSet<_>>> = ir
        .blocks
        .iter()
        .map(|block| {
            block
                .statements
                .iter()
                .map(|statement| {
                    field_reads(
                        program,
                        ir,
                        scheduled::statement_values(ir, statement).operations(),
                        field,
                    )
                })
                .collect()
        })
        .collect();
    let terminator_reads: IdVec<BlockId, _> = ir
        .blocks
        .iter()
        .map(|block| {
            field_reads(
                program,
                ir,
                block.terminator.operands().into_iter().flat_map(|value| {
                    scheduled::values(ir, ValueUse::Reference(value)).operations()
                }),
                field,
            )
        })
        .collect();
    let mut returns: IdVec<BlockId, _> = vec![false; ir.blocks.len()].into();
    for &block in order.iter().rev() {
        returns[block] = matches!(ir.blocks[block].terminator, Terminator::Return(None))
            || reads[block].iter().any(|values| !values.is_empty())
            || !terminator_reads[block].is_empty()
            || ir.blocks[block]
                .terminator
                .successors()
                .into_iter()
                .any(|next| returns[next]);
    }
    let mut assigned: IdVec<BlockId, _> = vec![false; ir.blocks.len()].into();
    let mut initialized: IdVec<BlockId, _> = vec![false; ir.blocks.len()].into();
    let mut plan = Plan {
        edges: vec![],
        returns: vec![],
        default_reads: BTreeSet::new(),
    };
    for block in order {
        let incoming = &predecessors[block];
        let any = incoming.iter().any(|&source| assigned[source]);
        let all = !incoming.is_empty() && incoming.iter().all(|&source| assigned[source]);
        if any && !all && returns[block] {
            for &source in incoming {
                if !assigned[source] {
                    if !initialized[source] {
                        return None;
                    }
                    plan.edges.push((source, block));
                }
            }
        }
        let mut current = any;
        for (index, statement) in ir.blocks[block].statements.iter().enumerate() {
            if !current {
                plan.default_reads
                    .extend(reads[block][index].iter().copied());
            }
            if let Statement::PutField {
                field: target,
                receiver: Some(receiver),
                ..
            } = statement
                && program.resolve_field(&target.owner, &target.name, &target.descriptor)
                    == Some(field)
                && is_this(ir, *receiver, &mut HashSet::new())
            {
                current = true;
            }
        }
        assigned[block] = current;
        if !current {
            plan.default_reads
                .extend(terminator_reads[block].iter().copied());
        }
        initialized[block] = (!incoming.is_empty()
            && incoming.iter().all(|&source| initialized[source]))
            || ir.blocks[block]
                .statements
                .iter()
                .any(|statement| matches!(statement, Statement::ConstructorCall { .. }));
        if matches!(ir.blocks[block].terminator, Terminator::Return(None)) && !assigned[block] {
            if !initialized[block] {
                return None;
            }
            plan.returns.push(block);
        }
    }
    plan.edges.sort_unstable();
    plan.edges.dedup();
    Some(plan)
}

fn field_reads(
    program: &Program<'_>,
    ir: &MethodIr,
    values: impl Iterator<Item = ValueId>,
    field: MemberId,
) -> BTreeSet<ValueId> {
    values.filter(|&value| {
        matches!(&ir.values[value].kind,
            ValueKind::Field { field: target, receiver: Some(receiver) }
                if program.resolve_field(&target.owner, &target.name, &target.descriptor) == Some(field)
                    && is_this(ir, *receiver, &mut HashSet::new()))
    }).collect()
}
