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

use super::{Stmt, analysis::MethodAnalysis as Analysis};

impl Analysis<'_> {
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
