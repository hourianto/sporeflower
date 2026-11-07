//! Optional authoring aids. References are counted from original bytecode;
//! coverage measures authored mappings, never automatically repaired names.
use anyhow::{Context, Result};
use sporeflower::{
    bytecode::{Operand, decode},
    mappings::{MappingPlan, Target},
    naming::NamePlan,
    program::{MemberId, Program},
};
use std::{collections::HashMap, fmt::Write, fs, path::Path};

pub const NAMES: [&str; 3] = ["coverage.md", "usage-priority.tsv", "semantic-summary.md"];

pub fn write(
    root: &Path,
    program: &Program<'_>,
    names: &NamePlan,
    maps: &MappingPlan,
) -> Result<()> {
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let mut coverage = String::from(
        "# Mapping coverage\n\nAuthored names; automatic repairs do not count. `@AlreadyMapped` classes are excluded.\n\n| Original class | Source class | Class named | Fields named | Methods named |\n|---|---|---:|---:|---:|\n",
    );
    let mut usage =
        String::from("uses\tkind\towner\tname\tdescriptor\tsource_owner\tsource_name\tmapped\n");
    let mut counts = HashMap::<(bool, MemberId), usize>::new();
    for class in program.classes().iter().take(program.own_count()) {
        for method in &class.methods {
            let Some(code) = &method.code else { continue };
            // Malformed methods already have a decompilation diagnostic. Do not
            // let an optional priority report hide the rest of the source.
            let Ok(instructions) = decode(&code.bytes) else {
                continue;
            };
            for instruction in instructions {
                if !(0xb2..=0xb9).contains(&instruction.opcode) {
                    continue;
                }
                let index = match instruction.operand {
                    Operand::Constant(index) | Operand::InterfaceCall { index, .. } => index,
                    _ => continue,
                };
                let Ok(reference) = class.constant_pool.member_ref(index) else {
                    continue;
                };
                let field = instruction.opcode <= 0xb5;
                let member = if field {
                    program.resolve_field(reference.owner, reference.name, reference.descriptor)
                } else {
                    program.resolve_method(reference.owner, reference.name, reference.descriptor)
                };
                if let Some(member) = member.filter(|member| program.is_own(member.class)) {
                    *counts.entry((field, member)).or_default() += 1;
                }
            }
        }
    }
    let mut members = Vec::new();
    for class_id in 0..program.own_count() {
        if maps.ignores(class_id) {
            continue;
        }
        let class = program.class(class_id);
        let mut field_count = 0;
        let mut method_count = 0;
        for field in [true, false] {
            let len = if field {
                class.fields.len()
            } else {
                class.methods.len()
            };
            for index in 0..len {
                let id = MemberId {
                    class: class_id,
                    index,
                };
                let (original, descriptor, emitted, mapped) = if field {
                    let member = program.field(id);
                    (
                        &member.name,
                        &member.descriptor,
                        names.field_name(id),
                        maps.names.fields.contains_key(&id),
                    )
                } else {
                    let member = program.method(id);
                    if member.name.starts_with('<') {
                        continue;
                    }
                    (
                        &member.name,
                        &member.descriptor,
                        names.method_name(id),
                        maps.names.methods.contains_key(&id),
                    )
                };
                if mapped {
                    if field {
                        field_count += 1;
                    } else {
                        method_count += 1;
                    }
                }
                members.push((
                    counts.get(&(field, id)).copied().unwrap_or(0),
                    field,
                    class_id,
                    original,
                    descriptor,
                    emitted,
                    mapped,
                ));
            }
        }
        let methods = class
            .methods
            .iter()
            .filter(|method| !method.name.starts_with('<'))
            .count();
        writeln!(
            coverage,
            "| `{}` | `{}` | {} | {}/{} | {}/{} |",
            markdown(&class.name),
            markdown(names.class_name(class_id)),
            maps.names.classes.contains_key(&class_id),
            field_count,
            class.fields.len(),
            method_count,
            methods
        )?;
    }
    members.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| (a.2, a.1, a.3, a.4).cmp(&(b.2, b.1, b.3, b.4)))
    });
    for (count, field, owner, original, descriptor, emitted, mapped) in members {
        writeln!(
            usage,
            "{count}\t{}\t{}\t{}\t{}\t{}\t{}\t{mapped}",
            if field { "field" } else { "method" },
            tsv(&program.class(owner).name),
            tsv(original),
            tsv(descriptor),
            tsv(names.class_name(owner)),
            tsv(emitted)
        )?;
    }
    let mut semantics = String::from(
        "# Semantic mappings\n\nResolved contracts in original JVM identities. These describe available meanings, not substitution counts.\n\n",
    );
    writeln!(
        semantics,
        "{} domains, {} member contracts, {} scoped call contracts.\n",
        maps.domains().len(),
        maps.bindings().len(),
        maps.calls().len()
    )?;
    for domain in maps.domains() {
        writeln!(
            semantics,
            "- `{}`: {:?}, {} constants, {} packed fields",
            markdown(&domain.owner.replace('/', ".")),
            domain.kind,
            domain.values.len(),
            domain.bit_fields.len()
        )?;
    }
    semantics.push_str("\n## Member contracts\n\n");
    let mut bindings: Vec<_> = maps.bindings().iter().collect();
    bindings.sort_by_key(|(target, _)| **target);
    for (target, contract) in bindings {
        let (member, role, field) = match *target {
            Target::Field(member) => (member, "field".into(), true),
            Target::Return(member) => (member, "return".into(), false),
            Target::Parameter { method, index } => (method, format!("parameter {index}"), false),
        };
        let identity = identity(program, member, field);
        writeln!(
            semantics,
            "- `{}` {role}: {}",
            markdown(&identity),
            describe_contract(maps, contract)
        )?;
    }
    if !maps.calls().is_empty() {
        semantics.push_str("\n## Scoped calls\n\nOffsets are original invoke byte offsets; parameter indexes exclude `this`.\n\n");
        let mut calls: Vec<_> = maps.calls().iter().collect();
        calls.sort_by_key(|(site, _)| **site);
        for (site, domain) in calls {
            let role = site
                .parameter
                .map_or_else(|| "return".into(), |index| format!("parameter {index}"));
            writeln!(
                semantics,
                "- `{}` at {} ({role}): `{}`",
                markdown(&identity(program, site.method, false)),
                site.offset,
                markdown(&maps.domain(*domain).owner.replace('/', "."))
            )?;
        }
    }
    for (name, text) in NAMES.into_iter().zip([coverage, usage, semantics]) {
        let path = root.join(name);
        fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

fn identity(program: &Program<'_>, member: MemberId, field: bool) -> String {
    let (name, descriptor) = if field {
        let member = program.field(member);
        (&member.name, &member.descriptor)
    } else {
        let member = program.method(member);
        (&member.name, &member.descriptor)
    };
    format!(
        "{}.{}{}",
        program.class(member.class).name,
        name,
        descriptor
    )
}

fn describe_contract(maps: &MappingPlan, contract: &sporeflower::mappings::Contract) -> String {
    let mut parts = Vec::new();
    let domain = |id| format!("`{}`", markdown(&maps.domain(id).owner.replace('/', ".")));
    if let Some(leaf) = contract.leaf {
        parts.push(domain(leaf));
    }
    if contract.class_name {
        parts.push("class-name string".into());
    }
    for (dimension, array) in &contract.arrays {
        if let Some(id) = array.index {
            parts.push(format!("dimension {dimension} index {}", domain(id)));
        }
        if let Some(id) = array.slots {
            parts.push(format!("dimension {dimension} slots {}", domain(id)));
        }
        if let Some(layout) = &array.records {
            parts.push(format!(
                "dimension {dimension} {} {} (stride {}, header {})",
                if layout.planes { "planes" } else { "records" },
                domain(layout.domain),
                layout.stride,
                layout.offset
            ));
        }
    }
    if let Some(container) = &contract.container {
        for (role, id) in [
            ("elements", container.elements),
            ("keys", container.keys),
            ("values", container.values),
        ] {
            if let Some(id) = id {
                parts.push(format!("{role} {}", domain(id)));
            }
        }
    }
    if let Some(derived) = &contract.derived {
        use sporeflower::mappings::DerivedDomain;
        match derived {
            DerivedDomain::Parameter(index) => parts.push(format!("from parameter {index}")),
            DerivedDomain::Slot {
                parameter,
                dimension,
                slot,
            } => parts.push(format!(
                "from parameter {parameter}, dimension {dimension}, slot {slot}"
            )),
            DerivedDomain::Conditional(cases) => {
                for case in cases {
                    let condition = if let Some(value) = case.equals {
                        format!("== {value}")
                    } else if let Some(value) = case.not_equals {
                        format!("!= {value}")
                    } else {
                        "otherwise".into()
                    };
                    parts.push(format!(
                        "{} when parameter {} {condition}",
                        domain(case.domain),
                        case.parameter
                    ));
                }
            }
        }
    }
    parts.join("; ")
}

fn tsv(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\0', "\\0")
}
fn markdown(text: &str) -> String {
    tsv(text).replace('|', "&#124;").replace('`', "&#96;")
}
