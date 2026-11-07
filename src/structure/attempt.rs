//! Outcomes of committed recovery strategies, not speculative pattern probes.
use super::BlockId;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStrategy {
    ExceptionRegions,
    Ordinary,
    Lexical,
    EntryDuplication,
    Dispatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectionReason {
    /// A throwing block still carries ordered handler dispatch.
    ProtectedBlock,
    /// An exit cannot be owned by this loop or an enclosing source scope.
    LoopExit {
        header: BlockId,
    },
    /// A shared continuation is not an eligible linear source tail.
    SharedTail,
    DuplicationLimit,
    /// The proposed region crosses an already owned or external boundary.
    RegionBoundary,
    /// Remaining transfers cannot be put in source order without a cycle.
    CyclicOrder,
    NestingLimit,
    /// Entry duplication is inapplicable: no eligible shared prefix was found.
    EntryPrefix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryRejection {
    pub block: BlockId,
    pub reason: RejectionReason,
}

impl RecoveryRejection {
    pub(super) fn at(block: BlockId, reason: RejectionReason) -> Self {
        Self { block, reason }
    }
}

/// One strategy applied to a graph entry. A missing rejection means selected
/// after an earlier rejection. Straightforward recovery needs no history;
/// candidate-local probes do not appear here either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryAttempt {
    pub entry: BlockId,
    pub strategy: RecoveryStrategy,
    pub rejection: Option<RecoveryRejection>,
}

pub(super) type RecoveryResult<T> = Result<T, RecoveryRejection>;

pub(super) fn record<T>(
    attempts: &mut Vec<RecoveryAttempt>,
    entry: BlockId,
    strategy: RecoveryStrategy,
    result: RecoveryResult<T>,
) -> RecoveryResult<T> {
    if result.is_err() || !attempts.is_empty() {
        attempts.push(RecoveryAttempt {
            entry,
            strategy,
            rejection: result.as_ref().err().copied(),
        });
    }
    result
}

impl fmt::Display for RecoveryStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ExceptionRegions => "exception regions",
            Self::Ordinary => "ordinary regions",
            Self::Lexical => "lexical jumps",
            Self::EntryDuplication => "entry duplication",
            Self::Dispatch => "dispatch",
        })
    }
}

impl fmt::Display for RecoveryRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B{}: ", self.block)?;
        match self.reason {
            RejectionReason::ProtectedBlock => {
                f.write_str("throwing block retains handler dispatch")
            }
            RejectionReason::LoopExit { header } => {
                write!(f, "loop B{header} exits have no owned continuation")
            }
            RejectionReason::SharedTail => {
                f.write_str("shared tail is not eligible for duplication")
            }
            RejectionReason::DuplicationLimit => f.write_str("source duplication limit reached"),
            RejectionReason::RegionBoundary => {
                f.write_str("flow crosses the region's owned boundary")
            }
            RejectionReason::CyclicOrder => {
                f.write_str("transfers cannot be ordered without a cycle")
            }
            RejectionReason::NestingLimit => f.write_str("recovery nesting limit reached"),
            RejectionReason::EntryPrefix => f.write_str("no eligible shared entry prefix"),
        }
    }
}

impl fmt::Display for RecoveryAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at B{}: ", self.strategy, self.entry)?;
        match self.rejection {
            Some(rejection) if rejection.reason == RejectionReason::EntryPrefix => {
                write!(f, "not applicable at {rejection}")
            }
            Some(rejection) => write!(f, "rejected at {rejection}"),
            None => f.write_str("selected"),
        }
    }
}
