//! Semantic presentation binds constant references before Java syntax is printed.
//! Every replacement remains a Java constant expression with the original value
//! and numeric width; inferred facts never depend on printed local names.
use super::{
    context::EmissionContext,
    literals,
    syntax::{Expr, Operator, Prefix},
};
use crate::{
    Result,
    classfile::JvmType,
    ir::Literal,
    mappings::{
        BitField, ConstantValue, Domain, DomainId, DomainKind, DomainValue, MappingPlan,
        NumberFormat,
    },
    naming::NamePlan,
    program::{MemberId, Program},
};
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub(super) struct ConstantExpression {
    pub expression: Expr,
    /// String expressions retain the literal's reference type.
    pub numeric_type: Option<&'static JvmType>,
}

#[derive(Clone, Copy)]
pub(super) enum Scope<'a> {
    Body(&'a dyn Fn(&str) -> bool),
    Field(MemberId),
}

impl Scope<'_> {
    fn unqualified(self, cx: &EmissionContext<'_, '_>, source: MemberId) -> bool {
        source.class == cx.types.owner
            && match self {
                Self::Body(shadowed) => !shadowed(cx.types.names.field_name(source)),
                // ConstantValue declarations retain their relative order. All
                // precede recovered initializers and static blocks, whose body
                // bindings only need the local-name check above.
                Self::Field(current) => source.index < current.index,
            }
    }
}

pub(super) fn literal(
    cx: &EmissionContext<'_, '_>,
    domains: &[DomainId],
    literal: &Literal,
    scope: Scope<'_>,
) -> Option<ConstantExpression> {
    let mappings = cx.mappings?;
    let mut domains = domains.iter().copied().collect::<BTreeSet<_>>();
    let unique = domains.len() == 1;
    let domain = mappings.domain(domains.pop_first()?);
    match literal {
        Literal::String(units) if unique => {
            let text = String::from_utf16(units).ok()?;
            let constant = domain.values.iter().find(|constant| {
                constant.value == ConstantValue::String(text.clone())
                    && accessible(cx, domain, constant, scope)
            })?;
            Some(ConstantExpression {
                expression: reference(cx, domain, constant, scope),
                numeric_type: None,
            })
        }
        Literal::Int(value) => {
            integral(cx, domain, domains, unique, i64::from(*value), false, scope)
        }
        Literal::Long(value) => integral(cx, domain, domains, unique, *value, true, scope),
        _ => None,
    }
}

fn integral(
    cx: &EmissionContext<'_, '_>,
    domain: &Domain,
    remaining: BTreeSet<DomainId>,
    unique: bool,
    value: i64,
    wide: bool,
    scope: Scope<'_>,
) -> Option<ConstantExpression> {
    if let Some(format) = &domain.format {
        let mut format = format;
        for other in remaining {
            let other = cx.mappings?.domain(other).format.as_ref()?;
            match (format, other) {
                (NumberFormat::Rgb, NumberFormat::Argb) => format = other,
                (NumberFormat::Argb, NumberFormat::Rgb) => {}
                _ if format == other => {}
                _ => return None,
            }
        }
        return formatted(format, value, wide).map(|expression| ConstantExpression {
            expression,
            numeric_type: Some(if wide { &JvmType::Long } else { &JvmType::Int }),
        });
    }
    if !unique {
        return None;
    }
    let available = domain
        .values
        .iter()
        .filter(|constant| accessible(cx, domain, constant, scope))
        .collect::<Vec<_>>();
    if let Some(constant) = available
        .iter()
        .find(|constant| constant.value == ConstantValue::Integral(value))
    {
        return Some(preserve_width(
            reference(cx, domain, constant, scope),
            primitive(&constant.descriptor)?,
            wide,
        ));
    }
    if domain.kind != DomainKind::Flags {
        return None;
    }
    let mask = if wide { u64::MAX } else { u32::MAX as u64 };
    let target = value as u64 & mask;
    if target == 0 || target == mask {
        return None;
    }
    let positive = cover(&available, target, mask, |bits| {
        domain.exclusive_masks.iter().all(|exclusive| {
            let selected = bits & *exclusive as u64 & mask;
            selected == 0 || selected == target & *exclusive as u64
        })
    });
    let negative = cover(&available, !target & mask, mask, |bits| {
        domain
            .exclusive_masks
            .iter()
            .all(|exclusive| bits & *exclusive as u64 & mask == 0)
    });
    let complement = !negative.0.is_empty()
        && negative.1 == 0
        && (positive.0.is_empty()
            || negative.0.len() < positive.0.len() + usize::from(positive.1 != 0));
    let (constants, residual) = if complement { negative } else { positive };
    if constants.is_empty() {
        return None;
    }
    let mut terms = constants
        .into_iter()
        .map(|constant| {
            // Every term has the expression's original width, including negative
            // narrow constants whose usual Java promotion performs sign extension.
            preserve_width(
                reference(cx, domain, constant, scope),
                primitive(&constant.descriptor).unwrap(),
                wide,
            )
            .expression
        })
        .collect::<Vec<_>>();
    if residual != 0 {
        terms.push(Expr::atom(if wide {
            format!("{}L", residual as i64)
        } else {
            (residual as i32).to_string()
        }));
    }
    let mut expression = terms.remove(0);
    for term in terms {
        expression = Expr::binary(Operator::Or, expression, term);
    }
    if complement {
        expression = Expr::prefix(Prefix::Complement, expression);
    }
    Some(ConstantExpression {
        expression,
        numeric_type: Some(if wide { &JvmType::Long } else { &JvmType::Int }),
    })
}

