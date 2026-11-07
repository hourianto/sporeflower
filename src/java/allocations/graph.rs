//! An argument region has one entry, one initialization boundary, and a uniform
//! outer exception frame. Fully owned inner handlers remain inside its helper.
use super::*;
use crate::ir::scheduled::{self, ValueUse, statement_may_throw as statement_throws};
use std::ops::Range;

pub(super) struct Shape {
    pub entry: BlockId,
    pub end: BlockId,
    pub allocation_pc: u32,
    pub occurrence: usize,
    pub spans: BTreeMap<BlockId, Range<usize>>,
    pub handlers: BTreeMap<BlockId, Vec<crate::cfg::HandlerEdge>>,
}

pub(super) fn select(
    ir: &MethodIr,
    site: &crate::ir::InitializationSite,
    value: ValueId,
) -> Result<Shape> {
    let end = ir.values[value].block;
    let occurrence = super::occurrence(ir, end, value)
        .context("external allocation has no unique scheduled evaluation")?;
    let mut spans = BTreeMap::new();
    let mut handlers = BTreeMap::new();
    let mut pending = vec![site.block];
    let frame = &ir.blocks[site.block].handlers;
    ensure!(
        ir.blocks[end].handlers == *frame,
        "external allocation {} at {} changes its exception frame before initialization",
        site.class,
        site.pc
    );
    while let Some(block) = pending.pop() {
        if spans.contains_key(&block) {
            continue;
        }
        let start = if block == site.block {
            site.statement_start
        } else {
            0
        };
        let stop = if block == end {
            occurrence
        } else {
            ir.blocks[block].statements.len()
        };
        ensure!(
            start <= stop,
            "allocation argument region has reversed statement boundaries"
        );
        let body = &ir.blocks[block];
        ensure!(
            !body.statements[start..stop]
                .iter()
                .any(|statement| matches!(
                    statement,
                    Statement::Monitor { .. } | Statement::ConstructorCall { .. }
                )),
            "allocation argument region crosses a monitor or receiver initialization"
        );
        let throwing =
            body.statements[start..stop]
                .iter()
                .any(|statement| statement_throws(ir, statement))
                || (block != end
                    && (matches!(body.terminator, Terminator::Throw { .. })
                        || body.terminator.operands().iter().any(|&value| {
                            scheduled::value_may_throw(ir, ValueUse::Reference(value))
                        })));
        if throwing {
            // A helper's catches run before its caller's catches. Thus owned
            // handlers must be an ordered prefix of the unchanged outer frame.
            let split = body
                .handlers
                .len()
                .checked_sub(frame.len())
                .filter(|&split| body.handlers[split..] == frame[..])
                .with_context(|| {
                    format!(
                        "external allocation {} at {} changes its outer argument exception frame",
                        site.class, site.pc
                    )
                })?;
            let inner: Vec<_> = crate::cfg::dispatch::effective(&body.handlers[..split])
                .cloned()
                .collect();
            for handler in &inner {
                ensure!(
                    handler.target != site.block,
                    "allocation argument handler reenters NEW"
                );
                pending.push(handler.target);
            }
            if !inner.is_empty() {
                handlers.insert(block, inner);
            }
        }
        spans.insert(block, start..stop);
        if block != end {
            ensure!(
                !matches!(
                    body.terminator,
                    Terminator::Return(_) | Terminator::Unreachable
                ),
                "external allocation {} at {} can return before initialization",
                site.class,
                site.pc
            );
            pending.extend(body.terminator.successors());
        }
    }
    ensure!(
        spans.contains_key(&end),
        "allocation initialization is not reachable from its NEW"
    );
    let reached = reachable(ir);
    if end != site.block {
        ensure!(
            matches!(ir.blocks[end].statements.get(occurrence), Some(Statement::Assign { value: crate::ir::ValueUse::Definition(root), .. } | Statement::Eval(root)) if *root == value),
            "cross-block allocation lacks an explicit initialization assignment"
        );
        for &source in &reached {
            for target in ir.blocks[source].terminator.successors() {
                // The initialization block's suffix/terminator is outside the
                // extracted prefix. A completed construction may loop to NEW.
                let internal = source != end && spans.contains_key(&source);
                if target == site.block {
                    ensure!(
                        !internal,
                        "allocation argument loop reexecutes NEW before initialization"
                    );
                } else if spans.contains_key(&target) {
                    ensure!(
                        internal,
                        "allocation argument region has another normal entry"
                    );
                }
            }
            for handler in crate::cfg::dispatch::effective(&ir.blocks[source].handlers) {
                ensure!(
                    !spans.contains_key(&handler.target)
                        || handler.target == site.block
                        || (source != end
                            && spans.contains_key(&source)
                            && handlers
                                .get(&source)
                                .is_some_and(|owned| owned.contains(handler))),
                    "allocation argument region has an exceptional entry"
                );
            }
        }
    }
    ensure!(
        frame
            .iter()
            .all(|handler| !spans.contains_key(&handler.target)),
        "allocation argument region contains its external handler"
    );
    let entries: BTreeSet<_> = handlers
        .values()
        .flatten()
        .map(|handler| handler.target)
        .collect();
    for &source in &reached {
        ensure!(
            !ir.blocks[source]
                .terminator
                .successors()
                .iter()
                .any(|target| entries.contains(target)),
            "allocation argument catch entry is also a normal continuation"
        );
    }
    Ok(Shape {
        entry: site.block,
        end,
        allocation_pc: site.pc,
        occurrence,
        spans,
        handlers,
    })
}

