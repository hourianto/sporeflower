//! Shared context for planning source from a frozen scheduled graph.
//! Planning can request support members; the final writer sees only bound syntax.
use super::{
    body::CodeWriter,
    boundary::BoundarySupport,
    exceptions,
    expr::{ExprPlanner, Helper},
    rethrow::RethrowHelpers,
    special::SpecialPlan,
    types::TypeNames,
};
use crate::{
    Result,
    ir::{BlockId, LocalId, MethodIr, Terminator},
    program::{MemberId, Program},
    structure::{PreparedIr, Stmt},
};
use std::{cell::RefCell, collections::BTreeSet};

#[derive(Clone, Default)]
struct Support {
    helpers: BTreeSet<Helper>,
    rethrows: Option<RethrowHelpers>,
    boundary: Option<BoundarySupport>,
    class_names: super::class_names::Support,
    diagnostics: BTreeSet<String>,
    relaxed_finals: BTreeSet<MemberId>,
}

pub(super) struct EmissionContext<'a, 'b> {
    pub program: &'a Program<'b>,
    pub types: &'a TypeNames<'a>,
    pub special: SpecialPlan,
    pub mappings: Option<&'a crate::mappings::MappingPlan>,
    pub rename_comments: bool,
    pub semantic_local_names: bool,
    support: RefCell<Support>,
    pub generated: RefCell<super::generated::Members>,
    pub helper_container: Option<String>,
}

pub(super) enum BodyKind {
    /// Source members trim their final void return and honor checked boundaries.
    Method,
    /// Early returns become exits from a labeled initializer block.
    Initializer,
    /// Delegation is emitted first, outside declarations and exception wrappers.
    Constructor {
        delegation: Option<(BlockId, usize)>,
        delegates: bool,
    },
    /// Synthetic methods retain their explicit return and declared exceptions.
    Helper,
}

pub(super) struct BodyOptions<'a> {
    pub kind: BodyKind,
    pub parameters: ParameterOrigin,
    pub indent: usize,
    /// Interface initializer segments share other locals through helper fields.
    pub declared: Option<&'a BTreeSet<LocalId>>,
}

pub(super) enum ParameterOrigin {
    /// Parameter indexes still refer to the original member's descriptor.
    Original,
    /// Extraction replaced parameter indexes with captures or carrier values.
    Captured,
}

impl BodyOptions<'_> {
    pub fn original(kind: BodyKind) -> Self {
        Self {
            parameters: ParameterOrigin::Original,
            kind,
            indent: 2,
            declared: None,
        }
    }

    pub fn captured(kind: BodyKind) -> Self {
        Self {
            parameters: ParameterOrigin::Captured,
            ..Self::original(kind)
        }
    }
}

pub(super) struct RenderedBody {
    pub text: String,
    pub fallback: bool,
    pub initializers: Vec<(usize, super::syntax::Expr)>,
}

/// Successful planning owns every support request and import until publication.
pub(super) struct Artifact<T> {
    pub value: T,
    support: Support,
    generated: super::generated::Members,
    imports: BTreeSet<String>,
}

impl<T> Artifact<T> {
    pub fn needs_helpers(&self) -> bool {
        !self.support.helpers.is_empty() || !self.support.class_names.is_empty()
    }
}

impl<'a, 'b> EmissionContext<'a, 'b> {
    pub fn new(program: &'a Program<'b>, types: &'a TypeNames<'a>, special: SpecialPlan) -> Self {
        Self {
            program,
            types,
            special,
            mappings: None,
            rename_comments: true,
            semantic_local_names: true,
            support: RefCell::new(Support::default()),
            generated: RefCell::new(Default::default()),
            helper_container: None,
        }
    }

    pub fn artifact<T>(
        &self,
        build: impl FnOnce(&EmissionContext<'_, 'b>) -> Result<T>,
    ) -> Result<Artifact<T>> {
        let types = self.types.clone();
        let child = EmissionContext {
            program: self.program,
            types: &types,
            special: self.special.clone(),
            mappings: self.mappings,
            rename_comments: self.rename_comments,
            semantic_local_names: self.semantic_local_names,
            support: RefCell::new(self.support.borrow().clone()),
            generated: RefCell::new(self.generated.borrow().clone()),
            helper_container: self.helper_container.clone(),
        };
        let value = build(&child)?;
        Ok(Artifact {
            value,
            support: child.support.into_inner(),
            generated: child.generated.into_inner(),
            imports: types.requested_imports(),
        })
    }

    pub fn commit<T>(&self, artifact: Artifact<T>) -> T {
        *self.support.borrow_mut() = artifact.support;
        *self.generated.borrow_mut() = artifact.generated;
        self.types.commit_imports(artifact.imports);
        artifact.value
    }

    pub fn register(
        &self,
        member: &crate::ir::Member,
        effects: super::exceptions::Thrown,
    ) -> Result<()> {
        self.generated.borrow_mut().insert(
            member,
            super::generated::Contract {
                signature: crate::classfile::MethodDescriptor::parse(&member.descriptor)?,
                effects,
                container: self.helper_container.clone(),
            },
        );
        Ok(())
    }

