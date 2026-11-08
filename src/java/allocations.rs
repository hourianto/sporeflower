//! Preserve an external class's NEW boundary by putting argument-region effects
//! inside a typed Java argument helper. Java evaluates NEW before its arguments.
//! The helper owns only the selected region, never a cached copy of the method.
use super::body::CodeWriter;
use super::context::{BodyKind, BodyOptions, EmissionContext};
use super::local_flow::{
    expression_effects, statement_assignments, statement_effects, statement_reads,
    value_assignments, value_reads,
};
use super::types::TypeNames;
use crate::Result;
use crate::classfile::{JvmType, MethodDescriptor};
use crate::index::{BlockId, IdVec, LocalId};
use crate::ir::fragments::Rebinder;
use crate::ir::scheduled::{ValueUse, for_each_statement_root};
use crate::ir::{
    InvokeKind, IrBlock, Local, Member, MethodIr, Statement, Terminator, Value, ValueId, ValueKind,
};
use crate::program::{MemberId, Program};
use crate::structure::PreparedIr;
use anyhow::{Context, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

mod graph;
mod state;

struct LoweringContext<'a, 'b> {
    cx: &'a EmissionContext<'a, 'b>,
    member: MemberId,
}

/// Lowered helpers are rendered only after every caller graph rewrite finishes.
#[derive(Default)]
pub(super) struct AllocationHelpers(Vec<AllocationHelper>);

struct AllocationHelper {
    header: String,
    ir: PreparedIr,
    carrier: String,
}

impl AllocationHelpers {
    pub fn render(
        self,
        cx: &EmissionContext<'_, '_>,
        member: MemberId,
    ) -> Result<super::context::RenderedBody> {
        let mut out = CodeWriter::new(if cx.helper_container.is_some() { 2 } else { 1 });
        let mut fallback = false;
        for helper in self.0 {
            if !out.text.is_empty() {
                out.line("");
            }
            out.open(helper.header);
            let mut options = BodyOptions::captured(BodyKind::Helper);
            options.indent = if cx.helper_container.is_some() { 3 } else { 2 };
            let body = cx.render(&helper.ir, Some(member), options)?;
            fallback |= body.fallback;
            out.text.push_str(&body.text);
            out.close("}");
            out.text.push_str(&helper.carrier);
        }
        Ok(super::context::RenderedBody {
            text: out.text,
            fallback,
        })
    }
}

struct Selection {
    site: crate::ir::InitializationSite,
    value: ValueId,
    arguments: Vec<ValueId>,
    parameters: Vec<JvmType>,
    end: usize,
    shape: graph::Shape,
    inputs: BTreeSet<LocalId>,
    written: BTreeSet<LocalId>,
    definite: BTreeSet<LocalId>,
    exports: BTreeSet<LocalId>,
}

pub(super) fn lower(
    cx: &EmissionContext<'_, '_>,
    member: MemberId,
    ir: &mut MethodIr,
) -> Result<AllocationHelpers> {
    let program = cx.program;
    let types = cx.types;
    let unresolved = program.initialization_plan().unresolved(member);
    if unresolved.is_empty() {
        return Ok(AllocationHelpers::default());
    }
    let mut sites: Vec<_> = ir
        .initialization_sites
        .iter()
        .filter(|site| {
            unresolved
                .iter()
                .any(|planned| planned.pc == site.pc && planned.class == site.class)
        })
        .cloned()
        .collect();
    // Removing a later region cannot change an earlier region's statement start.
    sites.sort_by_key(|site| std::cmp::Reverse((site.block, site.statement_start, site.pc)));
    let mut helpers = AllocationHelpers::default();
    let mut context = LoweringContext { cx, member };
    let mut handler_prefixes: BTreeMap<BlockId, Vec<Statement>> = BTreeMap::new();
    let mut sites: VecDeque<_> = sites.into();
    let mut deferred = 0;
    while let Some(site) = sites.pop_front() {
        let (value, arguments, return_type) = ir
            .values
            .iter_enumerated()
            .find_map(|(id, value)| {
                if let ValueKind::New {
                    class,
                    allocation_pc,
                    descriptor,
                    args,
                } = &value.kind
                    && *allocation_pc == site.pc
                    && *class == site.class
                    && occurrence(ir, value.block, id).is_some()
                {
                    return Some((id, args.clone(), MethodDescriptor::parse(descriptor)));
                }
                None
            })
            .with_context(|| {
                format!(
                    "external NEW at {} has no initialization expression",
                    site.pc
                )
            })?;
        let signature = return_type?;
        ensure!(
            !arguments.is_empty() && arguments.len() == signature.parameters.len(),
            "external allocation {} at {} has no argument in which to evaluate its region",
            site.class,
            site.pc
        );
        let mut shape = graph::select(ir, &site, value)?;
        // Handler bytecode need not follow its protected code physically. Use
        // actual containment to lower nested allocations before their owner.
        if sites.iter().any(|other| {
            shape.spans.contains_key(&other.block)
                && (other.block != shape.entry || other.pc > site.pc)
                && (other.block != shape.end || other.pc < ir.values[value].pc)
        }) {
            sites.push_back(site);
            deferred += 1;
            ensure!(
                deferred < sites.len(),
                "external allocation argument regions have interleaved initialization boundaries"
            );
            continue;
        }
        deferred = 0;
        let owned: BTreeSet<_> = shape
            .handlers
            .values()
            .flatten()
            .map(|handler| handler.target)
            .collect();
        for handler in owned {
            if let Some(mut prefix) = handler_prefixes.remove(&handler) {
                let added = prefix.len();
                prefix.append(&mut ir.blocks[handler].statements);
                ir.blocks[handler].statements = prefix;
                shape.spans.get_mut(&handler).unwrap().end += added;
                if handler == shape.end {
                    shape.occurrence += added;
                }
                for pending in sites.iter_mut().chain(ir.initialization_sites.iter_mut()) {
                    if pending.block == handler {
                        pending.statement_start += added;
                    }
                }
            }
        }
        let end = shape.occurrence;
        let start = site.statement_start;
        ensure!(
            shape.entry != shape.end || start < end,
            "external allocation {} at {} needs an expression-internal argument region",
            site.class,
            site.pc
        );
        // A literal or unchanged local needs no transport. Put the region in
        // its one dependent/effectful argument when the other arguments can
        // remain at the call site; use a carrier only for genuinely shared work.
        let region_facts = graph::facts(ir, &shape);
        let dependent: Vec<_> = arguments
            .iter()
            .enumerate()
            .filter_map(|(index, &argument)| {
                (!stable_argument(ir, argument)
                    || !value_reads(ir, ValueUse::Reference(argument))
                        .is_disjoint(&region_facts.written))
                .then_some(index)
            })
            .collect();
        let pivot = (dependent.len() <= 1).then(|| dependent.first().copied().unwrap_or(0));
        let mut facts = region_facts.with_arguments(
            ir,
            pivot.map_or(arguments.as_slice(), |index| {
                std::slice::from_ref(&arguments[index])
            }),
        );
        let mut outside = outside_reads(ir, &shape, value);
        if let Some(pivot) = pivot {
            for (index, &argument) in arguments.iter().enumerate() {
                if index != pivot {
                    outside.extend(value_reads(ir, ValueUse::Reference(argument)));
                }
            }
        }
        for prefix in handler_prefixes.values() {
            for statement in prefix {
                outside.extend(statement_reads(ir, statement));
            }
        }
        let exports: BTreeSet<_> = facts.written.intersection(&outside).copied().collect();
        if pivot.is_none() || !exports.is_empty() {
            if pivot.is_some() {
                facts = region_facts.with_arguments(ir, &arguments);
            }
            let exports = facts.written.intersection(&outside).copied().collect();
            let selection = Selection {
                site: site.clone(),
                value,
                arguments,
                parameters: signature.parameters,
                end,
                shape,
                inputs: facts.inputs,
                written: facts.written,
                definite: facts.definite,
                exports,
            };
            helpers.0.push(state::lower(
                &mut context,
                ir,
                selection,
                &mut handler_prefixes,
            )?);
            ir.initialization_sites
                .retain(|pending| pending.class != site.class || pending.pc != site.pc);
            continue;
        }
        let pivot = pivot.unwrap();
        let argument = arguments[pivot];
        let return_type = signature.parameters[pivot].clone();
        let all_locals: BTreeSet<_> = facts.inputs.union(&facts.written).copied().collect();
        let name = helper_name(program, types, member, site.pc);
        let mut region = Region::new(ir, &facts.inputs, &all_locals, return_type.clone())?;
        region.prepare_shape(&shape);
        region.clone_blocks(&shape)?;
        let returned = region.value(ValueUse::Reference(argument))?;
        region.ir.blocks[region.copy.blocks[&shape.end]].terminator =
            Terminator::Return(Some(returned));
        let (helper, call) = region.finish(&context, name, String::new(), None)?;
        helpers.0.push(helper);
        let call = ir.values.push(Value {
            local: None,
            kind: call,
            ty: return_type,
            pc: ir.values[value].pc,
            block: site.block,
        });
        let ValueKind::New { args, .. } = &mut ir.values[value].kind else {
            unreachable!()
        };
        args[pivot] = call;
        graph::place(ir, &shape, value);
        ir.initialization_sites
            .retain(|pending| pending.class != site.class || pending.pc != site.pc);
    }
    for (handler, mut prefix) in handler_prefixes {
        prefix.append(&mut ir.blocks[handler].statements);
        ir.blocks[handler].statements = prefix;
    }
    Ok(helpers)
}

/// Expressions evaluated before the selected argument must neither throw nor
/// change state. Materialized values are reads here, regardless of their origin.
fn stable_argument(ir: &MethodIr, value: ValueId) -> bool {
    if ir.values[value].local.is_some() {
        return true;
    }
    let kind = &ir.values[value].kind;
    !kind.is_observable()
        && kind
            .operands()
            .into_iter()
            .all(|operand| stable_argument(ir, operand))
}

/// Find the statement containing the actual NEW evaluation, stopping at locals.
fn occurrence(ir: &MethodIr, block: BlockId, sought: ValueId) -> Option<usize> {
    fn count(ir: &MethodIr, usage: ValueUse, sought: ValueId) -> usize {
        if usage.local(ir).is_some() {
            return 0;
        }
        if usage.value() == sought {
            return 1;
        }
        let mut total = 0;
        usage.for_each_child(ir, |child| total += count(ir, child, sought));
        total
    }
    let mut location = None;
    for (index, statement) in ir.blocks[block].statements.iter().enumerate() {
        let mut occurrences = 0;
        for_each_statement_root(statement, |root| occurrences += count(ir, root, sought));
        if occurrences != 0 {
            if occurrences != 1 || location.is_some() {
                return None;
            }
            location = Some(index);
        }
    }
    let count: usize = ir.blocks[block]
        .terminator
        .operands()
        .into_iter()
        .map(|value| count(ir, ValueUse::Reference(value), sought))
        .sum();
    if count != 0 {
        if count != 1 || location.is_some() {
            return None;
        }
        location = Some(ir.blocks[block].statements.len());
    }
    location
}

fn outside_reads(ir: &MethodIr, shape: &graph::Shape, moved_new: ValueId) -> BTreeSet<LocalId> {
    fn visit(
        ir: &MethodIr,
        usage: ValueUse,
        moved_new: ValueId,
        reads: &mut BTreeSet<LocalId>,
        seen: &mut HashSet<ValueUse>,
    ) {
        if let Some(local) = usage.local(ir) {
            reads.insert(local);
            return;
        }
        if usage.value() == moved_new || !seen.insert(usage) {
            return;
        }
        usage.for_each_child(ir, |child| visit(ir, child, moved_new, reads, seen));
    }
    let mut reads = BTreeSet::new();
    let mut seen = HashSet::new();
    for (block, body) in ir.blocks.iter_enumerated() {
        for (index, statement) in body.statements.iter().enumerate() {
            if shape
                .spans
                .get(&block)
                .is_some_and(|span| span.contains(&index))
            {
                continue;
            }
            for_each_statement_root(statement, |root| {
                visit(ir, root, moved_new, &mut reads, &mut seen)
            });
        }
        if block != shape.end && shape.spans.contains_key(&block) {
            continue;
        }
        for value in body.terminator.operands() {
            visit(
                ir,
                ValueUse::Reference(value),
                moved_new,
                &mut reads,
                &mut seen,
            );
        }
    }
    reads
}

/// Compact extraction: only live-in/defined locals and evaluated value nodes
/// enter a helper. Materialized source values become references to helper locals.
struct Region<'a> {
    original: &'a MethodIr,
    ir: MethodIr,
    copy: Rebinder<'a>,
    captured: Vec<LocalId>,
    instance: bool,
    boundary: Option<(BlockId, u32)>,
}

