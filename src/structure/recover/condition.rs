//! Compose uniquely entered decision regions before recovering their bodies.
//! A child such as `b && c` must expose its two outcomes before its parent can
//! recognize `a || (b && c)`; looking only at adjacent raw branches misses it.
use super::{Analysis, BlockId, Builder, FlowEnd, FlowGraph, Predicate, Stmt};

pub(in crate::structure) struct Decisions {
    pub graph: FlowGraph,
    predicates: crate::index::IdVec<BlockId, Option<Predicate>>,
    #[cfg(test)]
    expanded: usize,
}

impl Decisions {
    /// All recovery strategies consume the same uniquely owned decisions.
    /// A later enclosing lexical fallback must not split their predicates back
    /// into raw bytecode branches and discard the recovered body boundary.
    pub fn project(graph: &FlowGraph, analysis: &Analysis) -> Option<Self> {
        let mut planner = Planner::new(Builder::new(graph, analysis));
        let mut result = None;
        let mut predicates: crate::index::IdVec<BlockId, _> = vec![None; graph.blocks.len()].into();
        let mut pending = vec![graph.entry];
        while let Some(head) = pending.pop() {
            if planner.builder.visited[head] {
                continue;
            }
            planner.builder.mark(head);
            let FlowEnd::Branch {
                then_target,
                else_target,
            } = graph.blocks[head].end
            else {
                pending.extend(analysis.successors[head].iter().rev().copied());
                continue;
            };
            if then_target == else_target {
                pending.push(then_target);
                continue;
            }
            let plan = planner.decision(head, then_target, else_target);
            pending.extend([plan.no, plan.yes]);
            if plan.heads.len() == 1 {
                continue;
            }
            let projected = result.get_or_insert_with(|| graph.clone());
            projected.blocks[head].end = FlowEnd::Branch {
                then_target: plan.yes,
                else_target: plan.no,
            };
            for node in plan.heads.into_iter().skip(1) {
                planner.builder.mark(node);
                projected.blocks[head].may_throw |= graph.blocks[node].may_throw;
                projected.blocks[node].end = FlowEnd::Unreachable;
                projected.blocks[node].handlers.clear();
                projected.blocks[node].may_throw = false;
            }
            predicates[head] = Some(plan.predicate);
        }
        result.map(|graph| Self {
            graph,
            predicates,
            #[cfg(test)]
            expanded: planner.expanded,
        })
    }

    pub fn expand(&self, body: &mut [Stmt]) {
        let mut pending: Vec<_> = body.iter_mut().collect();
        while let Some(statement) = pending.pop() {
            if let Stmt::If { condition, .. } = statement {
                let mut predicates = vec![condition];
                while let Some(predicate) = predicates.pop() {
                    match predicate {
                        Predicate::Atom { head, negated } => {
                            if let Some(original) = &self.predicates[*head] {
                                *predicate = if *negated {
                                    original.clone().negated()
                                } else {
                                    original.clone()
                                };
                            }
                        }
                        Predicate::And(terms) | Predicate::Or(terms) => predicates.extend(terms),
                        Predicate::Select { condition, yes, no } => {
                            predicates.extend([condition.as_mut(), yes.as_mut(), no.as_mut()]);
                        }
                    }
                }
            }
            pending.extend(statement.bodies_mut().flatten());
        }
    }
}

struct Decision {
    predicate: Predicate,
    yes: BlockId,
    no: BlockId,
    heads: Vec<BlockId>,
}

impl Decision {
    fn atom(head: BlockId, yes: BlockId, no: BlockId) -> Self {
        Self {
            predicate: Predicate::Atom {
                head,
                negated: false,
            },
            yes,
            no,
            heads: vec![head],
        }
    }
}

enum State {
    Unseen,
    Pending,
    Ready(Decision),
    Adopted,
}

struct Frame {
    plan: Decision,
    next_arm: usize,
}

struct Planner<'a> {
    builder: Builder<'a>,
    states: crate::index::IdVec<BlockId, State>,
    #[cfg(test)]
    expanded: usize,
}

impl<'a> Planner<'a> {
    fn new(builder: Builder<'a>) -> Self {
        let states = (0..builder.graph.blocks.len())
            .map(|_| State::Unseen)
            .collect();
        Self {
            builder,
            states,
            #[cfg(test)]
            expanded: 0,
        }
    }

