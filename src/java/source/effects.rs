//! Java's checked-catch and method-boundary rules apply to surviving bound
//! operations. Handler ownership was proved earlier; this pass never changes it.
use super::bound::{Body, Statement, Throw};
use crate::{
    java::{exceptions::Thrown, syntax::Expr},
    program::Program,
};

pub(super) fn finalize(program: &Program<'_>, body: &mut Body) -> Thrown {
    let mut result = Thrown::default();
    for statement in body {
        super::evaluation::visit_immediate_expressions(statement, |expression| {
            if let Some(effects) = &expression.facts.throws {
                result.extend((**effects).clone());
            }
        });
        match statement {
            Statement::Throw(value) => result.extend(value.effects.clone()),
            Statement::Try {
                body,
                catches,
                cleanup,
            } => {
                let mut protected = finalize(program, body);
                let mut witnesses = Vec::new();
                for catch in catches.iter_mut() {
                    if !protected.source_caught_by(program, Some(&catch.catch_type)) {
                        witnesses.push(Statement::If {
                            condition: Expr::boolean(false),
                            yes: vec![Statement::Throw(Throw {
                                value: Expr::cast(&catch.ty, Expr::null()),
                                // Unreachable and handled by this catch; this
                                // source-checker witness has no escaping effect.
                                effects: Thrown::default(),
                            })],
                            no: Vec::new(),
                        });
                    }
                    result.extend(finalize(program, &mut catch.body));
                }
                // Preserve the already-owned catch without reviving an operation
                // removed by source facts, loop presentation or completion.
                body.splice(..0, witnesses);
                for catch in catches {
                    protected.remove_caught(program, Some(&catch.catch_type));
                }
                result.extend(protected);
                if let Some(cleanup) = cleanup {
                    result.extend(finalize(program, cleanup));
                }
            }
            _ => super::evaluation::children(statement, |body| {
                result.extend(finalize(program, body));
            }),
        }
    }
    result
}
