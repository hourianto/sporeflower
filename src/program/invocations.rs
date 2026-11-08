//! Shared JVM target identity. Each consumer decides which implementations and
//! boundary assumptions are relevant to its own proof domain.
use super::{ClassId, MemberId, Program};
use crate::ir::{InvokeKind, Member};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) struct InvocationTargets {
    pub members: Vec<MemberId>,
    /// Supplied receiver hierarchies and method lookups have no missing links.
    /// This does not assert that all possible external subclasses were supplied.
    pub hierarchy_complete: bool,
}

impl Program<'_> {
    pub(crate) fn virtual_targets(
        &self,
        owner: ClassId,
        target: MemberId,
    ) -> Arc<InvocationTargets> {
        let key = (owner, target);
        if let Some(found) = self.invocation_targets.read().unwrap().get(&key) {
            return Arc::clone(found);
        }
        let method = self.method(target);
        let mut members = BTreeSet::from([target]);
        let mut hierarchy_complete = self.hierarchy_complete[owner];
        for &class in self.descendants(owner) {
            hierarchy_complete &= self.hierarchy_complete[class];
            match self.resolve_method(&self.class(class).name, &method.name, &method.descriptor) {
                Some(member) => {
                    members.insert(member);
                }
                None => hierarchy_complete = false,
            }
        }
        let targets = Arc::new(InvocationTargets {
            members: members.into_iter().collect(),
            hierarchy_complete,
        });
        Arc::clone(
            self.invocation_targets
                .write()
                .unwrap()
                .entry(key)
                .or_insert(targets),
        )
    }

    pub(crate) fn call_targets(
        &self,
        caller: ClassId,
        kind: InvokeKind,
        method: &Member,
    ) -> Vec<MemberId> {
        if let Some(target) = self.exact_method(Some(caller), kind, method) {
            return vec![target];
        }
        if !matches!(kind, InvokeKind::Virtual | InvokeKind::Interface) {
            return Vec::new();
        }
        let (Some(owner), Some(target)) = (
            self.id(&method.owner),
            self.resolve_method(&method.owner, &method.name, &method.descriptor),
        ) else {
            return Vec::new();
        };
        self.virtual_targets(owner, target).members.clone()
    }
}
