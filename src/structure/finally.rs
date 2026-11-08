//! Prove cleanup ownership on the scheduled graph before handler lowering.
//! Every outward normal transfer must already execute equivalent cleanup.
//! Internal loop backedges are not exits from the protected region.
mod completion;
mod equivalent;
mod region;
mod specialization;
#[cfg(test)]
#[path = "../../tests/unit/cleanup_analysis.rs"]
mod tests;
pub(super) use region::plan_candidate;

use super::Stmt;
use crate::ir::MethodIr;
use std::cell::OnceCell;

/// Copy contents belong to this exact immutable scheduled graph. Region
/// composition changes only views of it, so all cleanup candidates share one
/// lazy analysis until this method's structuring ends.
pub(super) struct Analysis<'a> {
    ir: &'a MethodIr,
    available: OnceCell<crate::ir::AvailableCopies>,
}

impl<'a> Analysis<'a> {
    pub fn new(ir: &'a MethodIr) -> Self {
        Self {
            ir,
            available: OnceCell::new(),
        }
    }

    pub fn ir(&self) -> &'a MethodIr {
        self.ir
    }

    fn available(&self) -> &crate::ir::AvailableCopies {
        self.available
            .get_or_init(|| crate::ir::AvailableCopies::analyze(self.ir))
    }

    fn same(&self, left: &[Stmt], right: &[Stmt]) -> bool {
        equivalent::same(self, left, right)
    }
}

fn cleanup_control(body: &[Stmt]) -> bool {
    use super::tree::Binder;
    let mut valid = true;
    super::tree::walk_scoped(body, &mut |statement, scope| {
        valid &= match statement {
            Stmt::Basic(_)
            | Stmt::Statements { .. }
            | Stmt::Throw(_)
            | Stmt::If { .. }
            | Stmt::Loop { .. }
            | Stmt::Label { .. }
            | Stmt::Switch { .. }
            | Stmt::Try { .. }
            | Stmt::TryFinally { .. }
            | Stmt::Synchronized { .. } => true,
            Stmt::Break { target } => scope.iter().rev().any(|binder| binder.region() == *target),
            Stmt::Continue { header } => scope.iter().rev().any(
                |binder| matches!(binder, Binder::Loop { header: owner, .. } if owner == header),
            ),
            _ => false,
        };
    });
    valid
}
