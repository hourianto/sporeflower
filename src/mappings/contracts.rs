//! Declaration contracts are validated before they become inference facts.
use super::{
    parser::{Annotation, ParsedSource},
    resolve::{
        Binding, BindingKind, Resolver, fits, insert_binding, integral, leaf_type, validate_cases,
    },
    *,
};
use crate::hash::HashSet;
use crate::{
    Result,
    classfile::{JvmType, MethodDescriptor},
};
use anyhow::{bail, ensure};

impl Resolver<'_, '_> {
    pub(crate) fn bind(
        &mut self,
        source: &ParsedSource,
        target: Target,
        ty: &JvmType,
        annotations: &[Annotation],
        method: Option<&MethodDescriptor>,
    ) -> Result<()> {
        let mut contract = Contract::default();
        let mut seen = HashSet::default();
        for annotation in annotations {
            (|| -> Result<()> {
            let name = annotation.name.as_str();
            if ["DomainValue", "CallDomain"].contains(&name) {
                ensure!(
                    matches!(
                        (name, target),
                        ("DomainValue", Target::Field(_)) | ("CallDomain", Target::Return(_))
                    ),
                    "@{name} is not valid on this target"
                );
                return Ok(());
            }
            if !["IndexDomain", "Slots", "Records", "Planes", "DomainWhen"].contains(&name) {
                ensure!(seen.insert(name), "duplicate @{name}");
            }
            match name {
                "Domain" | "Flags" => {
                    annotation.allowed(&["value"])?;
                    let id = self.domain_id(source, annotation.class("value")?)?;
                    let kind = self.domain(id).kind;
                    ensure!(
                        if name == "Flags" {
                            kind == DomainKind::Flags
                        } else {
                            matches!(
                                kind,
                                DomainKind::Value
                                    | DomainKind::Packed
                                    | DomainKind::Numeric
                                    | DomainKind::String
                            )
                        },
                        "@{name} has incompatible domain kind"
                    );
                    self.scalar_type(id, leaf_type(ty))?;
                    ensure!(
                        contract.leaf.replace(id).is_none(),
                        "a target cannot have both @Domain and @Flags"
                    );
                }
                "ClassName" => {
                    annotation.allowed(&[])?;
                    ensure!(
                        matches!(ty,JvmType::Object(name) if name == "java/lang/String")
                            || matches!(ty,JvmType::Array(inner) if matches!(inner.as_ref(),JvmType::Object(name) if name == "java/lang/String")),
                        "@ClassName requires String or String[]"
                    );
                    contract.class_name = true;
                }
                "IndexDomain" | "Slots" | "Records" | "Planes" => {
                    annotation.allowed(if name == "Records" || name == "Planes" {
                        &["value", "dimension", "stride", "offset"]
                    } else {
                        &["value", "dimension"]
                    })?;
                    let dimensions = ty.array_dimensions();
                    ensure!(dimensions > 0, "@{name} requires an array");
                    let dimension = annotation.nonnegative(
                        "dimension",
                        if name != "IndexDomain" && dimensions == 1 {
                            Some(0)
                        } else {
                            None
                        },
                    )?;
                    ensure!(
                        dimension < dimensions,
                        "@{name} dimension is outside the array"
                    );
                    let id = self.domain_id(source, annotation.class("value")?)?;
                    ensure!(
                        self.domain(id).kind
                            == if name == "IndexDomain" {
                                DomainKind::Value
                            } else {
                                DomainKind::Slots
                            },
                        "@{name} has incompatible domain kind"
                    );
                    let entry = contract.arrays.entry(dimension).or_default();
                    if name == "IndexDomain" {
                        ensure!(
                            entry.index.replace(id).is_none(),
                            "duplicate index dimension"
                        );
                    } else if name == "Slots" {
                        ensure!(
                            entry.slots.replace(id).is_none(),
                            "duplicate slot dimension"
                        );
                    } else {
                        let stride = annotation.nonnegative("stride", None)?;
                        let offset = annotation.nonnegative("offset", Some(0))?;
                        ensure!(
                            stride > 0
                                && i32::try_from(stride).is_ok()
                                && i32::try_from(offset).is_ok(),
                            "invalid record stride/offset"
                        );
                        ensure!(
                            entry
                                .records
                                .replace(RecordLayout {
                                    domain: id,
                                    stride: stride as u32,
                                    offset: offset as u32,
                                    planes: name == "Planes"
                                })
                                .is_none(),
                            "duplicate record/plane dimension"
                        );
                    }
                    ensure!(
                        entry.index.is_none() || entry.slots.is_none() && entry.records.is_none(),
                        "index domain conflicts with slot/record layout"
                    );
                }
                "Elements" | "Keys" | "Values" => {
                    annotation.allowed(&["value"])?;
                    let id = self.domain_id(source, annotation.class("value")?)?;
                    ensure!(
                        self.domain(id).kind != DomainKind::Slots,
                        "container requires a scalar domain"
                    );
                    let expected = if name == "Elements" {
                        &["java/util/Vector", "java/util/Enumeration"][..]
                    } else {
                        &["java/util/Hashtable"][..]
                    };
                    ensure!(
                        matches!(ty,JvmType::Object(owner) if expected.contains(&owner.as_str())),
                        "@{name} requires a supported J2ME container"
                    );
                    let container = contract.container.get_or_insert_with(Default::default);
                    match name {
                        "Elements" => container.elements = Some(id),
                        "Keys" => container.keys = Some(id),
                        _ => container.values = Some(id),
                    }
                }
                "DomainFromParameter" => {
                    annotation.allowed(&["value"])?;
                    ensure!(
                        matches!(target, Target::Return(_)),
                        "@DomainFromParameter only binds a return"
                    );
                    let index = annotation.nonnegative("value", None)?;
                    let descriptor = method.unwrap();
                    ensure!(
                        integral(ty) && descriptor.parameters.get(index).is_some_and(integral),
                        "@DomainFromParameter requires integral return and argument"
                    );
                    ensure!(
                        contract
                            .derived
                            .replace(DerivedDomain::Parameter(index))
                            .is_none(),
                        "conflicting derived domains"
                    );
                }
                "DomainFromSlot" => {
                    annotation.allowed(&["parameter", "dimension", "slot"])?;
                    let descriptor = method
                        .ok_or_else(|| anyhow::anyhow!("@DomainFromSlot requires method target"))?;
                    ensure!(integral(ty), "@DomainFromSlot target must be integral");
                    let parameter = annotation.nonnegative("parameter", None)?;
                    let source_ty = descriptor
                        .parameters
                        .get(parameter)
                        .ok_or_else(|| anyhow::anyhow!("@DomainFromSlot parameter out of range"))?;
                    let dimensions = source_ty.array_dimensions();
                    ensure!(
                        dimensions > 0 && integral(leaf_type(source_ty)),
                        "@DomainFromSlot source requires integral array"
                    );
                    let dimension = annotation
                        .nonnegative("dimension", if dimensions == 1 { Some(0) } else { None })?;
                    ensure!(
                        dimension + 1 == dimensions,
                        "@DomainFromSlot must select innermost dimension"
                    );
                    let slot = annotation.nonnegative("slot", None)?;
                    ensure!(i32::try_from(slot).is_ok(), "slot out of range");
                    ensure!(
                        contract
                            .derived
                            .replace(DerivedDomain::Slot {
                                parameter,
                                dimension,
                                slot: slot as u32
                            })
                            .is_none(),
                        "conflicting derived domains"
                    );
                }
                "DomainWhen" => {
                    annotation.allowed(&[
                        "value",
                        "parameter",
                        "equals",
                        "notEquals",
                        "otherwise",
                    ])?;
                    let descriptor = method
                        .ok_or_else(|| anyhow::anyhow!("@DomainWhen requires method target"))?;
                    let parameter = annotation.nonnegative("parameter", None)?;
                    let selector = descriptor
                        .parameters
                        .get(parameter)
                        .ok_or_else(|| anyhow::anyhow!("@DomainWhen selector out of range"))?;
                    ensure!(integral(selector), "@DomainWhen selector must be integral");
                    let domain = self.domain_id(source, annotation.class("value")?)?;
                    self.scalar_type(domain, ty)?;
                    let equals = annotation.optional_integer("equals")?;
                    let not_equals = annotation.optional_integer("notEquals")?;
                    let otherwise = annotation.boolean("otherwise", false)?;
                    ensure!(
                        usize::from(equals.is_some())
                            + usize::from(not_equals.is_some())
                            + usize::from(otherwise)
                            == 1,
                        "@DomainWhen requires exactly one selector case"
                    );
                    for value in equals.iter().chain(not_equals.iter()) {
                        ensure!(
                            fits(selector, *value),
                            "@DomainWhen case does not fit selector"
                        );
                    }
                    let cases = contract
                        .derived
                        .get_or_insert_with(|| DerivedDomain::Conditional(Vec::new()));
                    let DerivedDomain::Conditional(cases) = cases else {
                        bail!("conflicting derived domains");
                    };
                    cases.push(ConditionalDomain {
                        parameter,
                        domain,
                        equals,
                        not_equals,
                        otherwise,
                    });
                }
                _ => bail!("unknown mapping annotation @{name}"),
            }
            Ok(())
            })().map_err(|error| annotation.origin.attach(error))?;
        }
        ensure!(
            !contract.class_name
                || contract.leaf.is_none()
                    && contract.container.is_none()
                    && contract.derived.is_none(),
            "@ClassName conflicts with another contract"
        );
        ensure!(
            contract.derived.is_none()
                || contract.leaf.is_none()
                    && contract.arrays.is_empty()
                    && contract.container.is_none(),
            "derived domains conflict with fixed contracts"
        );
        if let Some(DerivedDomain::Conditional(cases)) = &contract.derived {
            validate_cases(cases)?;
        }
        for dimension in contract.arrays.values() {
            if let Some(layout) = &dimension.records {
                for (value, _) in &self.domain(layout.domain).values {
                    let ConstantValue::Integral(value) = value.value else {
                        unreachable!()
                    };
                    ensure!(
                        value >= 0 && (layout.planes || value < layout.stride as i64),
                        "record slot outside stride"
                    );
                }
                if let Some(slots) = dimension.slots {
                    for (value, _) in &self.domain(slots).values {
                        let ConstantValue::Integral(value) = value.value else {
                            unreachable!()
                        };
                        ensure!(
                            value >= 0 && value < layout.offset as i64,
                            "header slot outside record header"
                        );
                    }
                }
            }
            for domain in dimension
                .slots
                .iter()
                .chain(dimension.records.iter().map(|layout| &layout.domain))
            {
                for (value, _) in &self.domain(*domain).values {
                    if let Some(domain) = value.slot_domain {
                        self.scalar_type(domain, leaf_type(ty))?;
                    }
                }
            }
        }
        if !contract.is_empty() {
            let origin = &annotations[0].origin;
            insert_binding(
                &mut self.bindings,
                &mut self.diagnostics,
                target,
                Binding::new(source, origin, contract),
                BindingKind::Contract,
            )?;
        }
        Ok(())
    }
}
