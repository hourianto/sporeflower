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
    ir::{LocalId, MethodIr, Terminator},
    program::{MemberId, Program},
    structure::{PreparedIr, Stmt},
};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

#[derive(Clone, Default)]
struct Support {
    helpers: BTreeSet<Helper>,
    rethrows: Option<RethrowHelpers>,
    boundary: Option<BoundarySupport>,
    class_names: super::class_names::Support,
    constructor_failure_markers: BTreeMap<String, String>,
    diagnostics: BTreeSet<String>,
    relaxed_finals: BTreeSet<MemberId>,
    simplifications: crate::ir::SimplificationCounts,
}

pub(super) struct EmissionContext<'a, 'b> {
    pub program: &'a Program<'b>,
    pub types: &'a TypeNames<'a>,
    pub special: SpecialPlan,
    pub options: super::JavaOptions<'a>,
    pub semantic_local_names: bool,
    support: RefCell<Support>,
    pub generated: RefCell<super::generated::Members>,
    pub helper_container: Option<String>,
    /// Java 1.3 forbids static support members inside an enclosing-instance class.
    inner_support: Option<Rc<OnceCell<String>>>,
}

pub(super) enum BodyKind<'a> {
    /// Source members trim their final void return and honor checked boundaries.
    Method,
    /// Early returns become exits from a labeled initializer block.
    Initializer,
    /// Delegation is emitted first, outside declarations and exception wrappers.
    Constructor(super::source::ConstructorEntry<'a>),
    /// Synthetic methods retain their explicit return and declared exceptions.
    Helper,
}

pub(super) struct BodyOptions<'a> {
    pub kind: BodyKind<'a>,
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

impl<'a> BodyOptions<'a> {
    pub fn original(kind: BodyKind<'a>) -> Self {
        Self {
            parameters: ParameterOrigin::Original,
            kind,
            indent: 2,
            declared: None,
        }
    }

    pub fn captured(kind: BodyKind<'a>) -> Self {
        Self {
            parameters: ParameterOrigin::Captured,
            ..Self::original(kind)
        }
    }
}

pub(super) struct RenderedBody {
    pub text: String,
    pub fallback: bool,
    pub initializers: Vec<super::source::bound::Initializer>,
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
    pub fn new(
        program: &'a Program<'b>,
        types: &'a TypeNames<'a>,
        special: SpecialPlan,
        options: super::JavaOptions<'a>,
    ) -> Self {
        Self {
            program,
            types,
            special,
            options,
            semantic_local_names: true,
            support: RefCell::new(Support::default()),
            generated: RefCell::new(Default::default()),
            helper_container: None,
            inner_support: types.units.capture(types.owner).map(|_| Rc::default()),
        }
    }

    pub fn method_ir(
        &self,
        member: MemberId,
    ) -> Result<(MethodIr, crate::ir::SimplificationCounts)> {
        let sites = self
            .program
            .source_initialization(self.options.preserve_bytecode)
            .sites(member);
        self.program
            .decompile_method(member, sites, self.options.preserve_bytecode)
    }

    pub fn passes(&self) -> crate::ir::SourcePasses {
        crate::ir::SourcePasses::for_inspection(self.options.preserve_bytecode)
    }

    pub fn artifact<T>(
        &self,
        build: impl FnOnce(&EmissionContext<'_, 'b>) -> Result<T>,
    ) -> Result<Artifact<T>> {
        self.artifact_with_options(self.options, build)
    }

