//! Control-flow reconstruction over immutable, densely indexed blocks.
//!
//! The tree refers to the lifted expressions rather than copying them. Every
//! bytecode transfer becomes either a tree edge or an explicit source exit.
//! Ordered exception dispatch remains part of the input throughout structuring.

mod analysis;
mod attempt;
pub use attempt::{RecoveryAttempt, RecoveryRejection, RecoveryStrategy, RejectionReason};
mod bindings;
mod cleanup;
pub(crate) use cleanup::facts as cleanup_facts;
mod connectors;
mod continuations;
mod duplication;
mod eligibility;
mod exception;
mod finally;
mod installation;
mod irreducible;
mod predicates;
mod prefix;
mod prepare;
pub use prepare::{PreparedIr, split_nonthrowing_prefixes};
mod recover;
mod regions;
mod scopes;
mod stackify;
mod synchronized;
pub(crate) mod tree;

use crate::cfg::HandlerEdge;
use crate::ir::scheduled::{self, ValueUse, statement_may_throw};
use crate::ir::{MethodIr, Terminator};

pub use crate::index::BlockId;
use crate::index::IdVec;
pub type RegionId = usize;

#[derive(Clone, Debug)]
pub struct FlowGraph {
    pub entry: BlockId,
    pub blocks: IdVec<BlockId, FlowBlock>,
}

#[derive(Clone, Debug)]
pub struct FlowBlock {
    pub pc: u32,
    /// Only the terminator executes here; opaque recovered regions set false.
    pub statements_empty: bool,
    /// Bounded emitted-expression cost for an eligible linear source block.
    pub duplication_cost: Option<u16>,
    /// Exactly one nonthrowing assignment that reads and updates its local.
    pub is_loop_update: bool,
    pub end: FlowEnd,
    pub handlers: Vec<HandlerEdge>,
    /// False only when all scheduled expressions and operations cannot throw.
    pub may_throw: bool,
}

#[derive(Clone, Debug)]
pub enum FlowEnd {
    Goto(BlockId),
    Branch {
        then_target: BlockId,
        else_target: BlockId,
    },
    Switch {
        cases: Vec<(i32, BlockId)>,
        default: BlockId,
    },
    Return,
    Throw,
    Unreachable,
    /// Already structured region: (logical exit, graph destination).
    Multi(Vec<(BlockId, BlockId)>),
}

impl FlowEnd {
    /// Remap actual CFG destinations while preserving semantic exit identities.
    fn map_destinations(&mut self, mut map: impl FnMut(BlockId) -> BlockId) {
        match self {
            Self::Goto(target) => *target = map(*target),
            Self::Branch {
                then_target,
                else_target,
            } => {
                *then_target = map(*then_target);
                *else_target = map(*else_target);
            }
            Self::Switch { cases, default } => {
                for (_, target) in cases {
                    *target = map(*target);
                }
                *default = map(*default);
            }
            Self::Multi(exits) => {
                for (_, target) in exits {
                    *target = map(*target);
                }
            }
            _ => (),
        }
    }

    pub fn successors(&self) -> Vec<BlockId> {
        let mut result = match self {
            Self::Goto(target) => vec![*target],
            Self::Branch {
                then_target,
                else_target,
            } => vec![*else_target, *then_target],
            Self::Switch { cases, default } => {
                let mut targets: Vec<_> = cases.iter().map(|&(_, target)| target).collect();
                targets.push(*default);
                targets
            }
            Self::Multi(exits) => exits.iter().map(|&(_, target)| target).collect(),
            _ => Vec::new(),
        };
        result.sort_unstable();
        result.dedup();
        result
    }
}