    /// Retain unsuccessful child plans: an OR pair in an else-if ladder tries
    /// the remaining ladder both before and after combining its own two tests.
    /// Recursively rebuilding that suffix doubles the work at each pair.
    ///
    /// Each head is planned once for this graph, including across projection
    /// roots. An adopted plan moves into its sole predecessor's plan; retained
    /// child plans own disjoint subtrees and remain valid as roots are committed.
    /// The worklist and cache together use linear storage.
    fn decision(&mut self, head: BlockId, yes: BlockId, no: BlockId) -> Decision {
        if matches!(self.states[head], State::Ready(_)) {
            return self.take(head);
        }
        let mut pending = vec![self.start(head, yes, no)];
        while let Some(mut frame) = pending.pop() {
            if frame.next_arm == 2 {
                if self.select(&mut frame.plan) {
                    frame.next_arm = 0;
                    pending.push(frame);
                    continue;
                }
                let head = frame.plan.heads[0];
                self.states[head] = State::Ready(frame.plan);
                continue;
            }
            let conjunction = frame.next_arm == 0;
            frame.next_arm += 1;
            let Some((candidate, yes, no, shared)) = self.candidate(&frame.plan, conjunction)
            else {
                pending.push(frame);
                continue;
            };
            match &self.states[candidate] {
                State::Unseen => {
                    // Resume the same arm after its child has been planned.
                    frame.next_arm -= 1;
                    pending.push(frame);
                    pending.push(self.start(candidate, yes, no));
                    continue;
                }
                State::Pending => {
                    pending.push(frame);
                    continue;
                }
                State::Adopted => {
                    // Its only incoming source belongs to the adopting plan.
                    // No independent candidate can enter an adopted child.
                    unreachable!("a decision has more than one owner");
                }
                State::Ready(child) => {
                    let outcome = if child.no == shared {
                        Some((child.yes, !conjunction))
                    } else if child.yes == shared {
                        Some((child.no, conjunction))
                    } else {
                        None
                    };
                    if let Some((remaining, negated)) = outcome
                        && !frame.plan.heads.contains(&remaining)
                    {
                        let State::Ready(child) =
                            std::mem::replace(&mut self.states[candidate], State::Adopted)
                        else {
                            unreachable!()
                        };
                        let child_predicate = if negated {
                            child.predicate.negated()
                        } else {
                            child.predicate
                        };
                        frame.plan.predicate =
                            frame.plan.predicate.combine(child_predicate, conjunction);
                        if conjunction {
                            frame.plan.yes = remaining;
                        } else {
                            frame.plan.no = remaining;
                        }
                        frame.plan.heads.extend(child.heads);
                        frame.next_arm = 0;
                    }
                }
            }
            pending.push(frame);
        }
        self.take(head)
    }

    fn take(&mut self, head: BlockId) -> Decision {
        let State::Ready(plan) = std::mem::replace(&mut self.states[head], State::Adopted) else {
            unreachable!()
        };
        plan
    }

    /// A decision can choose between two tests of the same outcomes. Preserve
    /// that choice as one predicate, evaluating its selector exactly once.
    /// Both child plans already proved unique entry and adopted their own
    /// descendants; shared outcome bodies stay outside the decision region.
    fn select(&mut self, plan: &mut Decision) -> bool {
        if plan.yes == plan.no {
            return false;
        }
        if self.candidate(plan, true).is_none() || self.candidate(plan, false).is_none() {
            return false;
        }
        let (State::Ready(yes), State::Ready(no)) = (&self.states[plan.yes], &self.states[plan.no])
        else {
            return false;
        };
        let invert = if yes.yes == no.yes && yes.no == no.no {
            false
        } else if yes.yes == no.no && yes.no == no.yes {
            true
        } else {
            return false;
        };
        if plan.heads.contains(&yes.yes) || plan.heads.contains(&yes.no) {
            return false;
        }
        let State::Ready(yes) = std::mem::replace(&mut self.states[plan.yes], State::Adopted)
        else {
            unreachable!()
        };
        let State::Ready(no) = std::mem::replace(&mut self.states[plan.no], State::Adopted) else {
            unreachable!()
        };
        let condition = std::mem::replace(&mut plan.predicate, Predicate::And(Vec::new()));
        plan.predicate = Predicate::Select {
            condition: Box::new(condition),
            yes: Box::new(yes.predicate),
            no: Box::new(if invert {
                no.predicate.negated()
            } else {
                no.predicate
            }),
        };
        plan.yes = yes.yes;
        plan.no = yes.no;
        plan.heads.extend(yes.heads);
        plan.heads.extend(no.heads);
        true
    }

    fn start(&mut self, head: BlockId, yes: BlockId, no: BlockId) -> Frame {
        self.states[head] = State::Pending;
        #[cfg(test)]
        {
            self.expanded += 1;
        }
        Frame {
            plan: Decision::atom(head, yes, no),
            next_arm: 0,
        }
    }

    fn candidate(
        &self,
        plan: &Decision,
        conjunction: bool,
    ) -> Option<(BlockId, BlockId, BlockId, BlockId)> {
        let (candidate, shared) = if conjunction {
            (plan.yes, plan.no)
        } else {
            (plan.no, plan.yes)
        };
        let builder = &self.builder;
        let block = &builder.graph.blocks[candidate];
        if !block.statements_empty
            || builder.visited[candidate]
            || builder.analysis.handler_entries[candidate]
            || builder.analysis.loop_at[candidate].is_some()
            || block.handlers != builder.graph.blocks[plan.heads[0]].handlers
            || plan.heads.contains(&candidate)
        {
            return None;
        }
        let mut incoming = builder.analysis.predecessors[candidate]
            .iter()
            .copied()
            .filter(|&source| builder.analysis.reachable[source]);
        if !incoming
            .next()
            .is_some_and(|source| plan.heads.contains(&source))
            || incoming.next().is_some()
        {
            return None;
        }
        let FlowEnd::Branch {
            then_target: yes,
            else_target: no,
        } = block.end
        else {
            return None;
        };
        (yes != no && yes != candidate && no != candidate).then_some((candidate, yes, no, shared))
    }
}

#[cfg(test)]
#[path = "condition_tests.rs"]
mod tests;
