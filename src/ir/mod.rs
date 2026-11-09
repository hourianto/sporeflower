//! Immutable JVM values, SSA frame joins, and a scheduled Java-oriented graph.
//!
//! A stack copy is another reference to the same value, never a copied expression.
//! Observable operations retain their instruction order until scheduling proves
//! that they can be embedded in a Java expression without changing that order.

use crate::classfile::{ClassFile, JvmType, Method};
use crate::{Result, bytecode::Instruction, cfg::ControlFlowGraph};

use crate::index::IdVec;
pub use crate::index::{BlockId, LocalId, ValueId};

/// How a scheduled occurrence uses an immutable value. A reference reads its
/// assigned storage when present; a definition evaluates its producer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueUse {
    Reference(ValueId),
    Definition(ValueId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Member {
    pub owner: String,
    pub name: String,
    pub descriptor: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvokeKind {
    Virtual,
    Special,
    Static,
    Interface,
}

#[derive(Clone, Debug)]
pub enum Literal {
    Null,
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(Vec<u16>),
    Class(JvmType),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    Ushr,
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    pub(crate) fn test(self, order: Option<std::cmp::Ordering>) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match self {
            Self::Eq => order == Some(Equal),
            Self::Ne => order != Some(Equal),
            Self::Lt => order == Some(Less),
            Self::Le => matches!(order, Some(Less | Equal)),
            Self::Gt => order == Some(Greater),
            Self::Ge => matches!(order, Some(Greater | Equal)),
        }
    }

    pub(crate) fn reversed(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Gt => Self::Lt,
            Self::Ge => Self::Le,
            op => op,
        }
    }

    pub fn inverted(self) -> Self {
        match self {
            Self::Eq => Self::Ne,
            Self::Ne => Self::Eq,
            Self::Lt => Self::Ge,
            Self::Le => Self::Gt,
            Self::Gt => Self::Le,
            Self::Ge => Self::Lt,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareKind {
    Long,
    FloatLess,
    FloatGreater,
    DoubleLess,
    DoubleGreater,
}

#[derive(Clone, Debug)]
pub struct PhiInput {
    pub block: BlockId,
    /// Exception edges leave immediately before this instruction, not at block end.
    pub throw_pc: Option<u32>,
    pub value: ValueId,
}

#[derive(Clone, Debug)]
pub enum UpdateTarget {
    Field {
        field: Member,
        receiver: Option<ValueId>,
    },
    Array {
        array: ValueId,
        index: ValueId,
        element_type: JvmType,
    },
}

#[derive(Clone, Debug)]
pub enum ValueKind {
    /// Explicit class-initialization boundary retained at an original NEW.
    /// The immutable source plan supplies a trigger only for an owned class.
    InitializeClass {
        class: String,
    },
    Alias(ValueId),
    /// Evaluate and capture a shared value at its first source-expression use.
    InlineAssign {
        local: LocalId,
        value: ValueId,
    },
    /// Commit the RHS to local storage while yielding its previous value for
    /// postfix syntax, or its committed value for prefix syntax. The RHS keeps
    /// the update's definition separate from the old expression result.
    LocalUpdate {
        local: LocalId,
        value: ValueId,
        increment: bool,
        postfix: bool,
    },
    Select {
        condition: Condition,
        then_value: ValueId,
        else_value: ValueId,
    },
    Parameter {
        slot: u16,
        index: Option<usize>,
    },
    Literal(Literal),
    Phi {
        inputs: Vec<PhiInput>,
        slot: Option<u16>,
    },
    Unary {
        op: UnaryOp,
        value: ValueId,
    },
    Binary {
        op: BinaryOp,
        left: ValueId,
        right: ValueId,
    },
    Compare {
        kind: CompareKind,
        left: ValueId,
        right: ValueId,
    },
    Cast {
        value: ValueId,
        ty: JvmType,
    },
    InstanceOf {
        value: ValueId,
        ty: JvmType,
    },
    Field {
        field: Member,
        receiver: Option<ValueId>,
    },
    Update {
        target: UpdateTarget,
        increment: bool,
        postfix: bool,
    },
    /// Owns the target once. Its read/null/bounds checks happen before the RHS,
    /// and the arithmetic/narrowing/write complete after it.
    CompoundUpdate {
        target: UpdateTarget,
        op: BinaryOp,
        right: ValueId,
        operation_type: JvmType,
    },
    ArrayLoad {
        array: ValueId,
        index: ValueId,
        element_type: JvmType,
    },
    ArrayLength {
        array: ValueId,
    },
    Call {
        kind: InvokeKind,
        method: Member,
        receiver: Option<ValueId>,
        args: Vec<ValueId>,
    },
    /// Only the verifier/lifter sees this token. Initializing one alias initializes
    /// all aliases in that frame, without conflating distinct NEW instructions.
    Allocation {
        class: String,
    },
    New {
        class: String,
        descriptor: String,
        args: Vec<ValueId>,
        allocation_pc: u32,
    },
    NewArray {
        element_type: JvmType,
        dimensions: Vec<ValueId>,
    },
    ArrayInitializer {
        element_type: JvmType,
        values: Vec<ValueId>,
        allocation_pc: u32,
    },
    CaughtException {
        handler: BlockId,
    },
}

#[derive(Clone, Debug)]
pub struct Value {
    /// Storage used to render this value, except at its definition's RHS root.
    pub local: Option<LocalId>,
    pub kind: ValueKind,
    pub ty: JvmType,
    pub pc: u32,
    pub block: BlockId,
}

#[derive(Clone, Debug)]
pub enum Statement {
    Assign {
        local: LocalId,
        value: ValueUse,
    },
    Eval(ValueId),
    PutField {
        /// Original putfield/putstatic offset; generated source-only stores have none.
        pc: Option<u32>,
        field: Member,
        receiver: Option<ValueId>,
        value: ValueId,
    },
    ArrayStore {
        array: ValueId,
        index: ValueId,
        value: ValueId,
        element_type: JvmType,
    },
    ConstructorCall {
        /// Original invokespecial offset, retained for instruction-scoped contracts.
        pc: u32,
        method: Member,
        receiver: ValueId,
        args: Vec<ValueId>,
    },
    Monitor {
        value: ValueId,
        enter: bool,
    },
}

#[derive(Clone, Debug)]
pub struct Condition {
    pub op: CompareOp,
    pub left: ValueId,
    pub right: ValueId,
}

#[derive(Clone, Debug)]
pub enum Terminator {
    Goto(BlockId),
    Branch {
        condition: Condition,
        then_target: BlockId,
        else_target: BlockId,
    },
    Switch {
        value: ValueId,
        cases: Vec<(i32, BlockId)>,
        default: BlockId,
    },
    Return(Option<ValueId>),
    Throw {
        value: ValueId,
        /// Exact normalized ATHROW origin, retained when its block is merged or split.
        pc: u32,
    },
    Unreachable,
}

impl Terminator {
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Self::Goto(target) => vec![*target],
            Self::Branch {
                then_target,
                else_target,
                ..
            } => {
                if then_target == else_target {
                    vec![*then_target]
                } else {
                    vec![*then_target, *else_target]
                }
            }
            Self::Switch { cases, default, .. } => {
                let mut result = vec![*default];
                for &(_, target) in cases {
                    if !result.contains(&target) {
                        result.push(target);
                    }
                }
                result
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct IrBlock {
    /// The caught value introduced by this handler entry, if any.
    pub caught_exception: Option<ValueId>,
    pub pc: u32,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
    /// Ordered lexical protection; use `cfg::dispatch` for executable edges.
    pub handlers: Vec<crate::cfg::HandlerEdge>,
}

#[derive(Clone, Debug)]
pub struct Local {
    /// Exact debug-variable identity supported by this storage's accesses.
    /// Naming evidence only; never part of SSA identity.
    pub name_hint: Option<crate::classfile::LocalVariable>,
    pub name: String,
    pub ty: JvmType,
    pub parameter: Option<usize>,
    pub is_this: bool,
    /// Java's definite-assignment rules do not follow individual exception edges.
    pub needs_default: bool,
}

#[derive(Clone, Debug)]
pub struct MethodIr {
    pub entry: BlockId,
    pub values: IdVec<ValueId, Value>,
    pub blocks: IdVec<BlockId, IrBlock>,
    pub locals: IdVec<LocalId, Local>,
    /// Includes the receiver first for instance methods.
    pub parameters: Vec<ValueId>,
    pub return_type: JvmType,
    /// Class initialization boundaries that ordinary Java NEW expressions moved
    /// past control flow or another observable operation. Source planning must
    /// resolve these before claiming successful emission.
    pub initialization_sites: Vec<InitializationSite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitializationSite {
    pub class: String,
    /// Stable instruction identity in normalized bytecode, before IR recovery.
    pub pc: u32,
    /// Scheduled IR region for allocation lowering, after expression recovery.
    /// Trigger replay must resolve `pc` in its own CFG instead of reusing this.
    pub block: BlockId,
    /// First scheduled statement after the instruction's prethrow snapshots; equal to
    /// statement count when the constructor only occurs in the terminator.
    pub statement_start: usize,
}

/// A query for a method's certified normal integer result.
pub type NormalIntReturn<'a> = &'a dyn Fn(&Member) -> Option<i32>;

/// A query for known reference widening conversions.
pub type ReferenceAssignable<'a> = &'a dyn Fn(&JvmType, &JvmType) -> bool;

/// Access flags of the field resolved by an original symbolic reference.
/// An unresolved reference supplies no proof about volatility.
pub type FieldAccessFlags<'a> = &'a dyn Fn(&Member) -> Option<u16>;

/// Optional original-program facts supplied before physical-local scheduling.
/// A normal result never implies that evaluating its call is effect-free.
#[derive(Default)]
pub struct MethodContext<'a> {
    pub initialization_sites: &'a [InitializationSite],
    pub normal_int_return: Option<NormalIntReturn<'a>>,
    /// Known reference widening conversions, including external API hierarchy.
    /// Unknown relationships retain conservative types and explicit conversions.
    pub reference_assignable: Option<ReferenceAssignable<'a>>,
    pub field_access_flags: Option<FieldAccessFlags<'a>>,
}

impl MethodIr {
    pub fn is_this(&self, value: ValueId) -> bool {
        matches!(
            self.values[value].kind,
            ValueKind::Parameter { index: None, .. }
        )
    }
}

mod available_copies;
pub(crate) use available_copies::AvailableCopies;
pub(crate) mod boolean_producers;
mod coalesce;
mod debug_names;
mod defaults;
mod evaluation;
pub(crate) mod execution;
mod expressions;
mod feasible;
pub(crate) mod fragments;
mod initialization;
mod lift;
mod liveness;
pub(crate) mod local_effects;
mod local_identity;
mod local_updates;
mod redundant_reads;
mod schedule;
pub(crate) mod scheduled;
pub(crate) mod scheduled_liveness;
mod select;
pub(crate) mod type_facts;
mod types;
pub(crate) mod update_computation;
mod values;

pub fn lift(
    class: &ClassFile,
    method: &Method,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
) -> Result<MethodIr> {
    lift_planned(
        class,
        method,
        instructions,
        cfg,
        &MethodContext::default(),
        None,
    )
}

fn lift_planned(
    class: &ClassFile,
    method: &Method,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
    context: &MethodContext<'_>,
    debug_pcs: Option<&[(u32, u32)]>,
) -> Result<MethodIr> {
    let mut raw = lift::lift(class, method, instructions, cfg)?;
    // Preserve original GETSTATIC boundaries before update recovery can replace
    // the read with a compound expression. Dead branches remove their triggers
    // together with the original instruction during feasibility simplification.
    initialization::insert_triggers(&mut raw, context.initialization_sites, instructions, cfg)?;
    // Java requires a static initializer to appear capable of completing
    // normally. Its call guards can be that source-level completion witness
    // even when every actual invocation chooses the throwing path.
    let normal_return = if method.name == "<clinit>" {
        None
    } else {
        context.normal_int_return
    };
    feasible::simplify(&mut raw, instructions, cfg, normal_return);
    redundant_reads::eliminate(&mut raw, class, context.field_access_flags);
    types::refine(&mut raw, context.reference_assignable);
    expressions::recover_casts(&mut raw);
    expressions::recover_updates(&mut raw, class);
    expressions::recover_array_initializers(&mut raw, context.reference_assignable);
    select::recover(&mut raw);
    schedule::schedule(
        raw,
        &method.code.as_ref().unwrap().local_variables,
        debug_pcs,
        context.reference_assignable,
    )
}

/// Convenient front end for clients that do not need the decoded CFG separately.
pub fn decompile_method(class: &ClassFile, method: &Method) -> Result<MethodIr> {
    decompile_method_with_initialization(class, method, &[])
}

pub fn decompile_method_with_initialization(
    class: &ClassFile,
    method: &Method,
    sites: &[InitializationSite],
) -> Result<MethodIr> {
    decompile_method_with_context(
        class,
        method,
        &MethodContext {
            initialization_sites: sites,
            ..MethodContext::default()
        },
    )
}

pub fn decompile_method_with_context(
    class: &ClassFile,
    method: &Method,
    context: &MethodContext<'_>,
) -> Result<MethodIr> {
    let code = method
        .code
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("method has no Code attribute"))?;
    let prepared = crate::cfg::PreparedCode::new(code)?;
    lift_planned(
        class,
        method,
        &prepared.instructions,
        &prepared.graph,
        context,
        prepared.debug_ranges(),
    )
}
