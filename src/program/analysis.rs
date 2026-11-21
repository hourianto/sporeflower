//! Shared analyses have serial lazy entry points for standalone queries. Source
//! preparation builds the same facts in the caller's pool before class workers
//! can wait on them. Only compact per-method summaries survive a scan.
use super::{ClassId, InvocationTargets, MemberId, Program};
use crate::{
    classfile::ClassFile, hash::HashMap, initialization::InitializationPlan,
    ir::boolean_producers::Summary as Booleans, semantics::class_names::Consumers,
    throws::ThrowsPlan,
};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Copy)]
pub(crate) enum MethodScans {
    Serial,
    Workers,
}

impl MethodScans {
    /// Parallel collection is indexed: consumers always see class/declaration order.
    /// The serial path streams each result directly to the consumer.
    pub(crate) fn map<T: Send>(
        self,
        members: impl Iterator<Item = MemberId>,
        scan: impl Fn(MemberId) -> T + Sync + Send,
    ) -> impl Iterator<Item = T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::{iter::Either, prelude::*};
            if matches!(self, Self::Workers) && rayon::current_num_threads() > 1 {
                let methods: Vec<_> = members.collect();
                let summaries: Vec<_> = methods.into_par_iter().map(scan).collect();
                return Either::Left(summaries.into_iter());
            }
            Either::Right(members.map(scan))
        }
        #[cfg(target_arch = "wasm32")]
        members.map(scan)
    }
}

#[derive(Default)]
pub(super) struct Analyses {
    pub(super) invocation_targets: RwLock<HashMap<(ClassId, MemberId), Arc<InvocationTargets>>>,
    pub(super) normal_int_returns: Vec<Vec<OnceLock<Option<i32>>>>,
    booleans: OnceLock<Booleans>,
    class_names: OnceLock<Consumers>,
    throws: OnceLock<ThrowsPlan>,
    effect_provenance: bool,
    initialization: OnceLock<InitializationPlan>,
    inspection: OnceLock<InitializationPlan>,
}

impl Analyses {
    pub(super) fn new(own: &[ClassFile]) -> Self {
        Self {
            normal_int_returns: own
                .iter()
                .map(|class| class.methods.iter().map(|_| OnceLock::new()).collect())
                .collect(),
            ..Self::default()
        }
    }
}

/// Build outside the lock. A Rayon worker can pick up another task that needs
/// this summary while the thread that requested it is waiting for the pool.
fn prepare<T>(slot: &OnceLock<T>, build: impl FnOnce() -> T) {
    if slot.get().is_none() {
        let _ = slot.set(build());
    }
}

impl Program<'_> {
    pub(super) fn boolean_producers(&self) -> &Booleans {
        self.analyses
            .booleans
            .get_or_init(|| Booleans::build(self, MethodScans::Serial))
    }

    pub(crate) fn class_name_consumers(&self) -> &Consumers {
        self.analyses
            .class_names
            .get_or_init(|| Consumers::build(self))
    }

    /// Record representative causes for checked-exception facts, queryable with
    /// [`ThrowsPlan::explain_effect`]. A previously built plan is invalidated.
    pub fn enable_effect_provenance(&mut self) {
        if !self.analyses.effect_provenance {
            self.analyses.effect_provenance = true;
            self.analyses.throws.take();
        }
    }

    pub(crate) fn records_effect_provenance(&self) -> bool {
        self.analyses.effect_provenance
    }

    pub fn throws_plan(&self) -> &ThrowsPlan {
        self.analyses
            .throws
            .get_or_init(|| ThrowsPlan::build(self, MethodScans::Serial))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare_throws(&mut self, pool: &rayon::ThreadPool) {
        pool.install(|| self.prepare_source_throws());
    }

    fn prepare_source_throws(&self) {
        prepare(&self.analyses.throws, || {
            ThrowsPlan::build(self, MethodScans::Workers)
        });
    }

    /// Prepare all summaries needed by Java naming and rendering before
    /// publishing this Program. No worker holds a OnceLock while starting
    /// parallel work; serial callers may still request each summary lazily.
    pub fn prepare_source(&mut self, preserve_bytecode: bool) {
        self.prepare_source_initialization(preserve_bytecode);
        self.prepare_analysis();
    }

    /// Eagerly compute the Boolean and checked-exception summaries in the
    /// current pool; standalone clients may also request them lazily.
    pub fn prepare_analysis(&mut self) {
        prepare(&self.analyses.booleans, || {
            Booleans::build(self, MethodScans::Workers)
        });
        self.prepare_source_throws();
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare_initialization(&mut self) {
        self.prepare_source_initialization(false);
    }

    pub fn prepare_source_initialization(&mut self, preserve_bytecode: bool) {
        prepare(&self.analyses.initialization, || {
            InitializationPlan::build(self, MethodScans::Workers)
        });
        if preserve_bytecode {
            prepare(&self.analyses.inspection, || {
                self.initialization_plan()
                    .inspection(self, MethodScans::Workers)
            });
        }
    }

    pub fn initialization_plan(&self) -> &InitializationPlan {
        self.analyses
            .initialization
            .get_or_init(|| InitializationPlan::build(self, MethodScans::Serial))
    }

    pub(crate) fn source_initialization(&self, preserve_bytecode: bool) -> &InitializationPlan {
        if preserve_bytecode {
            self.analyses.inspection.get_or_init(|| {
                self.initialization_plan()
                    .inspection(self, MethodScans::Serial)
            })
        } else {
            self.initialization_plan()
        }
    }
}
