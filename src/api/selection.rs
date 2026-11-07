use super::{
    ApiCatalog, ApiOptions, ApiSelection, Candidate,
    requirements::{self, Member, Requirements},
};
use crate::{Result, classfile::ClassFile};
use anyhow::Context;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

struct Selection<'a> {
    catalog: &'a ApiCatalog,
    project: HashMap<&'a str, &'a ClassFile>,
    requirements: Requirements,
    wants_cldc10: bool,
}

pub(super) fn select(
    catalog: &ApiCatalog,
    project: &[ClassFile],
    classpath: &[ClassFile],
    options: &ApiOptions,
) -> Result<ApiSelection> {
    let mut requirements = Requirements::inspect(project);
    requirements.classes.extend(
        options
            .additional_roots
            .iter()
            .filter(|name| catalog.providers.contains_key(name.as_str()))
            .cloned(),
    );
    // Resolve supplied declarations before collecting SDK dependencies:
    // application declarations win, then the last explicit classpath entry.
    let mut supplied: HashMap<_, _> = classpath
        .iter()
        .map(|class| (class.name.as_str(), class))
        .collect();
    for class in project {
        supplied.remove(class.name.as_str());
    }
    for class in supplied.values() {
        requirements.declaration(class);
    }
    supplied.extend(project.iter().map(|class| (class.name.as_str(), class)));
    let wants_cldc10 = options
        .configuration
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("CLDC-1.0"))
        && !requirements.floating_point
        && !requirements.classes.contains("java/lang/Float")
        && !requirements.classes.contains("java/lang/Double");
    let mut selection = Selection {
        catalog,
        project: supplied,
        requirements,
        wants_cldc10,
    };
    selection.propagate_project_demands();
    let mut selected = selection.choose_all();
    // Demands only grow. Each inherited requirement is inserted once per owner;
    // provider changes cannot cause an oscillating selection loop.
    loop {
        let mut additions = Vec::new();
        for (owner, members) in &selection.requirements.members {
            let Some(&candidate) = selected.get(owner.as_str()) else {
                continue;
            };
            let definition = catalog.definition(candidate);
            for member in members {
                if member.name == "<init>" || definition.declares(member) {
                    continue;
                }
                for &parent in &definition.parents {
                    if catalog.providers.get(parent).is_some_and(|alternatives| {
                        alternatives.iter().any(|candidate| {
                            selection.supports(
                                candidate.library,
                                parent,
                                member,
                                &mut HashSet::new(),
                            )
                        })
                    }) {
                        additions.push((parent.to_owned(), member.clone()));
                        break;
                    }
                }
            }
        }
        let mut changed = false;
        for (owner, member) in additions {
            changed |= selection
                .requirements
                .members
                .entry(owner)
                .or_default()
                .insert(member);
        }
        if !changed {
            break;
        }
        selected = selection.choose_all();
    }
    selection.materialize(selected, options.include_all)
}

