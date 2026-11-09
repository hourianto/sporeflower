//! Recognize string concatenation through the original append descriptors.
//! The scheduler materializes multiply-used observable values. An entirely
//! unmapped builder spine therefore certifies fresh, nonescaping identity.

use crate::classfile::JvmType;
use crate::ir::{InvokeKind, Literal, MethodIr, ValueId, ValueKind};
use crate::program::Program;

const BUFFER: &str = "java/lang/StringBuffer";
const BUILDER: &str = "java/lang/StringBuilder";
const STRING: &str = "java/lang/String";

pub(super) struct Part {
    pub value: ValueId,
    pub ty: JvmType,
}

pub(super) struct Concatenation {
    pub parts: Vec<Part>,
    pub empty_prefix: bool,
    /// A constant expression would be interned by javac, while toString returns
    /// a fresh String. Keep that observable distinction even for literal chains.
    pub fresh_wrapper: bool,
}

pub(super) fn recognize(
    program: &Program<'_>,
    ir: &MethodIr,
    root: ValueId,
) -> Option<Concatenation> {
    let (builder, mut current) = builder_root(ir, root)?;
    if [builder, STRING]
        .into_iter()
        .any(|class| program.id(class).is_some_and(|id| program.is_own(id)))
    {
        return None;
    }
    let mut parts = Vec::new();
    for _ in 0..ir.values.len() {
        if ir.values[current].local.is_some() {
            return None;
        }
        match &ir.values[current].kind {
            ValueKind::Call {
                kind: InvokeKind::Virtual,
                method,
                receiver: Some(receiver),
                args,
            } => {
                if method.owner != builder || method.name != "append" || args.len() != 1 {
                    return None;
                }
                let ty = append_type(builder, &method.descriptor)?;
                parts.push(Part { value: args[0], ty });
                current = *receiver;
            }
            ValueKind::New {
                class,
                descriptor,
                args,
                ..
            } if class == builder => {
                match (descriptor.as_str(), args.as_slice()) {
                    ("()V", []) => (),
                    ("(Ljava/lang/String;)V", [value]) => parts.push(string_seed(ir, *value)?),
                    // Capacity expressions and nullable String constructors
                    // have checks before the first append. Retain them intact.
                    _ => return None,
                }
                if parts.is_empty() {
                    return None;
                }
                parts.reverse();
                let first = &parts[0];
                // A primitive followed immediately by String already selects
                // Java's concatenation operator. Its conversion is pure, unlike
                // Object.toString, which must precede evaluation of the next
                // append argument and therefore needs the empty-string prefix.
                let primitive_string_pair = !first.ty.is_reference()
                    && parts
                        .get(1)
                        .is_some_and(|part| part.ty == JvmType::Object(STRING.into()));
                let empty_prefix = parts.len() == 1
                    || first.ty != JvmType::Object(STRING.into()) && !primitive_string_pair
                    || matches!(
                        ir.values[first.value].kind,
                        ValueKind::Literal(Literal::Null)
                    );
                let fresh_wrapper = parts
                    .iter()
                    .all(|part| may_be_constant(program, ir, part.value, 0));
                return Some(Concatenation {
                    parts,
                    empty_prefix,
                    fresh_wrapper,
                });
            }
            _ => return None,
        }
    }
    None
}

fn builder_root(ir: &MethodIr, root: ValueId) -> Option<(&str, ValueId)> {
    let mut current = root;
    for _ in 0..ir.values.len() {
        if current != root && ir.values[current].local.is_some() {
            return None;
        }
        match &ir.values[current].kind {
            ValueKind::Call {
                kind: InvokeKind::Static,
                method,
                receiver: None,
                args,
            } if method.owner == STRING
                && method.name == "valueOf"
                && method.descriptor == "(Ljava/lang/Object;)Ljava/lang/String;"
                && args.len() == 1 =>
            {
                // Accept conversions only if the final spine proves a fresh,
                // nonescaping builder. Its nonnull text makes every wrapper
                // transparent, including valueOf(valueOf(builder)).
                current = args[0];
            }
            ValueKind::Call {
                kind: InvokeKind::Virtual,
                method,
                receiver: Some(receiver),
                args,
            } if matches!(method.owner.as_str(), BUFFER | BUILDER)
                && method.name == "toString"
                && method.descriptor == "()Ljava/lang/String;"
                && args.is_empty() =>
            {
                return Some((method.owner.as_str(), *receiver));
            }
            _ if current != root => {
                let JvmType::Object(builder) = &ir.values[current].ty else {
                    return None;
                };
                return matches!(builder.as_str(), BUFFER | BUILDER)
                    .then_some((builder.as_str(), current));
            }
            _ => return None,
        }
    }
    None
}

