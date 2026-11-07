//! Preserve package-relative resource names after class relocation. Absolute
//! paths retain their ordinary invocation, including receiver and path effects.
use super::{literals, string_concat};
use crate::classfile::JvmType;
use crate::ir::{Literal, MethodIr, ValueId, ValueKind};
use crate::{naming::NamePlan, program::Program};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub(super) fn needs_relocation(program: &Program<'_>, ir: &MethodIr, path: ValueId) -> bool {
    if ir.values[path].local.is_none()
        && matches!(ir.values[path].kind, ValueKind::Literal(Literal::Null))
    {
        return false;
    }
    !absolute_path(program, ir, path, 0)
}

/// This is a fact about the successful result, never an expression rewrite.
/// Inspect only inline definitions: a physical local may have later writes.
/// Reuse concatenation recognition so escaped or shared builders stay opaque.
fn absolute_path(program: &Program<'_>, ir: &MethodIr, value: ValueId, depth: usize) -> bool {
    if depth > 64 || ir.values[value].local.is_some() {
        return false;
    }
    match &ir.values[value].kind {
        ValueKind::Literal(Literal::String(units)) => units.first() == Some(&u16::from(b'/')),
        ValueKind::Alias(value)
        | ValueKind::Cast { value, .. }
        | ValueKind::InlineAssign { value, .. } => absolute_path(program, ir, *value, depth + 1),
        ValueKind::Select {
            then_value,
            else_value,
            ..
        } => {
            absolute_path(program, ir, *then_value, depth + 1)
                && absolute_path(program, ir, *else_value, depth + 1)
        }
        _ => string_concat::recognize(program, ir, value).is_some_and(|concat| {
            let first = &concat.parts[0];
            match &first.ty {
                JvmType::Object(name) if name == "java/lang/String" => {
                    absolute_path(program, ir, first.value, depth + 1)
                }
                JvmType::Char => {
                    ir.values[first.value].local.is_none()
                        && matches!(
                            ir.values[first.value].kind,
                            ValueKind::Literal(Literal::Int(value)) if value == i32::from(b'/')
                        )
                }
                _ => false,
            }
        }),
    }
}

pub(super) fn helper_source(name: &str, names: &NamePlan) -> String {
    let mut source = format!(
        "private static java.io.InputStream {name}(java.lang.Class owner, java.lang.String path) {{\n"
    );
    source.push_str(
        "    if (path == null || path.startsWith(\"/\")) return owner.getResourceAsStream(path);\n",
    );
    source.push_str("    java.lang.String name = owner.getName();\n    int array = 0;\n    while (name.charAt(array) == '[') array++;\n    if (array != 0 && name.charAt(array) == 'L') name = name.substring(array + 1, name.length() - 1);\n");
    source.push_str("    int separator = name.lastIndexOf('.');\n    java.lang.String packageName = separator < 0 ? \"\" : name.substring(0, separator);\n");
    let mut originals = BTreeMap::<&str, BTreeSet<&str>>::new();
    fn package(name: &str) -> &str {
        name.rsplit_once('/').map_or("", |(package, _)| package)
    }
    let mut classes: Vec<_> = names.class_mappings().collect();
    classes.sort_unstable();
    for &(original, emitted) in &classes {
        originals
            .entry(package(emitted))
            .or_default()
            .insert(package(original));
    }
    let quoted = |text: &str| literals::string_literal(&text.encode_utf16().collect::<Vec<_>>());
    let mut emit = |original: &str, condition: String| {
        let prefix = if original.is_empty() {
            "/".into()
        } else {
            format!("/{original}/")
        };
        writeln!(
            source,
            "    if ({condition}) return owner.getResourceAsStream({} + path);",
            quoted(&prefix)
        )
        .unwrap();
    };
    // Whole-package relocation stays compact. A merged destination must use
    // class identity: two classes in the same new package can still load their
    // resources from different original directories.
    for (original, emitted) in names.package_moves() {
        if originals[emitted.as_str()].len() == 1 {
            emit(
                original,
                format!("packageName.equals({})", quoted(&emitted.replace('/', "."))),
            );
        }
    }
    for (original, emitted) in classes {
        if package(original) != package(emitted) && originals[package(emitted)].len() > 1 {
            emit(
                package(original),
                format!("name.equals({})", quoted(&emitted.replace('/', "."))),
            );
        }
    }
    source.push_str("    return owner.getResourceAsStream(path);\n}");
    source
}