impl From<&MethodIr> for FlowGraph {
    fn from(ir: &MethodIr) -> Self {
        Self {
            entry: ir.entry,
            blocks: ir
                .blocks
                .iter_enumerated()
                .map(|(id, block)| {
                    let may_throw = block_may_throw(ir, id);
                    FlowBlock {
                        pc: block.pc,
                        statements_empty: block.statements.is_empty(),
                        duplication_cost: duplication::linear_cost(ir, id),
                        is_loop_update: recover::is_loop_update(ir, id, may_throw),
                        end: match &block.terminator {
                            Terminator::Goto(target) => FlowEnd::Goto(*target),
                            Terminator::Branch {
                                then_target,
                                else_target,
                                ..
                            } => FlowEnd::Branch {
                                then_target: *then_target,
                                else_target: *else_target,
                            },
                            Terminator::Switch { cases, default, .. } => FlowEnd::Switch {
                                cases: cases.clone(),
                                default: *default,
                            },
                            Terminator::Return(_) => FlowEnd::Return,
                            Terminator::Throw { .. } => FlowEnd::Throw,
                            Terminator::Unreachable => FlowEnd::Unreachable,
                        },
                        handlers: block
                            .handlers
                            .iter()
                            .filter(|handler| handler.target != id || may_throw)
                            .cloned()
                            .collect(),
                        may_throw,
                    }
                })
                .collect(),
        }
    }
}

fn block_may_throw(ir: &MethodIr, block: BlockId) -> bool {
    let block = &ir.blocks[block];
    block
        .statements
        .iter()
        .any(|statement| statement_may_throw(ir, statement))
        || terminator_may_throw(ir, &block.terminator)
}

fn terminator_may_throw(ir: &MethodIr, terminator: &Terminator) -> bool {
    matches!(terminator, Terminator::Throw { .. })
        || terminator
            .operands()
            .into_iter()
            .any(|value| scheduled::value_may_throw(ir, ValueUse::Reference(value)))
}

#[derive(Clone, Debug)]
pub struct StructuredMethod {
    pub body: Vec<Stmt>,
    pub statistics: StructureStatistics,
    /// Strategy selection on the final graph of installed semantic regions:
    /// rejected strategies and the selected fallback. Empty when no strategy
    /// was rejected. Probes inside region candidates are excluded.
    pub recovery_attempts: Vec<RecoveryAttempt>,
    /// Monitor instructions whose ownership is represented by Synchronized.
    pub suppressed_monitors: Vec<(BlockId, usize)>,
}

#[derive(Clone, Debug, Default)]
pub struct StructureStatistics {
    /// Reducible regions expressed using forward labels rather than state.
    pub labeled_regions: usize,
    /// Regions requiring a state variable, normally irreducible SCCs.
    pub dispatch_regions: usize,
    pub dispatch_blocks: usize,
}

#[derive(Clone, Debug)]
pub enum Predicate {
    Atom {
        head: BlockId,
        negated: bool,
    },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Select {
        condition: Box<Predicate>,
        yes: Box<Predicate>,
        no: Box<Predicate>,
    },
}

impl Predicate {
    /// Compare ordered predicate shapes using the caller's atom equivalence.
    pub(crate) fn equivalent_by(
        &self,
        other: &Self,
        atom: &mut impl FnMut(BlockId, bool, BlockId, bool) -> bool,
    ) -> bool {
        match (self, other) {
            (
                Self::Atom {
                    head: a,
                    negated: an,
                },
                Self::Atom {
                    head: b,
                    negated: bn,
                },
            ) => atom(*a, *an, *b, *bn),
            (Self::And(a), Self::And(b)) | (Self::Or(a), Self::Or(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.equivalent_by(b, atom))
            }
            (
                Self::Select {
                    condition: a,
                    yes: ay,
                    no: an,
                },
                Self::Select {
                    condition: b,
                    yes: by,
                    no: bn,
                },
            ) => {
                a.equivalent_by(b, atom) && ay.equivalent_by(by, atom) && an.equivalent_by(bn, atom)
            }
            _ => false,
        }
    }

    fn combine(self, right: Self, conjunction: bool) -> Self {
        let mut terms = match (conjunction, self) {
            (true, Self::And(children)) | (false, Self::Or(children)) => children,
            (_, predicate) => vec![predicate],
        };
        match (conjunction, right) {
            (true, Self::And(children)) | (false, Self::Or(children)) => terms.extend(children),
            (_, predicate) => terms.push(predicate),
        }
        if conjunction {
            Self::And(terms)
        } else {
            Self::Or(terms)
        }
    }

    /// Original bytecode tests, in their left-to-right evaluation order.
    pub fn heads(&self) -> Vec<BlockId> {
        let mut heads = Vec::new();
        let mut pending = vec![self];
        while let Some(predicate) = pending.pop() {
            match predicate {
                Self::Atom { head, .. } => heads.push(*head),
                Self::And(terms) | Self::Or(terms) => pending.extend(terms.iter().rev()),
                Self::Select { condition, yes, no } => {
                    pending.extend([no.as_ref(), yes.as_ref(), condition.as_ref()]);
                }
            }
        }
        heads
    }