fn append_type(builder: &str, descriptor: &str) -> Option<JvmType> {
    let (argument, result) = descriptor.strip_prefix('(')?.split_once(')')?;
    if result.strip_prefix('L')?.strip_suffix(';')? != builder {
        return None;
    }
    Some(match argument {
        "Z" => JvmType::Boolean,
        "C" => JvmType::Char,
        "I" => JvmType::Int,
        "J" => JvmType::Long,
        "F" => JvmType::Float,
        "D" => JvmType::Double,
        "Ljava/lang/String;" => JvmType::Object(STRING.into()),
        "Ljava/lang/Object;" => JvmType::Object("java/lang/Object".into()),
        // char[] appends characters and throws on null; Java string
        // conversion of an array has different contents/behavior.
        _ => return None,
    })
}

/// Older javac versions seed concatenation with String.valueOf rather than an
/// empty builder. Preserve conversion boundaries while recognizing the forms
/// whose result cannot be null. An arbitrary Object.toString may return null,
/// in which case the StringBuffer(String) constructor must still throw.
fn string_seed(ir: &MethodIr, value: ValueId) -> Option<Part> {
    if matches!(
        ir.values[value].kind,
        ValueKind::Literal(Literal::String(_))
    ) {
        return Some(Part {
            value,
            ty: JvmType::Object(STRING.into()),
        });
    }
    if ir.values[value].local.is_some() {
        return None;
    }
    let ValueKind::Call {
        kind: InvokeKind::Static,
        method,
        receiver: None,
        args,
    } = &ir.values[value].kind
    else {
        return None;
    };
    if method.owner != STRING || method.name != "valueOf" || args.len() != 1 {
        return None;
    }
    let ty = match method.descriptor.as_str() {
        "(Z)Ljava/lang/String;" => JvmType::Boolean,
        "(C)Ljava/lang/String;" => JvmType::Char,
        "(I)Ljava/lang/String;" => JvmType::Int,
        "(J)Ljava/lang/String;" => JvmType::Long,
        "(F)Ljava/lang/String;" => JvmType::Float,
        "(D)Ljava/lang/String;" => JvmType::Double,
        "(Ljava/lang/Object;)Ljava/lang/String;"
            if ir.values[args[0]].ty == JvmType::Object(STRING.into()) =>
        {
            JvmType::Object(STRING.into())
        }
        "([C)Ljava/lang/String;" => {
            return Some(Part {
                value,
                ty: JvmType::Object(STRING.into()),
            });
        }
        _ => return None,
    };
    Some(Part { value: args[0], ty })
}

fn may_be_constant(program: &Program<'_>, ir: &MethodIr, value: ValueId, depth: usize) -> bool {
    if depth > 64 || ir.values[value].local.is_some() {
        return false;
    }
    match &ir.values[value].kind {
        ValueKind::Literal(literal) => !matches!(literal, Literal::Class(_)),
        ValueKind::Unary { value, .. }
        | ValueKind::Cast { value, .. }
        | ValueKind::Alias(value) => may_be_constant(program, ir, *value, depth + 1),
        ValueKind::Binary { left, right, .. } => {
            may_be_constant(program, ir, *left, depth + 1)
                && may_be_constant(program, ir, *right, depth + 1)
        }
        ValueKind::Select {
            condition,
            then_value,
            else_value,
        } => [condition.left, condition.right, *then_value, *else_value]
            .into_iter()
            .all(|value| may_be_constant(program, ir, value, depth + 1)),
        ValueKind::Field {
            field,
            receiver: None,
        } => program
            .resolve_field(&field.owner, &field.name, &field.descriptor)
            .is_some_and(|member| {
                // An uncertain field can still become a Java constant in its
                // declaring class. Keep the fresh wrapper for either outcome.
                !matches!(
                    super::fields::constant_field(program, member),
                    super::fields::ConstantField::Nonconstant
                )
            }),
        _ => false,
    }
}
