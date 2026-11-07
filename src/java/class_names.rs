//! Small caller-local adapters for finite class-name inputs. The argument is
//! evaluated once, while its ordinary aliases retain their contents and identity.
use super::{body::CodeWriter, literals, types::TypeNames};
use crate::program::Program;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub(super) struct Support {
    helpers: BTreeMap<Vec<(String, String)>, String>,
}

impl Support {
    pub fn is_empty(&self) -> bool {
        self.helpers.is_empty()
    }

    pub fn request(
        &mut self,
        program: &Program<'_>,
        types: &TypeNames<'_>,
        originals: &[String],
    ) -> Option<String> {
        let pairs: Vec<_> = originals
            .iter()
            .filter_map(|original| {
                let mapped = crate::semantics::class_names::relocated(original, types.names)?;
                (mapped != *original).then(|| (original.clone(), mapped))
            })
            .collect();
        if pairs.is_empty() {
            return None;
        }
        if let Some(name) = self.helpers.get(&pairs) {
            return Some(name.clone());
        }
        let base = format!("$sf$className{}", self.helpers.len());
        let name = super::expr::support_name(program, types, &base);
        self.helpers.insert(pairs, name.clone());
        Some(name)
    }

    pub fn emit(&self, out: &mut CodeWriter) {
        let quoted =
            |text: &str| literals::string_literal(&text.encode_utf16().collect::<Vec<_>>());
        for (pairs, name) in &self.helpers {
            out.line("");
            out.open(format!(
                "private static java.lang.String {name}(java.lang.String value) {{"
            ));
            for (original, mapped) in pairs {
                out.line(format!(
                    "if ({}.equals(value)) return {};",
                    quoted(original),
                    quoted(mapped)
                ));
            }
            out.line("return value;");
            out.close("}");
        }
    }
}
