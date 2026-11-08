//! Print the final Java source plan. Scope choices, statement movement and
//! source idioms are owned by source lowering, never rediscovered here.
use crate::classfile::JvmType;

pub struct CodeWriter {
    pub text: String,
    pub indent: usize,
}
impl CodeWriter {
    pub fn new(indent: usize) -> Self {
        Self {
            text: String::new(),
            indent,
        }
    }
    pub fn line(&mut self, text: impl AsRef<str>) {
        for line in text.as_ref().split('\n') {
            for _ in 0..self.indent {
                self.text.push_str("    ");
            }
            self.text.push_str(line);
            self.text.push('\n');
        }
    }
    pub fn open(&mut self, text: impl AsRef<str>) {
        self.line(text);
        self.indent += 1;
    }
    pub fn close(&mut self, text: impl AsRef<str>) {
        self.indent -= 1;
        self.line(text);
    }
}

impl CodeWriter {
    pub(super) fn body(&mut self, plan: &super::source::Plan) {
        for line in plan.text.split_terminator('\n') {
            self.line(line);
        }
    }
}

pub fn default_value(ty: &JvmType) -> &'static str {
    match ty {
        JvmType::Boolean => "false",
        JvmType::Long => "0L",
        JvmType::Float => "0.0F",
        JvmType::Double => "0.0D",
        JvmType::Object(_) | JvmType::Array(_) => "null",
        _ => "0",
    }
}
