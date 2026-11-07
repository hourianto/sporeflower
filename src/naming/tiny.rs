//! Resolved original-to-source Tiny v2 mappings shared by all frontends.
use crate::{Result, naming::NamePlan, program::Program};

pub fn mapping_text(program: &Program<'_>, names: &NamePlan) -> Result<String> {
    use crate::program::MemberId;
    use std::fmt::Write;
    let mut text = String::from("tiny\t2\t0\toriginal\tsource\n\tescaped-names\n");
    for id in 0..program.own_count() {
        let class = program.class(id);
        writeln!(
            text,
            "c\t{}\t{}",
            tiny_escape(&class.name),
            tiny_escape(names.class_name(id))
        )?;
        for (index, field) in class.fields.iter().enumerate() {
            writeln!(
                text,
                "\tf\t{}\t{}\t{}",
                tiny_escape(&field.descriptor),
                tiny_escape(&field.name),
                tiny_escape(names.field_name(MemberId { class: id, index }))
            )?;
        }
        for (index, method) in class.methods.iter().enumerate() {
            writeln!(
                text,
                "\tm\t{}\t{}\t{}",
                tiny_escape(&method.descriptor),
                tiny_escape(&method.name),
                tiny_escape(names.method_name(MemberId { class: id, index }))
            )?;
            let descriptor = crate::classfile::MethodDescriptor::parse(&method.descriptor)?;
            let mut slot = u16::from(method.access_flags & 0x0008 == 0);
            for (ordinal, ty) in descriptor.parameters.iter().enumerate() {
                if let Some(name) = names.parameter_name(MemberId { class: id, index }, ordinal) {
                    writeln!(text, "\t\tp\t{slot}\t\t{}", tiny_escape(name))?;
                }
                slot += ty.slots();
            }
        }
    }
    Ok(text)
}

fn tiny_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\0', "\\0")
}