impl<'a> Region<'a> {
    fn new(
        original: &'a MethodIr,
        inputs: &BTreeSet<LocalId>,
        locals: &BTreeSet<LocalId>,
        return_type: JvmType,
    ) -> Result<Self> {
        let instance = inputs.iter().any(|&local| original.locals[local].is_this);
        let mut result = Self {
            original,
            ir: MethodIr {
                entry: BlockId::from_index(0),
                values: IdVec::new(),
                blocks: IdVec::new(),
                locals: IdVec::new(),
                parameters: vec![],
                return_type,
                initialization_sites: vec![],
            },
            copy: Rebinder::new(original, BlockId::from_index(0)),
            captured: vec![],
            instance,
            boundary: None,
        };
        let mut slot = if instance { 1 } else { 0 };
        for &source in locals {
            let original = &original.locals[source];
            let is_this = original.is_this && inputs.contains(&source);
            let parameter = inputs.contains(&source) && !is_this;
            let index = parameter.then_some(result.captured.len());
            let name = if is_this {
                "this".into()
            } else if parameter {
                format!("p{slot}")
            } else {
                format!("v{}", result.ir.locals.len())
            };
            let declaration = Local {
                name_hint: None,
                name,
                ty: original.ty.clone(),
                parameter: index,
                is_this,
                needs_default: false,
            };
            let kind = if is_this {
                ValueKind::Parameter {
                    slot: 0,
                    index: None,
                }
            } else if parameter {
                ValueKind::Parameter { slot, index }
            } else {
                ValueKind::Phi {
                    inputs: vec![],
                    slot: None,
                }
            };
            let value = result
                .copy
                .declare(&mut result.ir, source, declaration, kind);
            if is_this || parameter {
                result.ir.parameters.push(value);
            }
            if parameter {
                result.captured.push(source);
                slot += original.ty.slots();
            }
        }
        ensure!(
            slot <= 255,
            "allocation helper requires more than 255 parameter slots"
        );
        Ok(result)
    }