fn cover<'a>(
    constants: &[&'a DomainValue],
    target: u64,
    mask: u64,
    allow: impl Fn(u64) -> bool,
) -> (Vec<&'a DomainValue>, u64) {
    let mut candidates = constants
        .iter()
        .filter_map(|constant| {
            let ConstantValue::Integral(value) = constant.value else {
                return None;
            };
            let bits = value as u64 & mask;
            (bits != 0 && bits & !target == 0 && allow(bits)).then_some((*constant, bits))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(a, am), (b, bm)| {
        bm.count_ones()
            .cmp(&am.count_ones())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.descriptor.cmp(&b.descriptor))
    });
    let mut residual = target;
    let mut chosen = Vec::new();
    for (constant, bits) in candidates {
        if bits & residual != 0 {
            chosen.push(constant);
            residual &= !bits;
        }
    }
    (chosen, residual)
}

fn primitive(descriptor: &str) -> Option<&'static JvmType> {
    Some(match descriptor {
        "B" => &JvmType::Byte,
        "S" => &JvmType::Short,
        "C" => &JvmType::Char,
        "I" => &JvmType::Int,
        "J" => &JvmType::Long,
        _ => return None,
    })
}

fn preserve_width(expression: Expr, ty: &'static JvmType, wide: bool) -> ConstantExpression {
    let target = if wide { &JvmType::Long } else { &JvmType::Int };
    if (ty == &JvmType::Long) != wide {
        ConstantExpression {
            expression: Expr::cast(if wide { "long" } else { "int" }, expression),
            numeric_type: Some(target),
        }
    } else {
        ConstantExpression {
            expression,
            numeric_type: Some(ty),
        }
    }
}

fn accessible(
    cx: &EmissionContext<'_, '_>,
    domain: &Domain,
    constant: &DomainValue,
    scope: Scope<'_>,
) -> bool {
    let owner = constant.source.map_or(domain.owner.as_str(), |source| {
        cx.program.class(source.class).name.as_str()
    });
    // Named replacements must remain Java constant expressions. A runtime
    // qualifier fallback cannot be used in a case label or narrowing context.
    if !constant
        .source
        .is_some_and(|source| scope.unqualified(cx, source))
        && !cx.types.can_name_qualifier(owner)
    {
        return false;
    }
    let package = |name: &str| {
        name.rsplit_once('/')
            .map_or("", |(package, _)| package)
            .to_owned()
    };
    let current = cx.types.owner;
    let current_package = package(cx.types.names.class_name(current));
    let Some(source) = constant.source else {
        return !package(&domain.owner).is_empty() || current_package.is_empty();
    };
    if matches!(scope, Scope::Field(current) if source == current) {
        return false;
    }
    if source.class == current {
        return true;
    }
    let same_package = package(cx.types.names.class_name(source.class)) == current_package;
    let class = cx.program.class(source.class);
    let field = cx.program.field(source);
    // A protected field in another package needs a qualifying subclass and may
    // be hidden there. Suppress that optional name rather than alter its binding.
    (class.access_flags & 1 != 0 || same_package)
        && field.access_flags & 2 == 0
        && (field.access_flags & 1 != 0 || same_package)
}