/// Immutable region transfer through the initialization boundary. Call arguments
/// are a tail after this sink, outside every owned handler, so their effects can
/// compose with the summary without another graph scan or dataflow solve.
#[derive(Clone)]
pub(super) struct Facts {
    pub inputs: BTreeSet<LocalId>,
    pub written: BTreeSet<LocalId>,
    pub definite: BTreeSet<LocalId>,
}

impl Facts {
    pub fn with_arguments(&self, ir: &MethodIr, arguments: &[ValueId]) -> Self {
        let mut result = self.clone();
        for &value in arguments {
            expression_effects(
                ir,
                ValueUse::Reference(value),
                &mut result.inputs,
                &mut result.definite,
            );
            result
                .written
                .extend(value_assignments(ir, ValueUse::Reference(value)));
        }
        result
    }
}

pub(super) fn facts(ir: &MethodIr, shape: &Shape) -> Facts {
    debug_assert!(!shape.handlers.contains_key(&shape.end));
    let blocks: Vec<_> = shape.spans.keys().copied().collect();
    let indices: HashMap<_, _> = blocks
        .iter()
        .enumerate()
        .map(|(index, &block)| (block, index))
        .collect();
    let mut used = Vec::new();
    let mut defined = Vec::new();
    let mut exceptional_defined = Vec::new();
    let mut written = BTreeSet::new();
    let mut successors = vec![Vec::new(); blocks.len()];
    let mut predecessors = vec![Vec::new(); blocks.len()];
    let caught: BTreeSet<_> = shape
        .handlers
        .values()
        .flatten()
        .map(|handler| handler.target)
        .collect();
    for (index, &block) in blocks.iter().enumerate() {
        let mut reads = BTreeSet::new();
        let mut defs = BTreeSet::new();
        if caught.contains(&block)
            && let Some(local) = ir.blocks[block]
                .caught_exception
                .and_then(|value| ir.values[value].local)
        {
            // A catch parameter is assigned by exceptional entry, not captured
            // from the caller. It remains a real local in the helper.
            defs.insert(local);
            written.insert(local);
        }
        let mut before_throw: Option<BTreeSet<LocalId>> = None;
        for statement in &ir.blocks[block].statements[shape.spans[&block].clone()] {
            if statement_throws(ir, statement) {
                intersect_throw_defs(&mut before_throw, &defs);
            }
            statement_effects(ir, statement, &mut reads, &mut defs);
            written.extend(statement_assignments(ir, statement));
        }
        if block != shape.end {
            let roots = ir.blocks[block].terminator.operands();
            if matches!(ir.blocks[block].terminator, Terminator::Throw { .. })
                || roots
                    .iter()
                    .any(|&value| scheduled::value_may_throw(ir, ValueUse::Reference(value)))
            {
                intersect_throw_defs(&mut before_throw, &defs);
            }
            for value in roots {
                expression_effects(ir, ValueUse::Reference(value), &mut reads, &mut defs);
                written.extend(value_assignments(ir, ValueUse::Reference(value)));
            }
            for target in ir.blocks[block].terminator.successors() {
                let target = indices[&target];
                successors[index].push((target, false));
                predecessors[target].push((index, false));
            }
        }
        for handler in
            crate::cfg::dispatch::effective(shape.handlers.get(&block).map_or(&[], Vec::as_slice))
        {
            let target = indices[&handler.target];
            successors[index].push((target, true));
            predecessors[target].push((index, true));
        }
        used.push(reads);
        exceptional_defined.push(before_throw.unwrap_or_else(|| defs.clone()));
        defined.push(defs);
    }
    let mut live = used.clone();
    loop {
        let mut changed = false;
        for block in (0..blocks.len()).rev() {
            let mut next = used[block].clone();
            for &(target, exceptional) in &successors[block] {
                let defs = if exceptional {
                    &exceptional_defined[block]
                } else {
                    &defined[block]
                };
                next.extend(live[target].difference(defs).copied());
            }
            if next != live[block] {
                live[block] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let entry = indices[&shape.entry];
    let mut incoming = vec![written.clone(); blocks.len()];
    incoming[entry].clear();
    loop {
        let mut changed = false;
        for block in 0..blocks.len() {
            let mut next = if block == entry {
                BTreeSet::new()
            } else {
                written.clone()
            };
            if block != entry {
                for &(source, exceptional) in &predecessors[block] {
                    let defs = if exceptional {
                        &exceptional_defined[source]
                    } else {
                        &defined[source]
                    };
                    let output: BTreeSet<_> = incoming[source].union(defs).copied().collect();
                    next = next.intersection(&output).copied().collect();
                }
            }
            if next != incoming[block] {
                incoming[block] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Facts {
        inputs: live[entry].clone(),
        written,
        definite: incoming[indices[&shape.end]]
            .union(&defined[indices[&shape.end]])
            .copied()
            .collect(),
    }
}

fn intersect_throw_defs(common: &mut Option<BTreeSet<LocalId>>, defined: &BTreeSet<LocalId>) {
    *common = Some(common.as_ref().map_or_else(
        || defined.clone(),
        |common| common.intersection(defined).copied().collect(),
    ));
}

/// Replace the region with its explicit NEW/helper at the original entry.
pub(super) fn place(ir: &mut MethodIr, shape: &Shape, value: ValueId) {
    if shape.entry == shape.end {
        ir.blocks[shape.entry]
            .statements
            .drain(shape.spans[&shape.entry].clone());
        return;
    }
    let initialization = ir.blocks[shape.end].statements[shape.occurrence].clone();
    ir.blocks[shape.end].statements.drain(..=shape.occurrence);
    ir.blocks[shape.entry]
        .statements
        .truncate(shape.spans[&shape.entry].start);
    ir.blocks[shape.entry].statements.push(initialization);
    ir.blocks[shape.entry].terminator = Terminator::Goto(shape.end);
    ir.values[value].block = shape.entry;
    if shape
        .handlers
        .values()
        .flatten()
        .any(|handler| handler.target == shape.end)
    {
        ir.blocks[shape.end].caught_exception = None;
    }
    for &block in shape.spans.keys() {
        if block == shape.entry || block == shape.end {
            continue;
        }
        ir.blocks[block].statements.clear();
        ir.blocks[block].terminator = Terminator::Unreachable;
        ir.blocks[block].handlers.clear();
        ir.blocks[block].caught_exception = None;
    }
}

fn reachable(ir: &MethodIr) -> BTreeSet<BlockId> {
    let mut reached = BTreeSet::new();
    let mut pending = vec![ir.entry];
    while let Some(block) = pending.pop() {
        if !reached.insert(block) {
            continue;
        }
        pending.extend(ir.blocks[block].terminator.successors());
        pending.extend(
            crate::cfg::dispatch::effective(&ir.blocks[block].handlers)
                .map(|handler| handler.target),
        );
    }
    reached
}
