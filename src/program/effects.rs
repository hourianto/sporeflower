//! Original class and member facts used to refine JVM exception behavior.
use super::{ClassId, MemberId, Program};
use crate::classfile::{ABSTRACT, INTERFACE, STATIC, package};
use crate::hash::HashMap;
use crate::index::{BlockId, IdVec};
use crate::ir::{InvokeKind, Member, MethodIr, Statement, UpdateTarget, ValueKind, scheduled};

#[derive(Clone, Copy)]
pub(crate) struct MethodFacts<'a> {
    program: &'a Program<'a>,
    method: MemberId,
}

/// Class initialization facts for one immutable scheduled method.
#[derive(Default)]
pub(crate) struct FieldResolution {
    owner_ready: std::cell::OnceCell<bool>,
    activated: std::cell::RefCell<HashMap<ClassId, IdVec<BlockId, usize>>>,
}

impl Program<'_> {
    pub(crate) fn method_facts(&self, method: MemberId) -> MethodFacts<'_> {
        MethodFacts {
            program: self,
            method,
        }
    }
}

impl MethodFacts<'_> {
    pub(crate) fn can_use_platform_contract(self, name: &str) -> bool {
        self.program.can_use_platform_contract(name)
    }

    pub(crate) fn complete_hierarchy(self, name: &str) -> bool {
        self.program
            .id(name)
            .is_some_and(|id| self.program.complete_hierarchy(id))
    }

    pub(crate) fn is_subtype(self, child: &str, parent: &str) -> bool {
        self.program.is_subtype(child, parent)
    }

    pub(crate) fn own_declared_static(self, field: &Member) -> bool {
        self.declared_static(self.method.class, field)
    }

    fn declared_static(self, owner: ClassId, field: &Member) -> bool {
        field.owner == self.program.class(owner).name
            && self
                .program
                .declared_field(owner, &field.name, &field.descriptor)
                .is_some_and(|id| self.program.field(id).access_flags & STATIC != 0)
    }

    /// Can this resolved field be accessed once its receiver is known non-null?
    /// A static access also requires evidence of successful class initialization.
    pub(crate) fn field_is_ready(
        self,
        ir: &MethodIr,
        access: &crate::ir::failures::FieldAccess<'_>,
        (block, index): (BlockId, usize),
        cache: &FieldResolution,
    ) -> bool {
        let program = self.program;
        let owner = self.method.class;
        let field = access.field;
        let Some(class) = program.id(&field.owner) else {
            return false;
        };
        if access.receiver.is_none() {
            let initialized = class == owner
                && *cache
                    .owner_ready
                    .get_or_init(|| self.initialized_on_entry());
            if !initialized
                && index
                    < cache
                        .activated
                        .borrow_mut()
                        .entry(class)
                        .or_insert_with(|| self.activated_after(ir, class))[block]
            {
                return false;
            }
        } else if package(&field.owner) != package(&program.class(owner).name) {
            // Within a package, protected access cannot impose another receiver constraint.
            return false;
        }
        let Some(member) = program.declared_field(class, &field.name, &field.descriptor) else {
            return false;
        };
        let flags = program.field(member).access_flags;
        (flags & STATIC != 0) == access.receiver.is_none()
            && program.member_accessible_from(flags, class, owner)
            && (!access.write || program.field_write_permitted(member, self.method))
    }

    fn initialized_on_entry(self) -> bool {
        let program = self.program;
        let owner = self.method.class;
        if program.method(self.method).access_flags & STATIC != 0 {
            return true;
        }
        // An instance can escape during initialization, including from a
        // Java 8 default-bearing superinterface, before its class fails.
        program.complete_hierarchy(owner)
            && crate::initialization::quiet_initialization(program, &program.class(owner).name)
            && program.ancestors(owner).iter().all(|&id| {
                let class = program.class(id);
                class.access_flags & INTERFACE == 0
                    || class.major_version < 52
                    || !class
                        .methods
                        .iter()
                        .any(|m| m.access_flags & (ABSTRACT | STATIC) == 0)
                    || crate::initialization::quiet_initialization(program, &class.name)
            })
    }

    // Follow paths on which this class might still be uninitialized. A normal
    // active use ends such a path. Its handler may instead have caught a failed
    // initialization, so exceptional successors remain in the search.
    fn activated_after(self, ir: &MethodIr, class: ClassId) -> IdVec<BlockId, usize> {
        let owner = &self.program.class(class).name;
        let activates = |values: scheduled::Values<'_>| {
            let mut found = false;
            for op in values.operations() {
                match &ir.values[op].kind {
                    // Either arm may be skipped; an active use in just one is
                    // not an initialization guarantee for the continuation.
                    ValueKind::Select { .. } => return false,
                    ValueKind::Call {
                        kind: InvokeKind::Static,
                        method,
                        ..
                    } => {
                        found |= method.owner == *owner
                            && self
                                .program
                                .declared_method(class, &method.name, &method.descriptor)
                                .is_some();
                    }
                    ValueKind::New { class, .. } | ValueKind::InitializeClass { class } => {
                        found |= class == owner
                    }
                    ValueKind::Field { field, receiver }
                    | ValueKind::Update {
                        target: UpdateTarget::Field { field, receiver },
                        ..
                    }
                    | ValueKind::CompoundUpdate {
                        target: UpdateTarget::Field { field, receiver },
                        ..
                    } if receiver.is_none() => found |= self.declared_static(class, field),
                    _ => (),
                }
            }
            found
        };
        // Zero means all incoming paths have already initialized the class;
        // usize::MAX means there is no unconditional active use in the block.
        let mut after: IdVec<BlockId, usize> = vec![0; ir.blocks.len()].into();
        let mut pending = vec![ir.entry];
        while let Some(node) = pending.pop() {
            if after[node] != 0 {
                continue;
            }
            let block = &ir.blocks[node];
            pending.extend(crate::cfg::dispatch::effective(&block.handlers).map(|h| h.target));
            let first = block.statements.iter().position(|stmt| {
                matches!(stmt, Statement::PutField {field:f, receiver:None,..} if self.declared_static(class, f))
                    || activates(scheduled::statement_values(ir, stmt))
            });
            after[node] = first.map_or(usize::MAX, |index| index + 1);
            if first.is_none() && !activates(scheduled::terminator_values(ir, &block.terminator)) {
                pending.extend(block.terminator.successors());
            }
        }
        after
    }
}