fn reference(
    cx: &EmissionContext<'_, '_>,
    domain: &Domain,
    constant: &DomainValue,
    scope: Scope<'_>,
) -> Expr {
    if let Some(source) = constant.source
        && scope.unqualified(cx, source)
    {
        return Expr::atom(cx.types.names.field_name(source));
    }
    let (owner, name) = match constant.source {
        Some(source) => (
            cx.program.class(source.class).name.as_str(),
            cx.types.names.field_name(source),
        ),
        None => (domain.owner.as_str(), constant.name.as_str()),
    };
    Expr::atom(
        cx.types
            .named_qualifier(owner)
            .expect("constant owner was checked by accessible"),
    )
    .member(name)
}

fn formatted(format: &NumberFormat, value: i64, wide: bool) -> Option<Expr> {
    if matches!(
        format,
        NumberFormat::Fixed { .. } | NumberFormat::Scaled { .. }
    ) && (value == 0
        || if wide {
            value == i64::MIN || value == i64::MAX
        } else {
            value == i32::MIN as i64 || value == i32::MAX as i64
        })
    {
        return None;
    }
    let suffix = if wide { "L" } else { "" };
    let mut comment = None;
    let token = match format {
        NumberFormat::Scaled { divisor, unit } => {
            comment = Some(format!(
                "/{divisor}: {}{}",
                rational(value, *divisor as u64),
                unit_text(unit)
            ));
            format!("{value}{suffix}")
        }
        _ => {
            let digits = match format {
                NumberFormat::Rgb => 6,
                NumberFormat::Argb => 8,
                _ => 1,
            };
            let bits = if wide {
                value as u64
            } else {
                value as u32 as u64
            };
            if let NumberFormat::Fixed {
                fraction_bits,
                unit,
            } = format
            {
                comment = Some(format!(
                    "Q{fraction_bits}: {}{}",
                    rational(value, 1u64 << fraction_bits),
                    unit_text(unit)
                ));
            }
            format!("0x{bits:0digits$X}{suffix}")
        }
    };
    let expression = Expr::atom(token);
    Some(if let Some(comment) = comment {
        expression.commented(comment)
    } else {
        expression
    })
}

fn unit_text(unit: &Option<String>) -> String {
    unit.as_ref().map_or_else(String::new, |unit| {
        format!(
            " {}",
            unit.replace("*/", "* /")
                .replace(['\r', '\n'], " ")
                .replace('\\', "/")
        )
    })
}

/// Exact decimal for terminating rationals, reduced fractions for all others.
fn rational(numerator: i64, denominator: u64) -> String {
    let mut n = numerator.unsigned_abs();
    let mut d = denominator;
    let (mut a, mut b) = (n, d);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    n /= a;
    d /= a;
    let mut reduced = d;
    while reduced.is_multiple_of(2) {
        reduced /= 2;
    }
    while reduced.is_multiple_of(5) {
        reduced /= 5;
    }
    let sign = if numerator < 0 { "-" } else { "" };
    if reduced != 1 {
        return format!("{sign}{n}/{d}");
    }
    let mut result = format!("{sign}{}", n / d);
    let mut remainder = n % d;
    if remainder != 0 {
        result.push('.');
    }
    while remainder != 0 {
        let digit = remainder as u128 * 10;
        result.push(char::from(b'0' + (digit / d as u128) as u8));
        remainder = (digit % d as u128) as u64;
    }
    result
}

pub(super) fn bit_field_name(field: &BitField, suffix: &str) -> Option<String> {
    let name = field.name.as_ref()?;
    let prefix = crate::mappings::constant_prefix(name);
    Some(format!("{prefix}_{suffix}"))
}

