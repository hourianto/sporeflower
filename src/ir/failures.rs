//! Intrinsic failures of scheduled JVM operations. A proof can refine a field
//! access with resolution and receiver facts, or compare specific exceptions
//! against ordered catch handlers. Operands contribute their own failures.
use super::{
    BinaryOp, JvmType, Literal, Member, MethodIr, Statement, Terminator, UpdateTarget, Value,
    ValueId, ValueKind, scheduled,
};

pub(crate) const NULL_POINTER: &str = "java/lang/NullPointerException";
const ARRAY_STORE: &str = "java/lang/ArrayStoreException";
const ARRAY_BOUNDS: &str = "java/lang/ArrayIndexOutOfBoundsException";
const ARITHMETIC: &str = "java/lang/ArithmeticException";
pub(crate) const RUNTIME_EXCEPTIONS: [&str; 4] =
    [NULL_POINTER, ARRAY_STORE, ARRAY_BOUNDS, ARITHMETIC];

#[derive(Clone, Copy)]
pub(crate) struct FieldAccess<'a> {
    pub field: &'a Member,
    pub receiver: Option<ValueId>,
    pub write: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Failure<'a> {
    Field(FieldAccess<'a>),
    Runtime(&'static str),
    Resolution,
    Unknown,
}

impl Failure<'_> {
    /// Source recovery ignores failures confined to literal and type resolution.
    pub(crate) fn in_source(self) -> bool {
        !matches!(self, Self::Resolution)
    }
}

fn array<'a>(reference_store: bool, throws: &mut impl FnMut(Failure<'a>) -> bool) -> bool {
    throws(Failure::Runtime(NULL_POINTER))
        || throws(Failure::Runtime(ARRAY_BOUNDS))
        || reference_store && throws(Failure::Runtime(ARRAY_STORE))
}

impl BinaryOp {
    pub(super) fn division_may_throw(self, ty: &JvmType) -> bool {
        matches!(self, Self::Div | Self::Rem) && matches!(ty, JvmType::Int | JvmType::Long)
    }
}

fn value<'a>(value: &'a Value, throws: &mut impl FnMut(Failure<'a>) -> bool) -> bool {
    match &value.kind {
        ValueKind::Literal(Literal::String(_) | Literal::Class(_))
        | ValueKind::InstanceOf { .. } => throws(Failure::Resolution),
        ValueKind::Field { field, receiver } => throws(Failure::Field(FieldAccess {
            field,
            receiver: *receiver,
            write: false,
        })),
        ValueKind::Update { target, .. } | ValueKind::CompoundUpdate { target, .. } => {
            if let ValueKind::CompoundUpdate {
                op, operation_type, ..
            } = &value.kind
                && op.division_may_throw(operation_type)
                && throws(Failure::Runtime(ARITHMETIC))
            {
                return true;
            }
            match target {
                UpdateTarget::Field { field, receiver } => throws(Failure::Field(FieldAccess {
                    field,
                    receiver: *receiver,
                    write: true,
                })),
                UpdateTarget::Array { element_type, .. } => {
                    array(element_type.is_reference(), throws)
                }
            }
        }
        ValueKind::ArrayLoad { .. } => array(false, throws),
        ValueKind::ArrayLength { .. } => throws(Failure::Runtime(NULL_POINTER)),
        ValueKind::Binary { op, .. } => {
            op.division_may_throw(&value.ty) && throws(Failure::Runtime(ARITHMETIC))
        }
        ValueKind::InlineAssign { .. } | ValueKind::LocalUpdate { .. } => false,
        other => other.is_observable() && throws(Failure::Unknown),
    }
}

impl Value {
    /// Whether this operation itself can transfer to a source handler. Reading
    /// a materialized result and evaluating operands are handled by the caller.
    pub fn may_throw(&self) -> bool {
        value(self, &mut Failure::in_source)
    }
}

pub(crate) fn statement<'a>(
    ir: &'a MethodIr,
    stmt: &'a Statement,
    mut throws: impl FnMut(Failure<'a>) -> bool,
) -> bool {
    (match stmt {
        Statement::Assign { .. } | Statement::Eval(_) => false,
        Statement::PutField {
            field, receiver, ..
        } => throws(Failure::Field(FieldAccess {
            field,
            receiver: *receiver,
            write: true,
        })),
        Statement::ArrayStore { element_type, .. } => {
            array(element_type.is_reference(), &mut throws)
        }
        Statement::ConstructorCall { .. } | Statement::Monitor { .. } => throws(Failure::Unknown),
    }) || scheduled::statement_values(ir, stmt)
        .operations()
        .any(|id| value(&ir.values[id], &mut throws))
}

pub(crate) fn terminator<'a>(
    ir: &'a MethodIr,
    term: &Terminator,
    mut throws: impl FnMut(Failure<'a>) -> bool,
) -> bool {
    (matches!(term, Terminator::Throw { .. }) && throws(Failure::Unknown))
        || scheduled::terminator_values(ir, term)
            .operations()
            .any(|id| value(&ir.values[id], &mut throws))
}