    /// The extracted graph owns its signature. Both direct argument helpers and
    /// state helpers derive their declaration and call from those same bindings.
    fn finish(
        self,
        context: &LoweringContext<'_, '_>,
        name: String,
        carrier: String,
        state: Option<ValueId>,
    ) -> Result<(AllocationHelper, ValueKind)> {
        let cx = context.cx;
        let parameters: Vec<_> = self
            .ir
            .parameters
            .iter()
            .filter_map(|&value| {
                let local = &self.ir.locals[self.ir.values[value].local?];
                (!local.is_this).then_some(local)
            })
            .collect();
        let method = Member {
            owner: cx.program.class(context.member.class).name.clone(),
            name,
            descriptor: MethodDescriptor {
                parameters: parameters.iter().map(|local| local.ty.clone()).collect(),
                return_type: self.ir.return_type.clone(),
            }
            .descriptor(),
        };
        let effects = helper_effects(context, &self.ir);
        let throws = super::generated::throws_clause(&effects.source_types, cx.types);
        cx.register(&method, effects)?;
        let header = format!(
            "private {}{} {}({}){throws} {{",
            if self.instance { "" } else { "static " },
            cx.types.ty(&self.ir.return_type),
            method.name,
            parameters
                .iter()
                .map(|local| format!("{} {}", cx.types.ty(&local.ty), local.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut args = self
            .captured
            .iter()
            .map(|&local| {
                local_value(self.original, local)
                    .context("allocation helper input has no local value identity")
            })
            .collect::<Result<Vec<_>>>()?;
        args.extend(state);
        ensure!(
            args.len() == parameters.len(),
            "allocation helper arguments do not match its parameters"
        );
        let receiver = if self.instance {
            Some(
                self.original
                    .parameters
                    .iter()
                    .copied()
                    .find(|&value| self.original.is_this(value))
                    .context("instance allocation helper has no receiver")?,
            )
        } else {
            None
        };
        Ok((
            AllocationHelper {
                header,
                ir: PreparedIr::new(self.ir),
                carrier,
            },
            ValueKind::Call {
                kind: if self.instance {
                    InvokeKind::Virtual
                } else {
                    InvokeKind::Static
                },
                method,
                receiver,
                args,
            },
        ))
    }

    fn validate(&self, usage: ValueUse) -> Result<()> {
        if let Some((block, pc)) = self.boundary {
            for value in crate::ir::scheduled::values(self.original, usage).operations() {
                let value = &self.original.values[value];
                ensure!(
                    !(value.block == block && value.pc < pc && value.may_throw()),
                    "argument region has an observable input evaluated before NEW"
                );
            }
        }
        Ok(())
    }

    fn value(&mut self, usage: ValueUse) -> Result<ValueId> {
        self.validate(usage)?;
        self.copy.value(&mut self.ir, usage)
    }

    fn prepare_shape(&mut self, shape: &graph::Shape) {
        self.copy.blocks = shape
            .spans
            .keys()
            .enumerate()
            .map(|(index, &block)| (block, BlockId::from_index(index)))
            .collect();
        self.ir.entry = self.copy.blocks[&shape.entry];
        self.boundary = Some((shape.entry, shape.allocation_pc));
    }

    fn clone_blocks(&mut self, shape: &graph::Shape) -> Result<()> {
        let caught: BTreeSet<_> = shape
            .handlers
            .values()
            .flatten()
            .map(|handler| handler.target)
            .collect();
        for &source in &caught {
            let original = self.original.blocks[source]
                .caught_exception
                .context("owned argument handler has no caught value")?;
            self.copy.bind_catch(&mut self.ir, original)?;
        }
        for (&source, span) in &shape.spans {
            let body = IrBlock {
                caught_exception: caught
                    .contains(&source)
                    .then_some(self.original.blocks[source].caught_exception)
                    .flatten(),
                pc: self.original.blocks[source].pc,
                statements: self.original.blocks[source].statements[span.clone()].to_vec(),
                terminator: if source == shape.end {
                    Terminator::Return(None)
                } else {
                    self.original.blocks[source].terminator.clone()
                },
                handlers: shape.handlers.get(&source).cloned().unwrap_or_default(),
            };
            for statement in &body.statements {
                let mut roots = Vec::new();
                for_each_statement_root(statement, |root| roots.push(root));
                for root in roots {
                    self.validate(root)?;
                }
            }
            for value in body.terminator.operands() {
                self.validate(ValueUse::Reference(value))?;
            }
            let copied = self.copy.block(&mut self.ir, source, &body)?;
            self.ir.blocks.push(copied);
        }
        Ok(())
    }
}

fn local_value(ir: &MethodIr, local: LocalId) -> Option<ValueId> {
    ir.values
        .iter_enumerated()
        .find_map(|(id, value)| (value.local == Some(local)).then_some(id))
}

fn helper_name(program: &Program<'_>, types: &TypeNames<'_>, member: MemberId, pc: u32) -> String {
    super::generated::symbol(
        program,
        types,
        &format!("$sf$argument_{}_{}", member.index, pc),
    )
}

fn helper_effects(context: &LoweringContext<'_, '_>, ir: &MethodIr) -> super::exceptions::Thrown {
    let mut result = super::exceptions::Thrown::default();
    for (id, block) in ir.blocks.iter_enumerated() {
        if matches!(block.terminator, Terminator::Unreachable) {
            continue;
        }
        let mut effects = super::exceptions::effects(
            context.cx,
            Some(context.member),
            ir,
            &[crate::structure::Stmt::Basic(id)],
        );
        super::exceptions::add_terminator(context.cx, Some(context.member), ir, id, &mut effects);
        for handler in &block.handlers {
            effects.remove_caught(context.cx.program, handler.catch_type.as_deref());
        }
        result.extend(effects);
    }
    result
        .source_types
        .retain(|ty| crate::throws::is_checked(context.cx.program, ty));
    // The generated declaration is owned here, even when an original call's
    // runtime effects remain unknown. Only its explicit throws clause is visible
    // to Java callers of this helper.
    result.source_unknown = false;
    result
}

#[cfg(test)]
#[path = "../../tests/unit/java_generated.rs"]
mod tests;
