//! Java 1.3 lacks precise rethrow typing. Keep the thrown object while exposing
//! only the checked types proven to reach a bytecode catch/rethrow continuation.
use super::{body::CodeWriter, types::TypeNames};
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone)]
pub struct RethrowHelpers {
    names: BTreeMap<Vec<String>, String>,
    occupied: HashSet<String>,
    next: usize,
}

impl RethrowHelpers {
    pub fn new(program: &Program<'_>, names: &NamePlan, owner: ClassId) -> Self {
        let occupied = std::iter::once(owner)
            .chain(program.ancestors(owner).iter().copied())
            .flat_map(|class| {
                program
                    .class(class)
                    .methods
                    .iter()
                    .enumerate()
                    .map(move |(index, _)| names.method_name(MemberId { class, index }).to_owned())
            })
            .collect();
        Self {
            names: BTreeMap::new(),
            occupied,
            next: 0,
        }
    }

    pub fn request(&mut self, checked: &[String]) -> String {
        if let Some(name) = self.names.get(checked) {
            return name.clone();
        }
        let name = loop {
            let name = format!("$sf$rethrow{}", self.next);
            self.next += 1;
            if self.occupied.insert(name.clone()) {
                break name;
            }
        };
        self.names.insert(checked.to_vec(), name.clone());
        name
    }

    pub fn emit(&self, types: &TypeNames<'_>, out: &mut CodeWriter) {
        for (checked, name) in &self.names {
            out.line("");
            let throws = if checked.is_empty() {
                String::new()
            } else {
                format!(
                    " throws {}",
                    checked
                        .iter()
                        .map(|ty| types.class(ty))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            out.open(format!("private static java.lang.RuntimeException {name}(java.lang.Throwable value){throws} {{"));
            for ty in checked {
                let ty = types.class(ty);
                out.line(format!("if (value instanceof {ty}) throw ({ty}) value;"));
            }
            out.line("if (value instanceof java.lang.Error) throw (java.lang.Error) value;");
            out.line("return (java.lang.RuntimeException) value;");
            out.close("}");
        }
    }
}
