//! Rebind a selected scheduled fragment into another method.
//!
//! Callers select the fragment and supply its storage and block destinations.
//! A definition copies its operation; a reference reads the destination storage.
//! This module neither selects extraction boundaries nor proves that moving an
//! operation across a constructor, allocation, or publication boundary is legal.
use super::scheduled::ValueUse;
use super::{
    BlockId, IrBlock, Local, LocalId, Member, MethodIr, Statement, Value, ValueId, ValueKind,
};
use crate::Result;
use crate::hash::HashMap;
use anyhow::{Context, ensure};

struct Binding {
    local: LocalId,
    reference: ValueId,
    field: Option<(Member, Option<ValueId>)>,
}

pub(crate) struct Rebinder<'a> {
    source: &'a MethodIr,
    bindings: HashMap<LocalId, Binding>,
    definitions: HashMap<ValueId, ValueId>,
    pub blocks: HashMap<BlockId, BlockId>,
    origin: BlockId,
}

impl<'a> Rebinder<'a> {
    pub fn new(source: &'a MethodIr, origin: BlockId) -> Self {
        Self {
            source,
            bindings: HashMap::default(),
            definitions: HashMap::default(),
            blocks: HashMap::default(),
            origin,
        }
    }

    pub fn bind_local(&mut self, source: LocalId, local: LocalId, reference: ValueId) {
        self.bindings.insert(
            source,
            Binding {
                local,
                reference,
                field: None,
            },
        );
    }

    pub fn declare(
        &mut self,
        target: &mut MethodIr,
        source: LocalId,
        declaration: Local,
        kind: ValueKind,
    ) -> ValueId {
        let ty = declaration.ty.clone();
        let local = target.locals.push(declaration);
        let reference = target.values.push(Value {
            local: Some(local),
            kind,
            ty,
            pc: 0,
            block: self.origin,
        });
        self.bind_local(source, local, reference);
        reference
    }

    /// A transported local still has a parameter/catch binding, but its reads
    /// and committed writes use the carrier field during the copied body.
    pub fn bind_field(&mut self, source: LocalId, field: Member, receiver: Option<ValueId>) {
        self.bindings.get_mut(&source).unwrap().field = Some((field, receiver));
    }

    pub fn local(&self, source: LocalId) -> LocalId {
        self.bindings[&source].local
    }

    pub fn reference(&self, source: LocalId) -> ValueId {
        self.bindings[&source].reference
    }

    pub fn value(&mut self, target: &mut MethodIr, usage: ValueUse) -> Result<ValueId> {
        let source = usage.value();
        if let Some(local) = usage.local(self.source) {
            let binding = self
                .bindings
                .get(&local)
                .context("fragment reads an unbound local")?;
            let Some((field, receiver)) = &binding.field else {
                return Ok(binding.reference);
            };
            let mut value = self.source.values[source].clone();
            value.kind = ValueKind::Field {
                field: field.clone(),
                receiver: *receiver,
            };
            value.local = None;
            value.ty = self.source.locals[local].ty.clone();
            value.block = self.origin;
            return Ok(target.values.push(value));
        }
        if let Some(&value) = self.definitions.get(&source) {
            return Ok(value);
        }
        let mut value = self.source.values[source].clone();
        ensure!(
            !matches!(
                value.kind,
                ValueKind::Phi { .. }
                    | ValueKind::Allocation { .. }
                    | ValueKind::CaughtException { .. }
            ),
            "fragment contains an unscheduled value"
        );
        if let Some((local, rhs)) = value.kind.local_write() {
            let binding = self
                .bindings
                .get(&local)
                .context("fragment writes an unbound local")?;
            ensure!(
                binding.field.is_none(),
                "embedded local write requires local storage"
            );
            let local = binding.local;
            // A postfix result is the old value. Its committed RHS is still a
            // definition, even when that RHS is bound to the updated local.
            let rhs = self.value(target, ValueUse::Definition(rhs))?;
            let (destination, operand) = value.kind.local_write_mut().unwrap();
            *destination = local;
            *operand = rhs;
        } else {
            let mut operands = value
                .kind
                .operands()
                .into_iter()
                .map(|value| self.value(target, ValueUse::Reference(value)))
                .collect::<Result<Vec<_>>>()?
                .into_iter();
            value.kind.map_operands(|_| operands.next().unwrap());
        }
        value.local = value
            .local
            .and_then(|local| self.bindings.get(&local))
            .filter(|binding| binding.field.is_none())
            .map(|binding| binding.local);
        value.block = self
            .blocks
            .get(&value.block)
            .copied()
            .unwrap_or(self.origin);
        let copied = target.values.push(value);
        self.definitions.insert(source, copied);
        Ok(copied)
    }

