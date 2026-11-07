//! Keep one-shot entry effects outside a residual state loop. Besides reducing
//! noise, this lets Java prove constructor blank-final assignments occur once.
use super::{FlowEnd, Stmt, tree};

pub(super) fn normalize(body: Vec<Stmt>) -> Vec<Stmt> {
    let mut result = Vec::new();
    for statement in body {
        let statement = statement.map_bodies(normalize);
        if let Stmt::Dispatch {
            region,
            mut entry,
            mut cases,
            exits,
        } = statement
        {
            while let Some(index) = cases.iter().position(|case| case.head == entry) {
                // A predecessor inside the remaining graph can execute this
                // case again. Exception edges count even if a normal SCC does not.
                if cases.iter().any(|case| {
                    case.end.successors().contains(&entry)
                        || case.handlers.iter().any(|handler| handler.target == entry)
                        || dispatches_to(&case.body, region, entry)
                }) {
                    break;
                }
                if !cases[index].handlers.is_empty()
                    || refers_to_dispatch(&cases[index].body, region)
                {
                    break;
                }
                result.append(&mut cases[index].body);
                if let FlowEnd::Goto(target) = cases[index].end
                    && cases.iter().any(|case| case.head == target)
                {
                    cases.remove(index);
                    entry = target;
                    continue;
                }
                // Keep branching/terminal evaluation at its original case. The
                // body has moved exactly once; no graph or instruction is cloned.
                break;
            }
            result.push(Stmt::Dispatch {
                region,
                entry,
                cases,
                exits,
            });
        } else {
            result.push(statement);
        }
    }
    result
}

fn dispatches_to(body: &[Stmt], region: usize, entry: crate::ir::BlockId) -> bool {
    tree::walk(body).any(|statement| {
        matches!(statement,
        Stmt::DispatchContinue { region: target_region, target }
            if *target_region == region && *target == entry)
    })
}

fn refers_to_dispatch(body: &[Stmt], region: usize) -> bool {
    tree::walk(body).any(|statement| match statement {
        Stmt::DispatchContinue { region: target, .. } | Stmt::Break { target } => *target == region,
        Stmt::Jump(_) | Stmt::BoundRegion { .. } => true,
        _ => false,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/structure_prefix.rs"]
mod tests;
