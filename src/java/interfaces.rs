//! Interface fields are initialized in bytecode assignment order. Each helper
//! returns one field's value, so callbacks observe the same partially initialized
//! interface; a single eager snapshot of every field would change that behavior.
use super::{
    body::CodeWriter,
    context::{BodyKind, BodyOptions, EmissionContext},
    special::SpecialPlan,
};
use crate::index::{BlockId, IdVec, LocalId};
use crate::structure::PreparedIr;
use crate::{
    Result,
    classfile::JvmType,
    ir::{IrBlock, Literal, MethodIr, Terminator, ValueId, ValueKind, ValueUse},
    program::{MemberId, Program},
};
use std::collections::{BTreeSet, HashSet};

pub struct InterfacePlan {
    /// Field indices and expressions, already ordered by actual initialization.
    pub fields: Vec<(usize, String)>,
    /// Extra field needed only when effects follow the last field assignment.
    pub trailing_field: Option<String>,
    /// Indented member class containing the initializer methods.
    pub helper: String,
    pub diagnostics: Vec<String>,
    pub fallback: bool,
}

mod graph;

impl InterfacePlan {
    pub fn build(
        parent: &EmissionContext<'_, '_>,
        linkage: &super::linkage::LinkagePlan,
    ) -> Result<Self> {
        let program = parent.program;
        let types = parent.types;
        let owner = types.owner;
        let mappings = parent.mappings;
        let helper_name = super::generated::symbol(program, types, "$Init");
        let mut cx = EmissionContext::new(program, types, SpecialPlan::default())
            .with_mappings(mappings)
            .stable_local_names();
        cx.helper_container = Some(helper_name.clone());
        let class = program.class(owner);
        let member = class
            .methods
            .iter()
            .position(|method| method.name == "<clinit>")
            .map(|index| MemberId {
                class: owner,
                index,
            });
        let (prepared, helpers) = if let Some(member) = member {
            let prepared = super::prepare::PreparedBody::new(&cx, linkage, member, &[])?;
            (prepared.ir, prepared.helpers)
        } else {
            (
                PreparedIr::new(MethodIr {
                    initialization_sites: vec![],
                    entry: BlockId::from_index(0),
                    values: IdVec::new(),
                    blocks: vec![IrBlock {
                        caught_exception: None,
                        pc: 0,
                        statements: Vec::new(),
                        terminator: Terminator::Return(None),
                        handlers: Vec::new(),
                    }]
                    .into(),
                    locals: IdVec::new(),
                    parameters: Vec::new(),
                    return_type: JvmType::Void,
                }),
                super::allocations::AllocationHelpers::default(),
            )
        };
        let ir = prepared.ir();
        let segments = graph::segments(program, owner, ir)?;
        let assigned: HashSet<_> = segments
            .iter()
            .filter_map(|segment| segment.field)
            .collect();

        let mut uses: IdVec<LocalId, _> = vec![BTreeSet::new(); ir.locals.len()].into();
        let mut segment_locals = Vec::new();
        for (segment_index, segment) in segments.iter().enumerate() {
            let mut locals = BTreeSet::new();
            for (_, block) in &segment.blocks {
                if let Some(caught) = block.caught_exception {
                    locals.extend(ir.values[caught].local);
                }
                for statement in &block.statements {
                    locals.extend(super::local_flow::statement_reads(ir, statement));
                    locals.extend(super::local_flow::statement_assignments(ir, statement));
                }
                for value in block.terminator.operands() {
                    locals.extend(super::local_flow::value_reads(
                        ir,
                        ValueUse::Reference(value),
                    ));
                    locals.extend(super::local_flow::value_assignments(
                        ir,
                        ValueUse::Reference(value),
                    ));
                }
            }
            for &local in &locals {
                uses[local].insert(segment_index);
            }
            segment_locals.push(locals);
        }
        let shared: HashSet<_> = uses
            .iter_enumerated()
            .filter_map(|(id, uses)| (uses.len() > 1).then_some(id))
            .collect();
        let mut out = CodeWriter::new(1);
        out.open(format!("final class {helper_name} {{"));
        for &local in &shared.iter().copied().collect::<BTreeSet<_>>() {
            out.line(format!(
                "private static {} {};",
                types.ty(&ir.locals[local].ty),
                ir.locals[local].name
            ));
        }
        // Segment locals may denote fields shared by several generated helper
        // methods. Their declarations are outside any one expression planner.
        // Direct fields all refer to this same frozen initializer. Keep its
        // lazy semantic analysis across fields, while support requests remain
        // isolated so one field's helper does not force later fields into it.
        let initializer = cx.expressions(ir, member);
        let mut fields = Vec::new();
        let mut trailing_field = None;
        let allocation = if let Some(member) = member {
            helpers.render(&cx, member)?
        } else {
            super::context::RenderedBody {
                text: String::new(),
                fallback: false,
            }
        };
        let mut fallback = allocation.fallback;
        let mut helper_used = !allocation.text.is_empty();
        out.text.push_str(&allocation.text);
        for (index, segment) in segments.into_iter().enumerate() {
            let (ty, method_name) = if let Some(field) = segment.field {
                (
                    JvmType::parse(&class.fields[field].descriptor)?,
                    format!("value{index}"),
                )
            } else {
                (
                    JvmType::Object("java/lang/Object".into()),
                    format!("finish{index}"),
                )
            };
            if let (Some(field), Some(value)) = (segment.field, segment.direct_value())
                && segment_locals[index].is_empty()
                && !compile_time_constant(program, ir, value)
            {
                let direct = EmissionContext::new(program, types, SpecialPlan::default())
                    .with_mappings(mappings)
                    .stable_local_names();
                *direct.generated.borrow_mut() = cx.generated.borrow().clone();
                if let Ok(candidate) =
                    direct.artifact(|probe| initializer.with_context(probe).assigned(value, &ty))
                    && !candidate.needs_helpers()
                {
                    let expression = direct.commit(candidate);
                    cx.record_diagnostics(direct.diagnostics());
                    fields.push((field, expression.render()));
                    continue;
                }
            }
            helper_used = true;
            out.open(format!(
                "private static {} {method_name}() {{",
                types.ty(&ty)
            ));
            let (mut segment_ir, local_map) = segment.method_ir(ir, &segment_locals[index])?;
            segment_ir.return_type = ty.clone();
            if segment.field.is_none() {
                let null = segment_ir.values.next_id();
                segment_ir.values.push(crate::ir::Value {
                    local: None,
                    ty: JvmType::Object("java/lang/Object".into()),
                    kind: ValueKind::Literal(Literal::Null),
                    pc: 0,
                    block: segment_ir.entry,
                });
                for block in &mut segment_ir.blocks {
                    if matches!(block.terminator, Terminator::Return(None)) {
                        block.terminator = Terminator::Return(Some(null));
                    }
                }
            }
            let declared = segment_locals[index]
                .iter()
                .filter(|local| !shared.contains(local))
                .map(|local| local_map[local])
                .collect();
            let body = cx.render(
                &PreparedIr::new(segment_ir),
                member,
                BodyOptions {
                    kind: BodyKind::Helper,
                    indent: 3,
                    declared: Some(&declared),
                },
            )?;
            fallback |= body.fallback;
            out.text.push_str(&body.text);
            out.close("}");
            if let Some(field) = segment.field {
                fields.push((field, format!("{helper_name}.{method_name}()")));
            } else {
                let mut name = "$initialized".to_owned();
                while class.fields.iter().enumerate().any(|(index, _)| {
                    types.names.field_name(MemberId {
                        class: owner,
                        index,
                    }) == name
                }) {
                    name.push('_');
                }
                trailing_field = Some(format!("Object {name} = {helper_name}.{method_name}();"));
            }
        }
        // Nonconstant interface fields with no bytecode assignment retain default
        // values without becoming compile-time constants in the rebuilt class.
        for (index, field) in class.fields.iter().enumerate() {
            if field.constant_value.is_some() || assigned.contains(&index) {
                continue;
            }
            helper_used = true;
            let ty = JvmType::parse(&field.descriptor)?;
            let method_name = format!("default{index}");
            out.line(format!(
                "private static {} {method_name}() {{ return {}; }}",
                types.ty(&ty),
                super::body::default_value(&ty)
            ));
            fields.push((index, format!("{helper_name}.{method_name}()")));
        }
        let mut diagnostics = cx.diagnostics();
        if let Some(diagnostic) = linkage.diagnostic(&cx) {
            diagnostics.push(diagnostic);
        }
        let before_linkage = out.text.len();
        linkage.emit(&cx, &mut out);
        helper_used |= out.text.len() != before_linkage;
        cx.emit_support(&mut out);
        out.close("}");
        Ok(Self {
            diagnostics,
            fallback,
            fields,
            trailing_field,
            helper: if helper_used { out.text } else { String::new() },
        })
    }
}

fn compile_time_constant(program: &Program<'_>, ir: &MethodIr, value: ValueId) -> bool {
    match &ir.values[value].kind {
        ValueKind::Literal(
            Literal::Int(_)
            | Literal::Long(_)
            | Literal::Float(_)
            | Literal::Double(_)
            | Literal::String(_),
        ) => true,
        ValueKind::Unary { value, .. } | ValueKind::Cast { value, .. } => {
            compile_time_constant(program, ir, *value)
        }
        ValueKind::Binary { left, right, .. } => {
            compile_time_constant(program, ir, *left) && compile_time_constant(program, ir, *right)
        }
        ValueKind::Field {
            field,
            receiver: None,
        } => program
            .resolve_field(&field.owner, &field.name, &field.descriptor)
            .is_some_and(|id| program.field(id).constant_value.is_some()),
        _ => false,
    }
}