    fn store(&self, local: LocalId, value: ValueUse) -> Result<Statement> {
        let binding = self
            .bindings
            .get(&local)
            .context("fragment writes an unbound local")?;
        Ok(if let Some((field, receiver)) = &binding.field {
            Statement::PutField {
                pc: None,
                field: field.clone(),
                receiver: *receiver,
                value: value.value(),
            }
        } else {
            Statement::Assign {
                local: binding.local,
                value,
            }
        })
    }

    pub fn statement(&mut self, target: &mut MethodIr, source: &Statement) -> Result<Statement> {
        Ok(match source {
            Statement::Assign { local, value } => {
                let copied = self.value(target, *value)?;
                self.store(*local, value.map_value(|_| copied))?
            }
            Statement::Eval(value) => {
                Statement::Eval(self.value(target, ValueUse::Definition(*value))?)
            }
            _ => {
                let mut operands = source
                    .operands()
                    .into_iter()
                    .map(|value| self.value(target, ValueUse::Reference(value)))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter();
                let mut statement = source.clone();
                statement.map_operands(|_| operands.next().unwrap());
                statement
            }
        })
    }

    /// Bind every selected handler before copying any block that reads it.
    pub fn bind_catch(&mut self, target: &mut MethodIr, source: ValueId) -> Result<()> {
        let mut value = self.source.values[source].clone();
        let ValueKind::CaughtException { handler } = &mut value.kind else {
            anyhow::bail!("fragment handler has no caught value");
        };
        *handler = self.blocks[handler];
        value.block = *handler;
        value.local = value.local.map(|local| self.local(local));
        let copied = target.values.push(value);
        self.definitions.insert(source, copied);
        if let Some(local) = self.source.values[source].local {
            self.bindings.get_mut(&local).unwrap().reference = copied;
        }
        Ok(())
    }

    pub fn block(
        &mut self,
        target: &mut MethodIr,
        source: BlockId,
        body: &IrBlock,
    ) -> Result<IrBlock> {
        self.origin = self.blocks[&source];
        let caught_exception = body.caught_exception.map(|value| self.definitions[&value]);
        let mut statements = Vec::new();
        if let Some(caught) = body.caught_exception
            && let Some(local) = self.source.values[caught].local
            && self.bindings[&local].field.is_some()
        {
            // Catch binding is an implicit write, just like an Assign when
            // its storage has been transported into a carrier field.
            statements.push(self.store(local, ValueUse::Reference(caught_exception.unwrap()))?);
        }
        for statement in &body.statements {
            statements.push(self.statement(target, statement)?);
        }
        let mut terminator = body.terminator.clone();
        let mut operands = terminator
            .operands()
            .into_iter()
            .map(|value| self.value(target, ValueUse::Reference(value)))
            .collect::<Result<Vec<_>>>()?
            .into_iter();
        terminator.map_operands(|_| operands.next().unwrap());
        match &mut terminator {
            super::Terminator::Goto(target) => *target = self.blocks[target],
            super::Terminator::Branch {
                then_target,
                else_target,
                ..
            } => {
                *then_target = self.blocks[then_target];
                *else_target = self.blocks[else_target];
            }
            super::Terminator::Switch { cases, default, .. } => {
                *default = self.blocks[default];
                for (_, target) in cases {
                    *target = self.blocks[target];
                }
            }
            _ => {}
        }
        Ok(IrBlock {
            caught_exception,
            pc: body.pc,
            statements,
            terminator,
            handlers: body
                .handlers
                .iter()
                .map(|handler| {
                    let mut handler = handler.clone();
                    handler.target = self.blocks[&handler.target];
                    handler
                })
                .collect(),
        })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/fragments.rs"]
mod tests;
