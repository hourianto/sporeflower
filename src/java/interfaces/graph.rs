//! Cut interface initialization at field publication, keeping the computation
//! before each publication as an ordinary typed control-flow graph.
use crate::hash::HashSet;
use crate::index::{BlockId, IdVec, LocalId};
use crate::ir::fragments::Rebinder;
use crate::{
    Result,
    ir::{InitializationSite, IrBlock, MethodIr, Statement, Terminator, ValueId, ValueKind},
    program::{ClassId, Program},
};
use anyhow::{bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Segment {
    pub field: Option<usize>,
    pub entry: BlockId,
    pub blocks: Vec<(BlockId, IrBlock)>,
    initialization_sites: Vec<InitializationSite>,
}

impl Segment {
    pub fn method_ir(
        &self,
        original: &MethodIr,
        locals: &BTreeSet<LocalId>,
    ) -> Result<(MethodIr, BTreeMap<LocalId, LocalId>)> {
        let blocks: crate::hash::HashMap<_, _> = self
            .blocks
            .iter()
            .enumerate()
            .map(|(index, (id, _))| (*id, BlockId::from_index(index)))
            .collect();
        let mut ir = MethodIr {
            entry: blocks[&self.entry],
            values: IdVec::new(),
            blocks: IdVec::new(),
            locals: IdVec::new(),
            parameters: Vec::new(),
            return_type: original.return_type.clone(),
            initialization_sites: self
                .initialization_sites
                .iter()
                .map(|site| {
                    let mut site = site.clone();
                    site.block = blocks[&site.block];
                    site
                })
                .collect(),
        };
        let mut copy = Rebinder::new(original, ir.entry);
        copy.blocks = blocks;
        let mut local_map = BTreeMap::new();
        for &source in locals {
            copy.declare(
                &mut ir,
                source,
                original.locals[source].clone(),
                ValueKind::Phi {
                    inputs: vec![],
                    slot: None,
                },
            );
            local_map.insert(source, copy.local(source));
        }
        for (_, body) in &self.blocks {
            if let Some(caught) = body.caught_exception {
                copy.bind_catch(&mut ir, caught)?;
            }
        }
        for (source, body) in &self.blocks {
            let body = copy.block(&mut ir, *source, body)?;
            ir.blocks.push(body);
        }
        Ok((ir, local_map))
    }

    pub fn direct_value(&self) -> Option<ValueId> {
        let blocks: BTreeMap<_, _> = self.blocks.iter().map(|(id, block)| (*id, block)).collect();
        let mut visited = HashSet::default();
        let mut next = self.entry;
        loop {
            if !visited.insert(next) {
                return None;
            }
            let block = blocks[&next];
            if !block.statements.is_empty() || !block.handlers.is_empty() {
                return None;
            }
            match block.terminator {
                Terminator::Goto(target) => next = target,
                Terminator::Return(value) => return value,
                _ => return None,
            }
        }
    }
}

pub(super) fn segments(
    program: &Program<'_>,
    owner: ClassId,
    ir: &MethodIr,
) -> Result<Vec<Segment>> {
    let class = program.class(owner);
    let mut blocks = ir.blocks.clone();
    let mut initialization_sites = ir.initialization_sites.clone();
    // A publication becomes the typed return of the initializer helper. Its
    // continuation is the entry of the following field's initializer.
    let mut publications: IdVec<BlockId, _> = vec![None; blocks.len()].into();
    for original in ir.blocks.ids() {
        let mut current = original;
        let mut pending = Vec::new();
        for (position, statement) in ir.blocks[original].statements.iter().enumerate() {
            for (site, copied) in ir
                .initialization_sites
                .iter()
                .zip(&mut initialization_sites)
            {
                if site.block == original && site.statement_start == position {
                    copied.block = current;
                    copied.statement_start = pending.len();
                }
            }
            if let Statement::PutField {
                field,
                receiver: None,
                value,
                ..
            } = statement
            {
                if super::super::fields::redundant_constant_store(
                    program, owner, ir, field, None, *value,
                ) {
                    continue;
                }
                if let Some(id) =
                    program.resolve_field(&field.owner, &field.name, &field.descriptor)
                    && id.class == owner
                {
                    ensure!(
                        class.fields[id.index].constant_value.is_none(),
                        "interface constant is overwritten by initializer"
                    );
                    let continuation = blocks.next_id();
                    blocks.push(IrBlock {
                        caught_exception: None,
                        pc: blocks[current].pc,
                        statements: Vec::new(),
                        terminator: blocks[current].terminator.clone(),
                        handlers: blocks[current].handlers.clone(),
                    });
                    publications.push(None);
                    blocks[current].statements = std::mem::take(&mut pending);
                    blocks[current].terminator = Terminator::Return(Some(*value));
                    publications[current] = Some((id.index, continuation));
                    current = continuation;
                    continue;
                }
            }
            pending.push(statement.clone());
        }
        for (site, copied) in ir
            .initialization_sites
            .iter()
            .zip(&mut initialization_sites)
        {
            if site.block == original
                && site.statement_start == ir.blocks[original].statements.len()
            {
                copied.block = current;
                copied.statement_start = pending.len();
            }
        }
        blocks[current].statements = pending;
    }
    let mut result = Vec::new();
    let mut entry = ir.entry;
    let mut assigned = HashSet::default();
    loop {
        let mut visited = BTreeSet::new();
        let mut pending = vec![entry];
        let mut exits = BTreeSet::new();
        let mut returned = false;
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if let Some(publication) = publications[id] {
                exits.insert(publication);
            } else {
                returned |= matches!(blocks[id].terminator, Terminator::Return(_));
                pending.extend(blocks[id].terminator.successors());
            }
            pending.extend(blocks[id].handlers.iter().map(|handler| handler.target));
        }
        let (field, next) = if let Some(&(field, next)) = exits.first() {
            ensure!(
                !returned,
                "interface initializer conditionally omits a field assignment"
            );
            ensure!(
                exits.iter().all(|&(other, _)| other == field),
                "interface field assignment order depends on control flow"
            );
            ensure!(
                exits.iter().all(|&(_, other)| other == next),
                "interface field assignment has distinct continuations"
            );
            ensure!(
                assigned.insert(field),
                "interface field {} is assigned more than once",
                class.fields[field].name
            );
            (Some(field), Some(next))
        } else {
            (None, None)
        };
        let segment = Segment {
            field,
            entry,
            initialization_sites: initialization_sites
                .iter()
                .filter(|site| visited.contains(&site.block))
                .cloned()
                .collect(),
            blocks: visited
                .into_iter()
                .map(|id| (id, blocks[id].clone()))
                .collect(),
        };
        // A final empty return has no effects to preserve with a marker field.
        if field.is_some()
            || segment.blocks.iter().any(|(_, block)| {
                !block.statements.is_empty()
                    || !matches!(
                        block.terminator,
                        Terminator::Goto(_) | Terminator::Return(None)
                    )
            })
        {
            result.push(segment);
        }
        match next {
            Some(next) => entry = next,
            None => break,
        }
        if result.len() > class.fields.len() {
            bail!("interface initializer does not make progress between field assignments");
        }
    }
    Ok(result)
}