pub(super) fn packed(
    cx: &EmissionContext<'_, '_>,
    constant: &crate::semantics::PackedConstant,
    literal: &Literal,
) -> Option<ConstantExpression> {
    use crate::semantics::PackedPart;
    let domain = cx.mappings?.domain(constant.domain);
    if !domain.owner.contains('/') && cx.types.names.class_name(cx.types.owner).contains('/') {
        return None;
    }
    let field = &domain.bit_fields[constant.field];
    let (suffix, wide) = match constant.part {
        PackedPart::Mask | PackedPart::ClearMask => {
            ("MASK", u16::from(field.shift) + u16::from(field.bits) >= 32)
        }
        PackedPart::ValueMask => ("VALUE_MASK", field.bits >= 32),
        PackedPart::Shift => ("SHIFT", false),
    };
    let low = if field.bits == 64 {
        u64::MAX
    } else {
        (1u64 << field.bits) - 1
    };
    let value = match constant.part {
        PackedPart::Mask => low << field.shift,
        PackedPart::ClearMask => !(low << field.shift),
        PackedPart::ValueMask => low,
        PackedPart::Shift => u64::from(field.shift),
    };
    if !match literal {
        Literal::Int(original) => value as i32 == *original,
        Literal::Long(original) => value as i64 == *original,
        _ => false,
    } {
        return None;
    }
    let expression =
        Expr::atom(cx.types.named_qualifier(&domain.owner)?).member(bit_field_name(field, suffix)?);
    let mut result = preserve_width(
        expression,
        if wide { &JvmType::Long } else { &JvmType::Int },
        matches!(literal, Literal::Long(_)),
    );
    if constant.part == PackedPart::ClearMask {
        // Complement at the original literal's width, after the reference's
        // generated int/long type has been adjusted to that width.
        result.expression = Expr::prefix(Prefix::Complement, result.expression);
    }
    Some(result)
}

pub(super) fn offset(
    cx: &EmissionContext<'_, '_>,
    constant: &crate::semantics::OffsetConstant,
    original: &Literal,
    scope: Scope<'_>,
) -> Option<ConstantExpression> {
    let wide = matches!(original, Literal::Long(_));
    let value = constant
        .slot
        .wrapping_mul(constant.multiplier)
        .wrapping_add(constant.addend);
    if !match original {
        Literal::Int(original) => value as i32 == *original,
        Literal::Long(original) => value == *original,
        _ => false,
    } {
        return None;
    }
    let slot = if wide {
        Literal::Long(constant.slot)
    } else {
        Literal::Int(i32::try_from(constant.slot).ok()?)
    };
    let named = literal(cx, &[constant.domain], &slot, scope)?;
    let mut expression = named.expression;
    let number = |value: i64| {
        Expr::atom(if wide {
            format!("{value}L")
        } else {
            (value as i32).to_string()
        })
    };
    if constant.multiplier != 1 {
        expression = Expr::binary(Operator::Multiply, expression, number(constant.multiplier));
    }
    if constant.addend != 0 {
        expression = Expr::binary(Operator::Add, expression, number(constant.addend));
    }
    Some(ConstantExpression {
        expression,
        numeric_type: Some(if wide { &JvmType::Long } else { &JvmType::Int }),
    })
}