impl Selection<'_> {
    fn supports(
        &self,
        library: usize,
        owner: &str,
        member: &Member,
        visited: &mut HashSet<String>,
    ) -> bool {
        if !visited.insert(owner.to_owned()) {
            return false;
        }
        let own = self.catalog.libraries[library]
            .by_name
            .get(owner)
            .map(|&class| Candidate { library, class });
        let result = if let Some(candidate) = own {
            self.definition_supports(candidate, member, visited)
        } else {
            self.catalog
                .providers
                .get(owner)
                .is_some_and(|alternatives| {
                    alternatives
                        .iter()
                        .any(|&candidate| self.definition_supports(candidate, member, visited))
                })
        };
        visited.remove(owner);
        result
    }

    fn definition_supports(
        &self,
        candidate: Candidate,
        member: &Member,
        visited: &mut HashSet<String>,
    ) -> bool {
        let definition = self.catalog.definition(candidate);
        definition.declares(member)
            || member.name != "<init>"
                && definition
                    .parents
                    .iter()
                    .any(|parent| self.supports(candidate.library, parent, member, visited))
    }

    fn inherited_owner(
        &self,
        owner: &str,
        member: &Member,
        visited: &mut HashSet<String>,
    ) -> Option<String> {
        if !visited.insert(owner.to_owned()) {
            return None;
        }
        let result = if let Some(class) = self.project.get(owner) {
            if member.name == "<init>" || requirements::declares(class, member) {
                None
            } else {
                class
                    .super_name
                    .iter()
                    .chain(&class.interfaces)
                    .find_map(|parent| self.inherited_owner(parent, member, visited))
            }
        } else if self
            .catalog
            .providers
            .get(owner)
            .is_some_and(|alternatives| {
                alternatives.iter().any(|candidate| {
                    self.supports(candidate.library, owner, member, &mut HashSet::new())
                })
            })
        {
            Some(owner.to_owned())
        } else if !self.catalog.providers.contains_key(owner)
            && self.requirements.superclasses.contains(owner)
        {
            // This is an unresolved superclass boundary, not a real declaration.
            // Carry the observed shape only into its export-only placeholder.
            Some(owner.to_owned())
        } else {
            None
        };
        visited.remove(owner);
        result
    }

    fn propagate_project_demands(&mut self) {
        let mut additions = Vec::new();
        for (owner, members) in &self.requirements.members {
            if !self.project.contains_key(owner.as_str()) {
                continue;
            }
            for member in members {
                if let Some(inherited) = self.inherited_owner(owner, member, &mut HashSet::new()) {
                    additions.push((inherited, member.clone()));
                }
            }
        }
        for (owner, member) in additions {
            self.requirements.classes.insert(owner.clone());
            self.requirements
                .members
                .entry(owner)
                .or_default()
                .insert(member);
        }
    }

    fn missing(&self, candidate: Candidate) -> usize {
        let definition = self.catalog.definition(candidate);
        let kind_mismatches = self
            .requirements
            .interface_owners
            .get(definition.name)
            .map_or(0, |kinds| {
                kinds
                    .iter()
                    .filter(|&&interface| interface != (definition.access & 0x0200 != 0))
                    .count()
            });
        kind_mismatches
            + self
                .requirements
                .members
                .get(definition.name)
                .map_or(0, |members| {
                    members
                        .iter()
                        .filter(|member| {
                            !self.supports(
                                candidate.library,
                                definition.name,
                                member,
                                &mut HashSet::new(),
                            )
                        })
                        .count()
                })
    }

    fn choose_boot(&self) -> Option<usize> {
        let mut required_core: BTreeSet<_> = self
            .requirements
            .classes
            .iter()
            .map(String::as_str)
            .chain(self.requirements.members.keys().map(String::as_str))
            .filter(|name| self.catalog.core_owners.contains(name))
            .collect();
        if self.requirements.floating_point {
            required_core.extend(["java/lang/Float", "java/lang/Double"]);
        }
        self.catalog
            .boot_providers
            .iter()
            .copied()
            .min_by_key(|&library| {
                let provider = &self.catalog.libraries[library];
                let absent = required_core
                    .iter()
                    .filter(|owner| !provider.by_name.contains_key(**owner))
                    .count();
                let missing = required_core
                    .iter()
                    .filter_map(|owner| provider.by_name.get(*owner))
                    .map(|&class| self.missing(Candidate { library, class }))
                    .sum::<usize>();
                let broad = provider
                    .by_name
                    .contains_key("javax/microedition/midlet/MIDlet");
                let core_mismatch =
                    provider.by_name.contains_key("java/lang/Float") == self.wants_cldc10;
                (
                    absent,
                    missing,
                    broad,
                    core_mismatch,
                    provider.fallback,
                    provider.classes.len(),
                    provider.name,
                )
            })
    }

    fn choose_all(&self) -> BTreeMap<&'static str, Candidate> {
        let boot = self.choose_boot();
        let mut selected = BTreeMap::new();
        for (&owner, alternatives) in &self.catalog.providers {
            if self.project.contains_key(owner) {
                continue;
            }
            let choice = if self.catalog.core_owners.contains(owner) {
                boot.and_then(|library| {
                    self.catalog.libraries[library]
                        .by_name
                        .get(owner)
                        .map(|&class| Candidate { library, class })
                })
            } else {
                alternatives.iter().copied().min_by_key(|&candidate| {
                    let library = &self.catalog.libraries[candidate.library];
                    (
                        self.missing(candidate),
                        library.fallback,
                        library.by_name.contains_key("java/lang/Object"),
                        library.classes.len(),
                        library.name,
                    )
                })
            };
            if let Some(choice) = choice {
                selected.insert(owner, choice);
            }
        }
        selected
    }

    fn supports_selected(
        &self,
        selected: &BTreeMap<&str, Candidate>,
        owner: &str,
        member: &Member,
        visited: &mut HashSet<String>,
    ) -> bool {
        if !visited.insert(owner.to_owned()) {
            return false;
        }
        let result = selected.get(owner).is_some_and(|&candidate| {
            let definition = self.catalog.definition(candidate);
            definition.declares(member)
                || member.name != "<init>"
                    && definition
                        .parents
                        .iter()
                        .any(|parent| self.supports_selected(selected, parent, member, visited))
        });
        visited.remove(owner);
        result
    }

    fn materialize(
        mut self,
        selected: BTreeMap<&'static str, Candidate>,
        include_all: bool,
    ) -> Result<ApiSelection> {
        let (inferred, inference_warnings) = super::inferred::build(&self.requirements, |name| {
            self.project.contains_key(name) || selected.contains_key(name)
        })?;
        self.requirements.warnings.extend(inference_warnings);
        let mut pending: VecDeque<String> = if include_all {
            selected.keys().map(|name| (*name).to_owned()).collect()
        } else {
            self.requirements.classes.iter().cloned().collect()
        };
        // The Java compiler needs language-level roots even when no constant
        // pool explicitly names them (for example Error in checked-exception
        // analysis of an ordinary method invocation).
        pending.extend(
            [
                "java/lang/Object",
                "java/lang/Class",
                "java/lang/String",
                "java/lang/Throwable",
                "java/lang/Exception",
                "java/lang/RuntimeException",
                "java/lang/Error",
                // Legacy javac lowers class literals through Class.forName
                // and translates a lookup failure to this supported CLDC error.
                "java/lang/NoClassDefFoundError",
            ]
            .into_iter()
            // Literal presentation can introduce these constant owners without
            // a class-pool reference. Export them with the compiler roots too.
            .chain(crate::java::STANDARD_CONSTANT_OWNERS.iter().copied())
            .map(str::to_owned),
        );
        let mut visited = HashSet::new();
        let mut classes = BTreeMap::new();
        let mut bytes = BTreeMap::new();
        let mut providers = BTreeMap::new();
        while let Some(name) = pending.pop_front() {
            if !visited.insert(name.clone())
                || self.project.contains_key(name.as_str())
                || name.starts_with('[')
            {
                continue;
            }
            let Some(&candidate) = selected.get(name.as_str()) else {
                continue;
            };
            let definition = self.catalog.definition(candidate);
            let class = ClassFile::parse(definition.bytes)
                .with_context(|| format!("parse bundled API {}", definition.name))?;
            let mut dependencies = Requirements::default();
            dependencies.declaration(&class);
            pending.extend(dependencies.classes);
            providers.insert(
                name.clone(),
                self.catalog.libraries[candidate.library].name.to_owned(),
            );
            bytes.insert(definition.name, definition.bytes);
            classes.insert(name, class);
        }
        for (owner, members) in &self.requirements.members {
            if self.project.contains_key(owner.as_str()) || !selected.contains_key(owner.as_str()) {
                continue;
            }
            for member in members {
                if !self.supports_selected(&selected, owner, member, &mut HashSet::new()) {
                    let kind = match member.is_static {
                        Some(true) => "static ",
                        Some(false) => "instance ",
                        None => "",
                    };
                    self.requirements.warnings.push(format!(
                        "selected API has no {kind}{} {}.{}{}",
                        if member.field { "field" } else { "method" },
                        owner,
                        member.name,
                        member.descriptor
                    ));
                }
            }
        }
        self.requirements.warnings.sort();
        self.requirements.warnings.dedup();
        Ok(ApiSelection {
            classes: classes.into_values().collect(),
            providers,
            warnings: self.requirements.warnings,
            bytes,
            inferred,
        })
    }
}
