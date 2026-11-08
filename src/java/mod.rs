//! Java source emission from scheduled values and structured control flow.
//! The writer does no bytecode repair and never substitutes a successful body
//! for an unsupported method: failures are explicit in both source and counts.

mod allocations;
mod array_literals;
mod body;
mod boolean_values;
mod boundary;
mod class_names;
mod constructors;
mod context;
mod contracts;
mod conversions;
mod exceptions;
mod expr;
mod field_joins;
mod fields;
mod generated;
mod interfaces;
mod layout;
mod linkage;
mod literals;
mod local_flow;
mod local_names;
mod overloads;
mod prepare;
mod resources;
mod rethrow;
mod scopes;
mod semantic;
mod source;
mod source_locals;
mod source_updates;
mod special;
mod string_concat;
mod syntax;
mod types;

use crate::Result;
use crate::classfile::{Constant, JvmType, MethodDescriptor};
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use anyhow::Context;
use body::CodeWriter;
use context::{BodyKind, BodyOptions, EmissionContext};
use types::TypeNames;

#[derive(Clone, Debug, Default)]
pub struct JavaOptions<'a> {
    pub mappings: Option<&'a crate::mappings::MappingPlan>,
}

pub(crate) use literals::STANDARD_OWNERS as STANDARD_CONSTANT_OWNERS;
pub use semantic::domain_sources;

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecompiledClass {
    pub source: String,
    pub diagnostics: Vec<String>,
    pub total_methods: usize,
    pub failed_methods: usize,
    pub fallback_methods: usize,
}

pub fn decompile_class(
    program: &Program<'_>,
    names: &NamePlan,
    id: ClassId,
    options: &JavaOptions<'_>,
) -> Result<DecompiledClass> {
    let class = program.class(id);
    linkage::validate_class(program, id)?;
    let contracts = contracts::Contracts::build(program, id);
    let generated_delegations = constructors::GeneratedDelegations::build(program, id)?;
    let types = TypeNames::with_methods(
        program,
        names,
        id,
        contracts.missing.iter().map(|method| method.contract),
        &generated_delegations.types(program)?,
        options.mappings,
    );
    let cx = EmissionContext::new(
        program,
        &types,
        special::SpecialPlan::new(program, names, id),
    )
    .with_mappings(options.mappings);
    let linkage = linkage::LinkagePlan::new(program, &types);
    let field_plan = fields::FieldPlan::new(program, &types)?;
    let mut result = DecompiledClass {
        total_methods: class.methods.len(),
        ..Default::default()
    };
    if contracts.uses_cldc_error(program) {
        result.diagnostics.push(format!(
            "{}: missing dispatch contracts use java.lang.Error because the selected CLDC API omits java.lang.AbstractMethodError (CLDC 1.1 section 4.2)", class.name
        ));
    }
    let interface = class.access_flags & 0x0200 != 0;
    let interface_plan = if interface
        && (class.methods.iter().any(|m| m.name == "<clinit>")
            || class
                .fields
                .iter()
                .any(|field| field.constant_value.is_none()))
    {
        Some(
            cx.artifact(|cx| interfaces::InterfacePlan::build(cx, &linkage))
                .map(|artifact| cx.commit(artifact)),
        )
    } else {
        None
    };
    let interface_fields = interface_plan.as_ref().and_then(|plan| plan.as_ref().ok());
    // Semantic constants may introduce imports absent from the original pool.
    // Bind the complete body before spelling its package/import header.
    let mut out = CodeWriter::new(1);
    for (index, method) in class.methods.iter().enumerate() {
        if index > 0 {
            out.line("");
        }
        if method.name == "<clinit>" && interface {
            if interface_fields.is_some() {
                continue;
            }
            result.failed_methods += 1;
            let reason = interface_plan
                .as_ref()
                .and_then(|plan| plan.as_ref().err())
                .map_or_else(
                    || "interface initialization requires source field reconstruction".to_owned(),
                    |error| format!("{error:#}"),
                );
            result.diagnostics.push(format!(
                "{}.{}{}: {reason}",
                class.name, method.name, method.descriptor
            ));
            out.line("// Decompilation failed: interface initializer could not be reconstructed.");
            continue;
        }
        emit_method(
            &cx,
            &linkage,
            &field_plan,
            &generated_delegations,
            index,
            &mut out,
            &mut result,
        )?;
    }
    if !interface && !class.methods.iter().any(|method| method.name == "<init>") {
        constructors::absent(program, &types, &generated_delegations, &mut out)?;
    }
    contracts.emit(program, names, &types, &mut out)?;
    if !interface {
        linkage.emit(&cx, &mut out);
    }
    if let Some(diagnostic) = linkage.diagnostic(&cx) {
        result.diagnostics.push(diagnostic);
    }
    cx.emit_support(&mut out);
    if let Some(helper) = names.initialization_helper(id) {
        out.line("");
        if interface {
            // Java 1.3 has no interface static methods. A nonconstant field
            // read triggers the interface itself (not its superinterfaces).
            let object = types.class("java/lang/Object");
            out.line(format!("{object} {helper} = null;"));
            out.open(format!("final class {helper}$Trigger {{"));
            out.open("public static void initialize() {");
            out.line(format!(
                "{object} ignored = {};",
                types.qualifier(&class.name).member(helper).render()
            ));
            out.close("}");
            out.close("}");
        } else {
            out.line(format!("public static void {helper}() {{}}"));
        }
    }
    if let Some(plan) = interface_fields {
        out.line("");
        out.text.push_str(&plan.helper);
        result.diagnostics.extend(plan.diagnostics.iter().cloned());
        result.fallback_methods += usize::from(plan.fallback);
    }
    for helper in &field_plan.helpers {
        out.line("");
        out.line(helper);
    }
    out.close("}");
    let mut fields = CodeWriter::new(1);
    emit_fields(&cx, &field_plan, interface_fields, &mut fields)?;
    let mut header = CodeWriter::new(0);
    emit_header(&cx, &mut header);
    header.text.push_str(&fields.text);
    header.text.push_str(&out.text);
    result.source = header.text;
    result.diagnostics.extend(cx.diagnostics());
    Ok(result)
}

