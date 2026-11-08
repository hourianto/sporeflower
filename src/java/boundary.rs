//! Rare JVM callbacks can let a checked exception escape a Java interface that
//! declares none. CLDC Class.newInstance propagates its nullary constructor's
//! exception directly. This bridge retains the exact object, without wrapping.
use super::{body::CodeWriter, source::Plan, types::TypeNames};
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::cell::Cell;

#[derive(Clone)]
pub struct BoundarySupport {
    bridge: String,
    used: Cell<bool>,
}

impl BoundarySupport {
    pub fn new(program: &Program<'_>, names: &NamePlan, owner: ClassId) -> Self {
        let occupied = |candidate: &str| {
            let binary = format!("{}${candidate}", names.class_name(owner));
            program
                .classes()
                .iter()
                .enumerate()
                .any(|(id, _)| names.class_name(id) == binary)
                || std::iter::once(owner)
                    .chain(program.ancestors(owner).iter().copied())
                    .any(|class| {
                        program
                            .class(class)
                            .fields
                            .iter()
                            .enumerate()
                            .any(|(index, _)| {
                                names.field_name(MemberId { class, index }) == candidate
                            })
                    })
        };
        let mut bridge = "$CheckedThrow".to_owned();
        while occupied(&bridge) {
            bridge.push('_');
        }
        Self {
            bridge,
            used: Cell::new(false),
        }
    }

    pub fn plan(
        &self,
        program: &Program<'_>,
        types: &TypeNames<'_>,
        member: MemberId,
        body: &mut Plan,
    ) {
        let boundary = program.throws_plan().boundary(member);
        if boundary.is_empty() {
            return;
        }
        self.used.set(true);
        let mut out = CodeWriter::new(0);
        out.open("try {");
        for ty in boundary
            .iter()
            .filter(|ty| !body.thrown.source_caught_by(program, Some(ty)))
        {
            out.line(format!("if (false) throw ({}) null;", types.class(ty)));
        }
        out.body(body);
        for (index, ty) in boundary.iter().enumerate() {
            out.close(format!("}} catch ({} $checked{index}) {{", types.class(ty)));
            out.indent += 1;
            out.line(format!("throw {}.raise($checked{index});", self.bridge));
        }
        out.close("}");
        body.text = out.text;
    }

    pub fn emit(&self, out: &mut CodeWriter) {
        if !self.used.get() {
            return;
        }
        let name = &self.bridge;
        out.line("");
        out.open(format!("private static final class {name} {{"));
        out.line(format!(
            "private static final java.lang.Class TYPE = new {name}(true).getClass();"
        ));
        out.line("private static java.lang.Throwable pending;");
        out.line(format!("private {name}(boolean unused) {{}}"));
        out.line(format!(
            "public {name}() throws java.lang.Throwable {{ throw pending; }}"
        ));
        out.open("private static java.lang.RuntimeException raise(java.lang.Throwable value) {");
        out.open("synchronized (TYPE) {");
        out.line("pending = value;");
        out.open("try {");
        out.line("TYPE.newInstance();");
        out.close("} catch (java.lang.InstantiationException impossible) {");
        out.indent += 1;
        out.line("throw new java.lang.Error(\"cannot instantiate generated throw bridge\");");
        out.close("} catch (java.lang.IllegalAccessException impossible) {");
        out.indent += 1;
        out.line("throw new java.lang.Error(\"cannot access generated throw bridge\");");
        out.close("} finally {");
        out.indent += 1;
        out.line("pending = null;");
        out.close("}");
        out.close("}");
        out.line("throw new java.lang.Error(\"generated throw bridge returned\");");
        out.close("}");
        out.close("}");
    }
}
