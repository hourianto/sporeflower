use super::{
    parser::{Declaration, ParsedSource, Value},
    resolve::{Resolver, fits, integral, qualified},
    *,
};
use crate::hash::{HashMap, HashSet};
use crate::{Result, classfile::JvmType, naming::is_identifier};
use anyhow::{bail, ensure};

impl Resolver<'_, '_> {
    pub(crate) fn domain_declaration(
        &mut self,
        source: &ParsedSource,
        declaration: &Declaration,
        kind: DomainKind,
    ) -> Result<()> {
        let id = self.domain_names[&qualified(&source.package, &declaration.name)];
        let mut values = Vec::new();
        let mut errors = MappingDiagnostics::default();
        for member in &declaration.members {
            let value = (|| -> Result<DomainValue> {
                ensure!(
                    member.parameters.is_none(),
                    "semantic domains may only declare constants"
                );
                ensure!(
                    is_identifier(&member.name),
                    "invalid semantic constant name {}",
                    member.name
                );
                ensure!(
                    member.original.is_none(),
                    "synthetic constants do not use was comments"
                );
                let initializer = member.initializer.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "synthetic semantic constant {} requires an initializer",
                        member.name
                    )
                })?;
                ensure!(
                    member.ty.dimensions == 0,
                    "semantic constant cannot be an array"
                );
                let (descriptor, value) = if kind == DomainKind::String {
                    ensure!(
                        ["String", "java.lang.String"].contains(&member.ty.name.as_str()),
                        "string domains require String constants"
                    );
                    let Value::String(value) = initializer else {
                        bail!("string domain requires literal string constant");
                    };
                    (
                        "Ljava/lang/String;".into(),
                        ConstantValue::String(value.clone()),
                    )
                } else {
                    ensure!(
                        matches!(
                            kind,
                            DomainKind::Value | DomainKind::Flags | DomainKind::Slots
                        ),
                        "packed and numeric domains cannot declare constants"
                    );
                    let ty = match member.ty.name.as_str() {
                        "byte" => JvmType::Byte,
                        "short" => JvmType::Short,
                        "char" => JvmType::Char,
                        "int" => JvmType::Int,
                        "long" => JvmType::Long,
                        _ => bail!("semantic constants require an integral type"),
                    };
                    ensure!(
                        kind != DomainKind::Slots || ty != JvmType::Long,
                        "slot domain constants cannot be long"
                    );
                    let Value::Integer(value) = initializer else {
                        bail!("semantic constant must be an integral literal");
                    };
                    ensure!(
                        fits(&ty, *value),
                        "semantic constant {} value {value} does not fit {}",
                        member.name,
                        member.ty.name
                    );
                    (ty.descriptor(), ConstantValue::Integral(*value))
                };
                let mut slot_domain = None;
                for annotation in &member.annotations {
                    ensure!(
                        annotation.name == "SlotValue" && kind == DomainKind::Slots,
                        "@{} is only valid on a slot domain constant",
                        annotation.name
                    );
                    annotation.allowed(&["value"])?;
                    let domain = self.domain_id(source, annotation.class("value")?)?;
                    ensure!(
                        slot_domain.replace(domain).is_none(),
                        "duplicate @SlotValue"
                    );
                    ensure!(
                        self.domain(domain).kind != DomainKind::Slots,
                        "@SlotValue requires a scalar domain"
                    );
                }
                Ok(DomainValue {
                    name: member.name.clone(),
                    descriptor,
                    value,
                    source: None,
                    slot_domain,
                })
            })()
            .map_err(|error| member.origin.attach(error));
            match value {
                Ok(value) => values.push((value, member.origin.clone())),
                Err(error) => errors.push(error),
            }
        }
        errors.finish()?;
        let mut masks = Vec::new();
        let mut fields = Vec::new();
        let mut format = None;
        for annotation in &declaration.annotations {
            (|| -> Result<()> {
                match annotation.name.as_str() {
                    "ValueDomain" | "SlotDomain" | "PackedDomain" | "StringDomain" => {
                        annotation.allowed(&[])?
                    }
                    "FlagDomain" => {
                        annotation.allowed(&["exclusiveMasks"])?;
                        if let Some(value) = annotation.args.get("exclusiveMasks") {
                            let values = if let Value::Array(values) = value {
                                values.as_slice()
                            } else {
                                std::slice::from_ref(value)
                            };
                            for value in values {
                                let Value::Integer(value) = value else {
                                    bail!("exclusiveMasks requires integral literals");
                                };
                                masks.push(*value);
                            }
                        }
                    }
                    "NumericDomain" => {
                        annotation.allowed(&["format", "fractionBits", "divisor", "unit"])?;
                        let unit = annotation.optional_string("unit")?;
                        if let Some(unit) = &unit {
                            ensure!(
                                !unit.contains("*/") && !unit.contains(['\n', '\r']),
                                "numeric unit must be a safe single-line comment label"
                            );
                        }
                        format = Some(match annotation.string("format")? {
                            color @ ("rgb" | "argb") => {
                                ensure!(
                                    unit.is_none()
                                        && !annotation.args.contains_key("fractionBits")
                                        && !annotation.args.contains_key("divisor"),
                                    "{color} does not accept scale metadata"
                                );
                                if color == "rgb" {
                                    NumberFormat::Rgb
                                } else {
                                    NumberFormat::Argb
                                }
                            }
                            "fixed" => {
                                ensure!(
                                    !annotation.args.contains_key("divisor"),
                                    "fixed format uses fractionBits, not divisor"
                                );
                                let fraction_bits =
                                    annotation.nonnegative("fractionBits", Some(0))?;
                                ensure!(fraction_bits <= 62, "fractionBits must be in 0..62");
                                NumberFormat::Fixed {
                                    fraction_bits: fraction_bits as u8,
                                    unit,
                                }
                            }
                            "scaled" => {
                                ensure!(
                                    !annotation.args.contains_key("fractionBits"),
                                    "scaled format uses divisor, not fractionBits"
                                );
                                let divisor = annotation.integer("divisor", None)?;
                                ensure!(divisor > 0, "scaled divisor must be positive");
                                NumberFormat::Scaled { divisor, unit }
                            }
                            value => bail!("unknown numeric format {value}"),
                        });
                    }
                    "BitField" => {
                        ensure!(
                            matches!(
                                kind,
                                DomainKind::Packed | DomainKind::Value | DomainKind::Flags
                            ),
                            "@BitField requires packed, value or flag domain"
                        );
                        annotation.allowed(&[
                            "name",
                            "value",
                            "shift",
                            "bits",
                            "signed",
                            "selectorMask",
                            "selectorValue",
                        ])?;
                        let domain = annotation
                            .args
                            .get("value")
                            .map(|_| self.domain_id(source, annotation.class("value")?))
                            .transpose()?;
                        if let Some(domain) = domain {
                            ensure!(
                                matches!(
                                    self.domain(domain).kind,
                                    DomainKind::Value
                                        | DomainKind::Flags
                                        | DomainKind::Packed
                                        | DomainKind::Numeric
                                ),
                                "bit-field value requires numeric domain"
                            );
                        }
                        let name = annotation.optional_string("name")?;
                        if let Some(name) = &name {
                            ensure!(
                                is_identifier(name),
                                "bit-field name must be a Java identifier"
                            );
                        }
                        ensure!(
                            domain.is_some() || name.is_some(),
                            "@BitField requires name or value"
                        );
                        let shift = annotation.nonnegative("shift", Some(0))?;
                        let bits = annotation.nonnegative("bits", None)?;
                        ensure!(
                            shift < 64 && (1..=64).contains(&bits) && shift + bits <= 64,
                            "invalid packed bit range"
                        );
                        let selector_mask = annotation.integer("selectorMask", Some(0))?;
                        let selector_value = annotation.integer("selectorValue", Some(0))?;
                        ensure!(
                            selector_value & selector_mask == selector_value,
                            "selectorValue exceeds selectorMask"
                        );
                        fields.push(BitField {
                            domain,
                            name,
                            shift: shift as u8,
                            bits: bits as u8,
                            signed: annotation.boolean("signed", false)?,
                            selector_mask,
                            selector_value,
                        });
                    }
                    name => bail!("unknown domain annotation @{name}"),
                }
                Ok(())
            })()
            .map_err(|error| annotation.origin.attach(error))?;
        }
        let domain = &mut self.domains[id.0];
        domain.values = values;
        domain.exclusive_masks = masks;
        domain.bit_fields = fields;
        domain.format = format;
        Ok(())
    }

    pub(crate) fn scalar_type(&self, domain: DomainId, ty: &JvmType) -> Result<()> {
        let domain = self.domain(domain);
        ensure!(
            domain.kind != DomainKind::Slots,
            "slot domains cannot bind scalar values"
        );
        if domain.kind == DomainKind::String {
            ensure!(
                matches!(ty,JvmType::Object(name) if ["java/lang/String","java/lang/Object"].contains(&name.as_str())),
                "string domain requires String or Object"
            );
        } else {
            ensure!(
                integral(ty)
                    || matches!(ty,JvmType::Object(name) if ["java/lang/Byte","java/lang/Short","java/lang/Character","java/lang/Integer","java/lang/Long","java/lang/Object"].contains(&name.as_str())),
                "numeric domain requires integral, boxed integral or Object value"
            );
            if integral(ty) {
                let width = match ty {
                    JvmType::Byte => 8,
                    JvmType::Short | JvmType::Char => 16,
                    JvmType::Long => 64,
                    _ => 32,
                };
                ensure!(
                    domain
                        .bit_fields
                        .iter()
                        .all(|field| field.shift as u16 + field.bits as u16 <= width),
                    "packed fields exceed target storage width"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn validate_domains(&self) -> Result<()> {
        let mut errors = MappingDiagnostics::default();
        for domain in &self.domains {
            let result = (|| -> Result<()> {
                let mut occupied = 0i64;
                for &mask in &domain.exclusive_masks {
                    ensure!(
                        mask != 0 && occupied & mask == 0,
                        "exclusive masks must be nonzero and disjoint in {}",
                        domain.owner
                    );
                    occupied |= mask;
                }
                let mut names = HashSet::default();
                let mut values = HashMap::default();
                let mut name_origins = HashMap::default();
                for (value, origin) in &domain.values {
                    if let Some(previous) = values.insert(&value.value, origin) {
                        let message = match &value.value {
                            ConstantValue::Integral(integer) => {
                                format!("duplicate semantic value {integer} in {}", domain.owner)
                            }
                            ConstantValue::String(_) => {
                                format!("duplicate semantic string value in {}", domain.owner)
                            }
                        };
                        bail!(
                            MappingDiagnostic::with_origins(
                                message,
                                [previous.clone(), origin.clone()]
                            )
                            .hint("give each domain constant a distinct value")
                        );
                    }
                    if value.source.is_none() {
                        if let Some(previous) = name_origins.insert(&value.name, origin) {
                            bail!(
                                MappingDiagnostic::with_origins(
                                    format!(
                                        "duplicate synthetic constant name {}.{}",
                                        domain.owner, value.name
                                    ),
                                    [previous.clone(), origin.clone()]
                                )
                                .hint("give each generated constant a distinct name")
                            );
                        }
                        names.insert(value.name.clone());
                    }
                    match &value.value {
                        ConstantValue::Integral(integer) => {
                            ensure!(
                                matches!(
                                    domain.kind,
                                    DomainKind::Value | DomainKind::Flags | DomainKind::Slots
                                ),
                                "domain {} cannot contain integral constants",
                                domain.owner
                            );
                            let ty = JvmType::parse(&value.descriptor)?;
                            ensure!(
                                fits(&ty, *integer)
                                    && (domain.kind != DomainKind::Slots || ty != JvmType::Long),
                                "invalid constant storage type in {}",
                                domain.owner
                            );
                        }
                        ConstantValue::String(_) => {
                            ensure!(
                                domain.kind == DomainKind::String,
                                "domain {} cannot contain String constants",
                                domain.owner
                            );
                        }
                    }
                }
                for (index, field) in domain.bit_fields.iter().enumerate() {
                    let field_mask = bit_mask(field.bits) << field.shift;
                    for other in &domain.bit_fields[..index] {
                        let overlap = field_mask & (bit_mask(other.bits) << other.shift) != 0;
                        let exclusive = (field.selector_value ^ other.selector_value)
                            & field.selector_mask
                            & other.selector_mask
                            != 0;
                        ensure!(
                            !overlap || exclusive,
                            "overlapping packed fields need mutually exclusive selectors in {}",
                            domain.owner
                        );
                        if field.name.is_some() && field.name == other.name {
                            ensure!(
                                field.shift == other.shift
                                    && field.bits == other.bits
                                    && field.signed == other.signed,
                                "named packed field has conflicting layouts in {}",
                                domain.owner
                            );
                        }
                    }
                    if let Some(id) = field.domain {
                        for (value, _) in &self.domain(id).values {
                            if let ConstantValue::Integral(value) = value.value {
                                let fits = if field.bits == 64 {
                                    true
                                } else if field.signed {
                                    let edge = 1i64 << (field.bits - 1);
                                    value >= -edge && value < edge
                                } else {
                                    value >= 0 && (value as u64) <= bit_mask(field.bits)
                                };
                                ensure!(
                                    fits,
                                    "domain constant {value} exceeds packed field range in {}",
                                    domain.owner
                                );
                            }
                        }
                    }
                }
                let mut field_names = HashSet::default();
                for field in &domain.bit_fields {
                    if let Some(name) = &field.name {
                        if !field_names.insert(name) {
                            continue;
                        }
                        for suffix in ["MASK", "VALUE_MASK", "SHIFT"] {
                            let generated = format!("{}_{suffix}", constant_prefix(name));
                            ensure!(
                                names.insert(generated.clone()),
                                "generated bit-field constant collision {}.{generated}",
                                domain.owner
                            );
                        }
                    }
                }
                Ok(())
            })()
            .map_err(|error| domain.origin.attach(error));
            if let Err(error) = result {
                errors.push(error);
            }
        }
        errors.finish()?;
        let mut done = HashSet::default();
        let mut active = HashSet::default();
        fn visit(
            resolver: &Resolver<'_, '_>,
            id: DomainId,
            done: &mut HashSet<DomainId>,
            active: &mut HashSet<DomainId>,
        ) -> Result<()> {
            ensure!(
                !active.contains(&id),
                "cyclic packed domain {}",
                resolver.domain(id).owner
            );
            if !done.insert(id) {
                return Ok(());
            }
            active.insert(id);
            for field in &resolver.domain(id).bit_fields {
                if let Some(domain) = field.domain {
                    visit(resolver, domain, done, active)?;
                }
            }
            active.remove(&id);
            Ok(())
        }
        for i in 0..self.domains.len() {
            visit(self, DomainId(i), &mut done, &mut active)
                .map_err(|error| self.domains[i].origin.attach(error))?;
        }
        Ok(())
    }
}

fn bit_mask(bits: u8) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// Naming rule used for the three generated physical-layout constants.
pub fn constant_prefix(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<_> = name.chars().collect();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_uppercase()
            && index > 0
            && (chars[index - 1].is_lowercase()
                || chars[index - 1].is_ascii_digit()
                || chars[index - 1].is_uppercase()
                    && chars.get(index + 1).is_some_and(|next| next.is_lowercase()))
        {
            out.push('_');
        }
        out.extend(ch.to_uppercase());
    }
    out
}
