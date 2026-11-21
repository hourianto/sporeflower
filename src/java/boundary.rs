//! Rare JVM callbacks can let a checked exception escape a Java interface that
//! declares none. CLDC Class.newInstance propagates its nullary constructor's
//! exception directly. This bridge retains the exact object, without wrapping.
use super::{body::CodeWriter, source::Plan, types::TypeNames};
use crate::program::Program;

#[derive(Clone)]
pub struct BoundarySupport {
    bridge: String,
}

impl BoundarySupport {
    pub fn new(program: &Program<'_>, types: &TypeNames<'_>) -> Self {
        Self {
            bridge: super::generated::symbol(program, types, "$CheckedThrow"),
        }
    }

    pub fn plan(
        &self,
        program: &Program<'_>,
        types: &TypeNames<'_>,
        boundary: &[String],
        body: &mut Plan,
        container: Option<&str>,
    ) {
        let mut out = CodeWriter::new(0);
        out.open("try {");
        for ty in boundary
            .iter()
            .filter(|ty| !body.thrown.source_caught_by(program, Some(ty)))
        {
            out.line(format!("if (false) throw ({}) null;", types.class(ty)));
        }
        out.body(body);
        let prefix = container.map_or_else(String::new, |name| format!("{name}."));
        for (index, ty) in boundary.iter().enumerate() {
            out.close(format!("}} catch ({} $checked{index}) {{", types.class(ty)));
            out.indent += 1;
            out.line(format!(
                "throw {prefix}{}.raise($checked{index});",
                self.bridge
            ));
        }
        out.close("}");
        body.text = out.text;
    }

    /// Request exact same-object transport for a rethrow whose residual effect
    /// excludes both reflection exception families. The caller owns that proof.
    pub fn rethrow(&self) -> String {
        format!("{}.raise", self.bridge)
    }

    pub fn emit(&self, out: &mut CodeWriter) {
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