    pub fn negated(self) -> Self {
        match self {
            Self::Atom { head, negated } => Self::Atom {
                head,
                negated: !negated,
            },
            Self::And(terms) => Self::Or(terms.into_iter().map(Self::negated).collect()),
            Self::Or(terms) => Self::And(terms.into_iter().map(Self::negated).collect()),
            Self::Select { condition, yes, no } => Self::Select {
                condition,
                yes: Box::new(yes.negated()),
                no: Box::new(no.negated()),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// Emit only the scheduled statements, never the block's terminator.
    Basic(BlockId),
    /// Exact scheduled operations when a lexical boundary falls inside a block.
    Statements {
        block: BlockId,
        start: usize,
        end: usize,
    },
    Return(BlockId),
    Throw(BlockId),
    /// A genuinely unreachable bytecode terminator, not a recovery placeholder.
    Unreachable(BlockId),
    If {
        condition: Predicate,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    /// Initially an infinite loop. A leading guard supplies its source test;
    /// an owned continuation supplies a source `for` update.
    Loop {
        region: RegionId,
        header: BlockId,
        body: Vec<Stmt>,
        /// Owned iteration boundary. Normal body completion and Continue to
        /// this loop execute this scheduled block once, before the next guard.
        /// Break, Return, and Throw skip it. None preserves direct-header
        /// continue semantics; a loop with a bypassing backedge cannot own one.
        continuation: Option<BlockId>,
    },
    Switch {
        region: RegionId,
        head: BlockId,
        cases: Vec<SwitchCase>,
    },
    Label {
        region: RegionId,
        body: Vec<Stmt>,
    },
    Break {
        target: RegionId,
    },
    /// Complete this iteration of the named loop. Its owned continuation, if
    /// any, runs before the next guard; without one this jumps to the guard.
    Continue {
        header: BlockId,
    },
    DispatchContinue {
        region: RegionId,
        target: BlockId,
    },
    /// Internal region exit, bound to lexical control before Java emission.
    Jump(BlockId),
    /// Internal invocation of an opaque region with its lexical exit bindings.
    BoundRegion {
        head: BlockId,
        exits: Vec<(BlockId, Vec<Stmt>)>,
    },
    Try {
        body: Vec<Stmt>,
        catches: Vec<Catch>,
    },
    /// Cleanup whose copies were proved on every outward normal transfer.
    TryFinally {
        body: Vec<Stmt>,
        cleanup: Vec<Stmt>,
        /// Original catch-all entry, retained as the exception-analysis proof.
        handler: BlockId,
    },
    Synchronized {
        monitor: crate::ir::ValueId,
        body: Vec<Stmt>,
    },
    /// A local bytecode state dispatcher. Each case emits its block statements
    /// and original terminator; handlers are dispatched in original table order.
    /// External destinations leave through the corresponding enclosing label.
    Dispatch {
        region: RegionId,
        entry: BlockId,
        cases: Vec<DispatchCase>,
        exits: Vec<(BlockId, RegionId)>,
    },
}

pub struct ScheduledStatements<'a> {
    pub block: BlockId,
    pub start: usize,
    pub statements: &'a [crate::ir::Statement],
}

impl ScheduledStatements<'_> {
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start..self.start + self.statements.len()
    }
    pub fn iter(&self) -> impl Iterator<Item = (usize, &crate::ir::Statement)> {
        self.statements
            .iter()
            .enumerate()
            .map(|(index, statement)| (self.start + index, statement))
    }
}

impl Stmt {
    pub fn scheduled<'a>(&self, ir: &'a MethodIr) -> Option<ScheduledStatements<'a>> {
        let (block, start, end) = match *self {
            Self::Basic(block) => (block, 0, ir.blocks[block].statements.len()),
            Self::Statements { block, start, end } => (block, start, end),
            _ => return None,
        };
        Some(ScheduledStatements {
            block,
            start,
            statements: &ir.blocks[block].statements[start..end],
        })
    }
}

#[derive(Clone, Debug)]
pub struct DispatchCase {
    pub head: BlockId,
    pub body: Vec<Stmt>,
    pub end: FlowEnd,
    pub handlers: Vec<HandlerEdge>,
}