fn emit_header(cx: &EmissionContext<'_, '_>, out: &mut CodeWriter) {
    let program = cx.program;
    let types = cx.types;
    let names = types.names;
    let id = types.owner;
    let class = program.class(id);
    let interface = class.access_flags & 0x0200 != 0;
    let internal = names.class_name(id);
    let (package, simple) = internal.rsplit_once('/').unwrap_or(("", internal));
    if !package.is_empty() {
        out.line(format!("package {};", package.replace('/', ".")));
        out.line("");
    }
    let imports = types.imports();
    for import in &imports {
        out.line(format!("import {import};"));
    }
    if !imports.is_empty() {
        out.line("");
    }
    let mut declaration = String::new();
    if class.access_flags & 1 != 0 {
        declaration.push_str("public ");
    }
    if !interface {
        if class.access_flags & 0x0010 != 0 {
            declaration.push_str("final ");
        }
        if class.access_flags & 0x0400 != 0 {
            declaration.push_str("abstract ");
        }
    }
    declaration.push_str(if interface { "interface " } else { "class " });
    declaration.push_str(simple);
    if !interface
        && let Some(parent) = class
            .super_name
            .as_deref()
            .filter(|&name| name != "java/lang/Object")
    {
        declaration.push_str(" extends ");
        declaration.push_str(&types.class(parent));
    }
    if !class.interfaces.is_empty() {
        declaration.push_str(if interface {
            " extends "
        } else {
            " implements "
        });
        declaration.push_str(
            &class
                .interfaces
                .iter()
                .map(|name| types.class(name))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    declaration.push_str(" {");
    out.open(declaration);
}

fn emit_fields(
    cx: &EmissionContext<'_, '_>,
    field_plan: &fields::FieldPlan,
    interface_fields: Option<&interfaces::InterfacePlan>,
    out: &mut CodeWriter,
) -> Result<()> {
    let program = cx.program;
    let types = cx.types;
    let names = types.names;
    let id = types.owner;
    let class = program.class(id);
    let interface = class.access_flags & 0x0200 != 0;
    let field_order: Vec<_> = if let Some(plan) = interface_fields {
        class
            .fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| field.constant_value.is_some().then_some(index))
            .chain(plan.fields.iter().map(|(index, _)| *index))
            .collect()
    } else {
        (0..class.fields.len()).collect()
    };
    for index in field_order {
        let field = &class.fields[index];
        let ty = JvmType::parse(&field.descriptor)?;
        let flags = if interface || cx.retains_final(MemberId { class: id, index }) {
            field.access_flags
        } else {
            field.access_flags & !0x0010
        };
        let mut declaration = modifiers(flags, MemberKind::Field, interface);
        declaration.push_str(&types.ty(&ty));
        declaration.push(' ');
        declaration.push_str(names.field_name(MemberId { class: id, index }));
        // The JVM ignores ConstantValue on instance fields. Their original
        // constructor stores must retain their timing and preceding zero reads.
        if let Some(constant) =
            fields::constant_initializer(program, MemberId { class: id, index }, flags)?
        {
            declaration.push_str(" = ");
            let semantic = cx.mappings.and_then(|mappings| {
                let member = MemberId { class: id, index };
                let contract = mappings.contract(crate::mappings::Target::Field(member))?;
                let literal = match class.constant_pool.get(constant).ok()? {
                    Constant::Integer(value) => crate::ir::Literal::Int(*value),
                    Constant::Long(value) => crate::ir::Literal::Long(*value),
                    Constant::String(_) => {
                        crate::ir::Literal::String(class.constant_pool.string_utf16(constant).ok()?)
                    }
                    _ => return None,
                };
                if contract.class_name
                    && let crate::ir::Literal::String(units) = &literal
                {
                    let original = String::from_utf16(units).ok()?;
                    let mapped = expr::relocated_class_name(&original, names)?;
                    return Some(literals::string_literal(
                        &mapped.encode_utf16().collect::<Vec<_>>(),
                    ));
                }
                semantic::literal(cx, &[contract.leaf?], &literal, Some(member))
                    .map(|constant| constant.expression.render())
            });
            declaration.push_str(&match semantic {
                Some(semantic) => semantic,
                None => constant_value(types, class, constant, &ty)?,
            });
        } else if let Some(default) = field_plan.defaults.get(&index) {
            declaration.push_str(" = ");
            declaration.push_str(default);
        } else if let Some(plan) = interface_fields {
            declaration.push_str(" = ");
            declaration.push_str(
                &plan
                    .fields
                    .iter()
                    .find(|(field, _)| *field == index)
                    .expect("planned field")
                    .1,
            );
        } else if interface {
            declaration.push_str(" = ");
            declaration.push_str(body::default_value(&ty));
        }
        declaration.push(';');
        out.line(declaration);
    }
    if let Some(field) = interface_fields.and_then(|plan| plan.trailing_field.as_ref()) {
        out.line(field);
    }
    if !class.fields.is_empty() && !class.methods.is_empty() {
        out.line("");
    }
    Ok(())
}

fn emit_method(
    cx: &EmissionContext<'_, '_>,
    linkage: &linkage::LinkagePlan,
    field_plan: &fields::FieldPlan,
    generated_delegations: &constructors::GeneratedDelegations,
    index: usize,
    out: &mut CodeWriter,
    result: &mut DecompiledClass,
) -> Result<()> {
    let program = cx.program;
    let types = cx.types;
    let id = types.owner;
    let class = program.class(id);
    let method = &class.methods[index];
    let member = MemberId { class: id, index };
    let parameter_names = local_names::parameters(method, member, types)?;
    let body = cx.artifact(|cx| lower_method(cx, linkage, field_plan, member, &parameter_names));
    let mut header = method_header(cx, index, &parameter_names)?;
    let failure = body.as_ref().err().map(|error| {
        result.failed_methods += 1;
        let diagnostic = format!(
            "{}.{}{}: {error:#}",
            class.name, method.name, method.descriptor
        );
        result.diagnostics.push(diagnostic.clone());
        diagnostic
    });
    if method.code.is_none() {
        if failure.is_some() {
            out.line("// Decompilation failed: this declaration conflicts with its external exception contract.");
        }
        out.line(format!("{header};"));
        return Ok(());
    }
    if let Some(helper) = cx.special.helper(MemberId { class: id, index }) {
        header = header.replacen("synchronized ", "", 1);
        out.open(format!("{header} {{"));
        out.line(special::forwarding_body(method, helper, &parameter_names)?);
        out.close("}");
        out.line("");
        header = special::helper_header(
            types,
            method,
            helper,
            program.method_throws(member),
            &parameter_names,
        )?;
    }
    out.open(format!("{header} {{"));
    let mut declarations = String::new();
    match body {
        Ok(artifact) => {
            let rendered = cx.commit(artifact);
            result.fallback_methods += usize::from(rendered.fallback);
            out.text.push_str(&rendered.body);
            declarations = rendered.declarations;
        }
        Err(_) => {
            if method.name == "<init>" {
                out.line(constructors::failure_delegation(
                    program,
                    types,
                    generated_delegations,
                    member,
                )?);
            }
            let diagnostic = failure.expect("recorded method failure");
            let diagnostic: Vec<_> = diagnostic.encode_utf16().collect();
            let guard = if method.name == "<clinit>" {
                "if (true) "
            } else {
                ""
            };
            out.line(format!(
                "{guard}throw new java.lang.Error({});",
                literals::string_literal(&diagnostic)
            ));
        }
    }
    out.close("}");
    if !declarations.is_empty() {
        out.line("");
        out.text.push_str(&declarations);
    }
    Ok(())
}

#[derive(Default)]
struct RenderedMethod {
    body: String,
    declarations: String,
    fallback: bool,
}

fn lower_method(
    cx: &EmissionContext<'_, '_>,
    linkage: &linkage::LinkagePlan,
    fields: &fields::FieldPlan,
    member: MemberId,
    parameter_names: &[String],
) -> Result<RenderedMethod> {
    let program = cx.program;
    let method = program.method(member);
    if let Some(diagnostic) = program.throws_plan().diagnostic(member) {
        anyhow::bail!("{diagnostic}");
    }
    if method.code.is_none() {
        return Ok(RenderedMethod::default());
    }
    let prepared = prepare::PreparedBody::new(cx, linkage, member, parameter_names)?;
    let allocation_declarations = prepared.helpers.render(cx, member)?;
    let prepared = prepared.ir;
    let constructor = method.name == "<init>";
    let mut rendered = if constructor {
        constructors::render(
            cx,
            method,
            &prepared,
            member.index,
            &fields.constructor_defaults(program, member.class, prepared.ir()),
        )?
    } else {
        let kind = if method.name == "<clinit>" {
            BodyKind::Initializer
        } else {
            BodyKind::Method
        };
        let body = cx.render(&prepared, Some(member), BodyOptions::original(kind))?;
        RenderedMethod {
            body: body.text,
            fallback: body.fallback,
            declarations: String::new(),
        }
    };
    rendered.fallback |= allocation_declarations.fallback;
    rendered.declarations = allocation_declarations.text + &rendered.declarations;
    Ok(rendered)
}

fn method_header(
    cx: &EmissionContext<'_, '_>,
    index: usize,
    parameter_names: &[String],
) -> Result<String> {
    let program = cx.program;
    let types = cx.types;
    let names = types.names;
    let owner = types.owner;
    let member = MemberId {
        class: owner,
        index,
    };
    let class = program.class(owner);
    let method = &class.methods[index];
    if method.name == "<clinit>" {
        return Ok("static".into());
    }
    let descriptor = MethodDescriptor::parse(&method.descriptor)?;
    let mut header = modifiers(
        contracts::method_flags(program, member),
        if method.name == "<init>" {
            MemberKind::Constructor
        } else {
            MemberKind::Method
        },
        class.access_flags & 0x0200 != 0,
    );
    if method.name == "<init>" {
        header.push_str(names.class_name(owner).rsplit('/').next().unwrap());
    } else {
        header.push_str(&types.ty(&descriptor.return_type));
        header.push(' ');
        header.push_str(names.method_name(member));
    }
    header.push('(');
    for (index, parameter) in descriptor.parameters.iter().enumerate() {
        if index > 0 {
            header.push_str(", ");
        }
        header.push_str(&types.ty(parameter));
        header.push(' ');
        header.push_str(&parameter_names[index]);
    }
    header.push(')');
    let exceptions = program.method_throws(member);
    if !exceptions.is_empty() {
        header.push_str(" throws ");
        header.push_str(
            &exceptions
                .iter()
                .map(|name| types.class(name))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Ok(header)
}

enum MemberKind {
    Field,
    Method,
    Constructor,
}
fn modifiers(flags: u16, kind: MemberKind, interface: bool) -> String {
    let mut text = String::new();
    if !interface {
        if flags & 0x0001 != 0 {
            text.push_str("public ");
        } else if flags & 0x0002 != 0 {
            text.push_str("private ");
        } else if flags & 0x0004 != 0 {
            text.push_str("protected ");
        }
    }
    if flags & 0x0008 != 0 && !interface {
        text.push_str("static ");
    }
    if flags & 0x0010 != 0 && !interface {
        text.push_str("final ");
    }
    match kind {
        MemberKind::Field => {
            if flags & 0x0040 != 0 {
                text.push_str("volatile ");
            }
            if flags & 0x0080 != 0 {
                text.push_str("transient ");
            }
        }
        MemberKind::Method => {
            if flags & 0x0020 != 0 {
                text.push_str("synchronized ");
            }
            if flags & 0x0100 != 0 {
                text.push_str("native ");
            }
            if flags & 0x0400 != 0 && !interface {
                text.push_str("abstract ");
            }
            if flags & 0x0800 != 0 {
                text.push_str("strictfp ");
            }
        }
        MemberKind::Constructor => {}
    }
    text
}

fn constant_value(
    types: &TypeNames<'_>,
    class: &crate::classfile::ClassFile,
    index: u16,
    ty: &JvmType,
) -> Result<String> {
    let literal = match class.constant_pool.get(index).context("field constant")? {
        Constant::Integer(value) => Some(crate::ir::Literal::Int(*value)),
        Constant::Long(value) => Some(crate::ir::Literal::Long(*value)),
        Constant::Float(value) => Some(crate::ir::Literal::Float(*value)),
        Constant::Double(value) => Some(crate::ir::Literal::Double(*value)),
        _ => None,
    };
    if let Some(literal) = literal
        && let Some(constant) = literals::standard_constant(types, &literal, ty)
    {
        return Ok(constant.render());
    }
    Ok(
        match class.constant_pool.get(index).context("field constant")? {
            Constant::Integer(value) if *ty == JvmType::Boolean => {
                if *value == 0 { "false" } else { "true" }.into()
            }
            Constant::Integer(value) if *ty == JvmType::Char => {
                literals::char_literal(*value as u16)
            }
            Constant::Integer(value) => value.to_string(),
            Constant::Long(value) => format!("{value}L"),
            Constant::Float(value) => literals::float_literal(*value),
            Constant::Double(value) => literals::double_literal(*value),
            Constant::String(_) => {
                literals::string_literal(&class.constant_pool.string_utf16(index)?)
            }
            other => anyhow::bail!("invalid field ConstantValue {other:?}"),
        },
    )
}