/// Generated interfaces are source artifacts only; real bytecode constants keep
/// their existing declaration and never create an otherwise empty interface.
pub fn domain_sources(
    mappings: &MappingPlan,
    _program: &Program<'_>,
    _names: &NamePlan,
) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    for domain in mappings.domains() {
        if !domain.generated() {
            continue;
        }
        let (package, simple) = domain
            .owner
            .rsplit_once('/')
            .unwrap_or(("", domain.owner.as_str()));
        let mut source = String::new();
        if !package.is_empty() {
            writeln!(source, "package {};\n", package.replace('/', ".")).unwrap();
        }
        writeln!(source, "public interface {simple} {{").unwrap();
        for constant in &domain.values {
            if constant.source.is_some() {
                continue;
            }
            let (ty, value) = match &constant.value {
                ConstantValue::String(value) => (
                    "java.lang.String",
                    literals::string_literal(&value.encode_utf16().collect::<Vec<_>>()),
                ),
                ConstantValue::Integral(value) => {
                    let ty = match constant.descriptor.as_str() {
                        "B" => "byte",
                        "S" => "short",
                        "C" => "char",
                        "I" => "int",
                        "J" => "long",
                        _ => unreachable!("validated integral domain"),
                    };
                    (
                        ty,
                        if ty == "long" {
                            format!("{value}L")
                        } else {
                            value.to_string()
                        },
                    )
                }
            };
            writeln!(source, "    {ty} {} = {value};", constant.name).unwrap();
        }
        let mut emitted = BTreeSet::new();
        for field in &domain.bit_fields {
            let low = if field.bits == 64 {
                u64::MAX
            } else {
                (1u64 << field.bits) - 1
            };
            for (suffix, value, wide) in [
                (
                    "MASK",
                    low << field.shift,
                    u16::from(field.shift) + u16::from(field.bits) >= 32,
                ),
                ("VALUE_MASK", low, field.bits >= 32),
                ("SHIFT", u64::from(field.shift), false),
            ] {
                if let Some(name) = bit_field_name(field, suffix)
                    && emitted.insert(name.clone())
                {
                    let ty = if wide { "long" } else { "int" };
                    let value = if wide {
                        format!("{}L", value as i64)
                    } else {
                        (value as i32).to_string()
                    };
                    writeln!(source, "    {ty} {name} = {value};").unwrap();
                }
            }
        }
        source.push_str("}\n");
        result.push((domain.owner.clone(), source));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_formats_preserve_bits_and_exact_fraction() {
        assert_eq!(
            formatted(&NumberFormat::Rgb, -1, false).unwrap().render(),
            "0xFFFFFFFF"
        );
        assert_eq!(
            formatted(&NumberFormat::Argb, 255, true).unwrap().render(),
            "0x000000FFL"
        );
        assert_eq!(
            formatted(
                &NumberFormat::Fixed {
                    fraction_bits: 8,
                    unit: None
                },
                384,
                false
            )
            .unwrap()
            .render(),
            "0x180 /* Q8: 1.5 */"
        );
        assert_eq!(
            formatted(
                &NumberFormat::Scaled {
                    divisor: 3,
                    unit: Some("px".into())
                },
                -2,
                true
            )
            .unwrap()
            .render(),
            "-2L /* /3: -2/3 px */"
        );
        assert_eq!(rational(i64::MIN, 1u64 << 62), "-2");
        assert_eq!(
            rational(1, 1u64 << 62),
            "0.00000000000000000021684043449710088680149056017398834228515625"
        );
        assert!(
            formatted(
                &NumberFormat::Fixed {
                    fraction_bits: 8,
                    unit: None
                },
                i32::MAX as i64,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn constant_width_is_preserved_without_widening_narrow_names() {
        let narrow = preserve_width(Expr::atom("Kinds.SMALL"), &JvmType::Byte, false);
        assert_eq!(narrow.expression.render(), "Kinds.SMALL");
        assert_eq!(narrow.numeric_type, Some(&JvmType::Byte));
        let int = preserve_width(Expr::atom("Kinds.WIDE"), &JvmType::Long, false);
        assert_eq!(int.expression.render(), "(int) Kinds.WIDE");
        let long = preserve_width(Expr::atom("Kinds.SMALL"), &JvmType::Byte, true);
        assert_eq!(long.expression.render(), "(long) Kinds.SMALL");
    }

    #[test]
    fn numeric_comments_do_not_change_grouping_or_create_java_tokens() {
        let value = formatted(
            &NumberFormat::Scaled {
                divisor: 3,
                unit: Some("*/\\u000a".into()),
            },
            -2,
            false,
        )
        .unwrap();
        assert_eq!(
            Expr::prefix(Prefix::Negate, value).render(),
            "-(-2 /* /3: -2/3 * //u000a */)"
        );
        assert_eq!(
            Expr::prefix(
                Prefix::Complement,
                Expr::binary(Operator::Or, Expr::atom("A"), Expr::atom("B"))
            )
            .render(),
            "~(A | B)"
        );
    }
}