    pub fn with_mappings(mut self, mappings: Option<&'a crate::mappings::MappingPlan>) -> Self {
        self.mappings = mappings;
        self
    }

    pub fn stable_local_names(mut self) -> Self {
        self.semantic_local_names = false;
        self
    }

    pub fn expressions<'s>(
        &'s self,
        ir: &'s MethodIr,
        member: Option<MemberId>,
    ) -> ExprPlanner<'s, 'b> {
        ExprPlanner {
            cx: self,
            ir,
            member,
            parameter_member: member,
            booleans: Default::default(),
            semantics: Default::default(),
            shared_semantics: None,
            semantic_diagnostics: Default::default(),
            semantic_names: Default::default(),
        }
    }

    pub fn helper(&self, helper: Helper) -> String {
        self.support.borrow_mut().helpers.insert(helper);
        super::expr::helper_name(self.program, self.types, helper)
    }

    pub fn record_diagnostics(&self, diagnostics: impl IntoIterator<Item = String>) {
        self.support.borrow_mut().diagnostics.extend(diagnostics);
    }

    pub fn diagnostics(&self) -> Vec<String> {
        self.support.borrow().diagnostics.iter().cloned().collect()
    }

    pub fn retains_final(&self, field: MemberId) -> bool {
        self.program.field(field).access_flags & 0x0010 != 0
            && self.program.initialization_plan().permits_final(field)
            && !self.support.borrow().relaxed_finals.contains(&field)
    }

    pub fn relax_final(&self, field: MemberId) {
        self.support.borrow_mut().relaxed_finals.insert(field);
    }

    pub fn rethrow(&self, checked: &[String]) -> String {
        self.support
            .borrow_mut()
            .rethrows
            .get_or_insert_with(|| {
                RethrowHelpers::new(self.program, self.types.names, self.types.owner)
            })
            .request(checked)
    }

    pub fn class_name_helper(&self, originals: &[String]) -> Option<String> {
        self.support
            .borrow_mut()
            .class_names
            .request(self.program, self.types, originals)
    }

    pub fn emit_support(&self, out: &mut CodeWriter) {
        let support = self.support.borrow();
        for helper in &support.helpers {
            out.line("");
            for line in helper
                .source(
                    &super::expr::helper_name(self.program, self.types, *helper),
                    self.types.names,
                )
                .lines()
            {
                out.line(line);
            }
        }
        if let Some(rethrows) = &support.rethrows {
            rethrows.emit(self.types, out);
        }
        if let Some(boundary) = &support.boundary {
            boundary.emit(out);
        }
        support.class_names.emit(out);
    }

    pub fn render(
        &self,
        prepared: &PreparedIr,
        member: Option<MemberId>,
        options: BodyOptions<'_>,
    ) -> Result<RenderedBody> {
        let source = exceptions::structure(self, member, prepared)?;
        let tree = &source;
        let ir = prepared.ir();
        let initializer = matches!(options.kind, BodyKind::Initializer);
        let fallback = tree.statistics.dispatch_regions != 0;
        let mut body = tree.body.as_slice();
        if initializer
            && let Some(Stmt::Return(block)) = body.last()
            && matches!(ir.blocks[*block].terminator, Terminator::Return(None))
        {
            body = &body[..body.len() - 1];
        }
        let mut expr = self.expressions(ir, member);
        if matches!(options.parameters, ParameterOrigin::Captured) {
            expr.parameter_member = None;
        }
        let mut plan = super::source::plan(
            &expr,
            tree,
            body,
            super::source::Options {
                initialize: fallback,
                selected: options.declared,
                skip_constructor: match options.kind {
                    BodyKind::Constructor { delegation, .. } => delegation,
                    _ => None,
                },
                delegates: matches!(
                    options.kind,
                    BodyKind::Constructor {
                        delegates: true,
                        ..
                    }
                ),
                initializes_fields: !matches!(options.kind, BodyKind::Helper),
                initializer,
                returns_void: !matches!(options.kind, BodyKind::Helper | BodyKind::Initializer)
                    && ir.return_type == crate::classfile::JvmType::Void,
            },
        )?;
        if initializer && contains_return(body) {
            let mut writer = CodeWriter::new(0);
            writer.open("$initializer: {");
            writer.body(&plan);
            writer.close("}");
            plan.text = writer.text;
        }
        if matches!(options.kind, BodyKind::Method | BodyKind::Initializer) {
            let member = member.expect("source member");
            self.support
                .borrow_mut()
                .boundary
                .get_or_insert_with(|| {
                    BoundarySupport::new(self.program, self.types.names, self.types.owner)
                })
                .plan(self.program, self.types, member, &mut plan);
        }
        let mut writer = CodeWriter::new(options.indent);
        writer.body(&plan);
        let text = writer.text;
        Ok(RenderedBody {
            text,
            fallback,
            initializers: plan.initializers,
        })
    }
}

fn contains_return(body: &[Stmt]) -> bool {
    crate::structure::tree::walk(body)
        .any(|statement| matches!(statement, Stmt::Return(_) | Stmt::Dispatch { .. }))
}