#[derive(Clone, Debug)]
pub struct SwitchCase {
    pub keys: Vec<i32>,
    pub default: bool,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Catch {
    pub catch_type: Option<String>,
    /// Identifies the caught-exception SSA value in `MethodIr`.
    pub handler: BlockId,
    pub body: Vec<Stmt>,
}

pub fn structure(prepared: &PreparedIr) -> crate::Result<StructuredMethod> {
    structure_with_filter(prepared, None)
}

/// Source-language eligibility is a view over the proved regions. The ordered
/// bytecode handlers in MethodIr remain intact for monitor and cleanup proofs.
pub(crate) type HandlerFilter<'a> =
    dyn Fn(BlockId, Option<&[Stmt]>, &[HandlerEdge]) -> Vec<HandlerEdge> + 'a;

pub(crate) fn structure_with_filter(
    prepared: &PreparedIr,
    filter: Option<&HandlerFilter<'_>>,
) -> crate::Result<StructuredMethod> {
    let analysis = finally::Analysis::new(prepared.ir());
    let result = cleanup::recover(&analysis, filter).lower(filter)?;
    finalize_source(analysis.ir(), result)
}

/// All recovery strategies end with expanded lexical regions. Source scope
/// preparation and dispatcher prefixes therefore run once over the complete
/// tree. Cleanup ownership is already settled in the graph, so continuation
/// and predicate projection see the final topology and remove redundant labels
/// bottom-up without another cleanup proof or structural repair.
fn finalize_source(ir: &MethodIr, mut result: StructuredMethod) -> crate::Result<StructuredMethod> {
    ensure_lowered(&result.body)?;
    result.body = scopes::normalize(ir, result.body);
    result.body = prefix::normalize(result.body);
    result.body = predicates::normalize(ir, result.body);
    renumber_regions(&mut result.body);
    ensure_lowered(&result.body)?;
    result.statistics = statistics(&result.body);
    Ok(result)
}

pub fn structure_graph(graph: &FlowGraph) -> crate::Result<StructuredMethod> {
    let mut result = structure_graph_inner(graph, &eligibility::Eligibility::default())?;
    result.body = prefix::normalize(result.body);
    renumber_regions(&mut result.body);
    result.statistics = statistics(&result.body);
    Ok(result)
}

fn structure_graph_inner(
    graph: &FlowGraph,
    eligibility: &eligibility::Eligibility<'_>,
) -> crate::Result<StructuredMethod> {
    analysis::validate(graph)?;
    let mut recovery_attempts = Vec::new();
    if graph.blocks.is_empty() {
        return Ok(StructuredMethod {
            body: Vec::new(),
            statistics: StructureStatistics::default(),
            recovery_attempts,
            suppressed_monitors: Vec::new(),
        });
    }
    if graph.blocks.iter().any(|block| !block.handlers.is_empty())
        && let Some(body) =
            exception::recover_with_eligibility(graph, eligibility, &mut recovery_attempts)
    {
        let labeled_regions = count_labels(&body);
        return Ok(StructuredMethod {
            body,
            statistics: StructureStatistics {
                labeled_regions,
                ..Default::default()
            },
            suppressed_monitors: Vec::new(),
            recovery_attempts,
        });
    }
    if let Ok(body) = recover_normal_flow(graph, &mut recovery_attempts) {
        return Ok(StructuredMethod {
            statistics: statistics(&body),
            body,
            suppressed_monitors: Vec::new(),
            recovery_attempts,
        });
    }
    let body = installation::dispatch(graph, eligibility);
    recovery_attempts.push(RecoveryAttempt {
        entry: graph.entry,
        strategy: RecoveryStrategy::Dispatch,
        rejection: None,
    });
    Ok(StructuredMethod {
        statistics: statistics(&body),
        body,
        suppressed_monitors: Vec::new(),
        recovery_attempts,
    })
}

/// Alternative representations of one handler-free graph, from ordinary
/// source regions to lexical jumps to local irreducible dispatch. Exception
/// recovery invokes the same strategy selection after installing its regions;
/// none of these alternatives is a postprocessing pass over another's tree.
fn recover_normal_flow(
    graph: &FlowGraph,
    attempts: &mut Vec<RecoveryAttempt>,
) -> attempt::RecoveryResult<Vec<Stmt>> {
    let contracted = connectors::contract(graph);
    let graph = contracted.as_ref().unwrap_or(graph);
    let analysis = analysis::Analysis::new(graph);
    let decisions = recover::Decisions::project(graph, &analysis);
    let graph = decisions
        .as_ref()
        .map_or(graph, |decisions| &decisions.graph);
    let analysis = decisions
        .as_ref()
        .map(|_| analysis::Analysis::new(graph))
        .unwrap_or(analysis);
    let mut body = attempt::record(
        attempts,
        graph.entry,
        RecoveryStrategy::Ordinary,
        recover::recover(graph, &analysis),
    )
    .or_else(|_| {
        attempt::record(
            attempts,
            graph.entry,
            RecoveryStrategy::Lexical,
            stackify::stackify(graph, &analysis),
        )
    })
    .or_else(|_| {
        attempt::record(
            attempts,
            graph.entry,
            RecoveryStrategy::EntryDuplication,
            irreducible::recover(graph, &analysis),
        )
    })?;
    if let Some(decisions) = decisions {
        decisions.expand(&mut body);
    }
    Ok(body)
}

fn count_labels(body: &[Stmt]) -> usize {
    tree::walk(body)
        .filter(|statement| matches!(statement, Stmt::Label { .. }))
        .count()
}

fn statistics(body: &[Stmt]) -> StructureStatistics {
    let mut result = StructureStatistics::default();
    for statement in tree::walk(body) {
        match statement {
            Stmt::Label { .. } => result.labeled_regions += 1,
            Stmt::Dispatch { cases, .. } => {
                result.dispatch_regions += 1;
                result.dispatch_blocks += cases.len();
            }
            _ => (),
        }
    }
    result
}

pub fn can_complete(body: &[Stmt]) -> bool {
    body.last().is_none_or(|stmt| match stmt {
        Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::Unreachable(_)
        | Stmt::Break { .. }
        | Stmt::Continue { .. }
        | Stmt::DispatchContinue { .. }
        | Stmt::Dispatch { .. }
        | Stmt::Jump(_) => false,
        Stmt::If {
            then_body,
            else_body,
            ..
        } => can_complete(then_body) || can_complete(else_body),
        Stmt::Loop { region, body, .. } => contains_break(body, *region),
        Stmt::Label { region, body } => can_complete(body) || contains_break(body, *region),
        Stmt::Try { body, catches } => {
            can_complete(body) || catches.iter().any(|catch| can_complete(&catch.body))
        }
        Stmt::Synchronized { body, .. } => can_complete(body),
        Stmt::TryFinally { body, cleanup, .. } => can_complete(body) && can_complete(cleanup),
        Stmt::Switch { region, cases, .. } => {
            !cases.iter().any(|case| case.default)
                || cases.iter().any(|case| contains_break(&case.body, *region))
                || cases.last().is_none_or(|case| can_complete(&case.body))
        }
        Stmt::Basic(_) | Stmt::Statements { .. } | Stmt::BoundRegion { .. } => true,
    })
}

fn renumber_regions(body: &mut [Stmt]) {
    renumber_regions_from(body, 0);
}

fn renumber_regions_from(body: &mut [Stmt], start: usize) -> usize {
    bindings::renumber_regions_from(body, start)
}

fn contains_break(body: &[Stmt], target: RegionId) -> bool {
    body.iter().any(|statement| {
        let exits = match statement {
            Stmt::Break { target: found } => *found == target,
            Stmt::Dispatch { exits, .. } => exits.iter().any(|&(_, region)| region == target),
            _ => false,
        };
        // Independently recovered regions can reuse IDs until renumbering.
        // A nested binder shadows this target, but dispatch exits are outside it.
        exits
            || (statement
                .binder()
                .is_none_or(|binder| binder.region() != target)
                && statement.bodies().any(|body| contains_break(body, target)))
    })
}

fn ensure_lowered(body: &[Stmt]) -> crate::Result<()> {
    anyhow::ensure!(
        !tree::walk(body)
            .any(|statement| matches!(statement, Stmt::Jump(_) | Stmt::BoundRegion { .. })),
        "unlowered control-flow region remained after structuring"
    );
    Ok(())
}