    pub fn artifact_with_options<T>(
        &self,
        options: super::JavaOptions<'a>,
        build: impl FnOnce(&EmissionContext<'_, 'b>) -> Result<T>,
    ) -> Result<Artifact<T>> {
        let types = self.types.for_artifact();
        let child = EmissionContext {
            program: self.program,
            types: &types,
            special: self.special.clone(),
            options,
            semantic_local_names: self.semantic_local_names,
            support: RefCell::new(self.support.borrow().clone()),
            generated: RefCell::new(self.generated.borrow().clone()),
            helper_container: self.helper_container.clone(),
            inner_support: self.inner_support.clone(),
        };
        let value = build(&child)?;
        Ok(Artifact {
            value,
            support: child.support.into_inner(),
            generated: child.generated.into_inner(),
            imports: types.take_imports(),
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
        kind: crate::ir::InvokeKind,
    ) -> Result<()> {
        let container = match kind {
            crate::ir::InvokeKind::Static => self
                .helper_container
                .as_deref()
                .or_else(|| self.inner_support_name()),
            _ => None,
        };
        self.generated.borrow_mut().insert(
            member,
            super::generated::Contract {
                signature: crate::classfile::MethodDescriptor::parse(&member.descriptor)?,
                effects,
                container: container.map(str::to_owned),
            },
        );
        Ok(())
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
            local_names: Default::default(),
            catches: None,
        }
    }

    pub fn helper(&self, helper: Helper) -> String {
        self.support.borrow_mut().helpers.insert(helper);
        self.support_call(super::expr::helper_name(self.program, self.types, helper))
    }

    pub(super) fn inner_support_name(&self) -> Option<&str> {
        Some(
            self.inner_support
                .as_ref()?
                .get_or_init(|| super::enclosing::support_name(self.program, self.types)),
        )
    }

    pub(super) fn support_call(&self, name: String) -> String {
        match self.inner_support_name() {
            Some(container) => format!("{container}.{name}"),
            None => name,
        }
    }

    pub fn constructor_failure_marker(&self, name: String, diagnostic: &str) -> String {
        self.support
            .borrow_mut()
            .constructor_failure_markers
            .insert(name.clone(), diagnostic.to_owned());
        self.support_call(name)
    }

    pub fn record_diagnostics(&self, diagnostics: impl IntoIterator<Item = String>) {
        self.support.borrow_mut().diagnostics.extend(diagnostics);
    }

    pub fn simplifications(&self) -> crate::ir::SimplificationCounts {
        self.support.borrow().simplifications
    }

    pub fn record_simplifications(&self, counts: crate::ir::SimplificationCounts) {
        self.support.borrow_mut().simplifications += counts;
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

    pub fn rethrow(&self, checked: &[String], bridge: bool) -> String {
        let mut support = self.support.borrow_mut();
        let bridge = bridge.then(|| {
            support
                .boundary
                .get_or_insert_with(|| BoundarySupport::new(self.program, self.types))
                .rethrow()
        });
        let name = support
            .rethrows
            .get_or_insert_with(|| {
                RethrowHelpers::new(self.program, self.types.names, self.types.owner)
            })
            .request(checked, bridge);
        self.support_call(name)
    }

    pub fn class_name_helper(&self, originals: &[String]) -> Option<String> {
        self.support
            .borrow_mut()
            .class_names
            .request(self.program, self.types, originals)
            .map(|name| self.support_call(name))
    }

    pub fn emit_support(&self, types: &TypeNames<'_>, out: &mut CodeWriter) {
        let support = self.support.borrow();
        for (name, diagnostic) in &support.constructor_failure_markers {
            out.line("");
            out.open(format!("private static final class {name} {{"));
            out.open(format!("{name}() {{"));
            out.line(format!("throw new java.lang.Error({diagnostic});"));
            out.close("}");
            out.close("}");
        }
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
            rethrows.emit(types, out);
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
        let catches = super::scopes::CatchScopes::analyze(ir, &tree.body, options.declared);
        let mut expr = self.expressions(ir, member);
        expr.catches = Some(&catches);
        if matches!(options.parameters, ParameterOrigin::Captured) {
            expr.parameter_member = None;
        }
        let returns_void = !matches!(options.kind, BodyKind::Helper | BodyKind::Initializer)
            && ir.return_type == crate::classfile::JvmType::Void;
        let initializes_fields = !matches!(options.kind, BodyKind::Helper);
        let member_body = matches!(options.kind, BodyKind::Method | BodyKind::Initializer);
        let declared_throws = match options.kind {
            BodyKind::Method | BodyKind::Constructor(_) => {
                member.map_or(&[][..], |id| self.program.method_throws(id))
            }
            BodyKind::Helper | BodyKind::Initializer => &[],
        };
        let mut plan = super::source::plan(
            &expr,
            &catches.candidates,
            tree,
            body,
            super::source::Options {
                initialize: fallback,
                selected: options.declared,
                constructor: match options.kind {
                    BodyKind::Constructor(entry) => Some(entry),
                    _ => None,
                },
                initializes_fields,
                initializer,
                returns_void,
                declared_throws,
            },
        )?;
        if initializer && contains_return(body) {
            let mut writer = CodeWriter::new(0);
            writer.open("$initializer: {");
            writer.body(&plan);
            writer.close("}");
            plan.text = writer.text;
        }
        if let Some(member) = member.filter(|_| member_body)
            && let boundary = self.program.throws_plan().boundary(member)
            && !boundary.is_empty()
        {
            self.support
                .borrow_mut()
                .boundary
                .get_or_insert_with(|| BoundarySupport::new(self.program, self.types))
                .plan(
                    self.program,
                    self.types,
                    boundary,
                    &mut plan,
                    self.inner_support_name(),
                );
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
