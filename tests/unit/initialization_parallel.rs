use super::*;
use crate::input::Input;
use std::path::Path;

#[test]
fn prepared_and_lazy_initialization_preserve_sites_and_final_writes_across_workers() {
    let mut own = Input::read(Path::new("tests/fixtures/allocation_graph/classes"))
        .unwrap()
        .into_classes();
    // The fixture has NEW crossing a branch, a loop and alternative constructor
    // calls. Supplying one target as a library also exercises external sites.
    let external = own
        .iter()
        .position(|class| class.name == "AllocationGraphProbe$Loop")
        .unwrap();
    let libraries = [own.remove(external)];
    let probe = own
        .iter_mut()
        .find(|class| class.name == "AllocationGraphProbe")
        .unwrap();
    // Pre-Java-9 bytecode may write a final from a non-initializer. This field
    // must be recorded as written but cannot remain final in generated Java.
    probe.major_version = 49;
    probe
        .fields
        .iter_mut()
        .find(|field| field.name == "branchEffects")
        .unwrap()
        .access_flags |= 0x0010;
    let mut invalid = probe
        .methods
        .iter()
        .find(|method| method.name == "branch")
        .unwrap()
        .clone();
    invalid.name = "invalid".into();
    invalid.code.as_mut().unwrap().bytes = vec![0xff];
    probe.methods.push(invalid);

    let serial = Program::new(&own, &libraries).unwrap();
    let expected = serial.initialization_plan();
    assert!(expected.statistics.candidate_methods > 0);
    assert!(expected.statistics.moved_boundaries > 0);
    assert!(expected.statistics.trigger_sites > 0);
    assert!(expected.statistics.external_sites > 0);
    assert!(!expected.classes.is_empty());
    let mutable = serial
        .resolve_field("AllocationGraphProbe", "branchEffects", "I")
        .unwrap();
    assert!(expected.writes_final(mutable));
    assert!(!expected.permits_final(mutable));
    let final_field = serial
        .resolve_field("AllocationGraphProbe$Choice", "value", "I")
        .unwrap();
    assert!(expected.writes_final(final_field));
    assert!(expected.permits_final(final_field));

    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let mut prepared = Program::new(&own, &libraries).unwrap();
        pool.install(|| prepared.prepare_initialization());
        assert_eq!(
            prepared.initialization_plan(),
            expected,
            "{threads} workers"
        );
        pool.install(|| prepared.prepare_initialization());
        assert_eq!(prepared.initialization_plan(), expected);

        // Lazy library callers may request the same plan from class workers.
        // The initializer stays serial, so its OnceLock cannot wait on a
        // suspended Rayon job which itself needs the plan.
        let lazy = Program::new(&own, &libraries).unwrap();
        let (left, right) = pool
            .install(|| rayon::join(|| lazy.initialization_plan(), || lazy.initialization_plan()));
        assert_eq!(left, expected);
        assert_eq!(right, expected);
    }
}
