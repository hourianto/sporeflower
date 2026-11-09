use super::*;
use crate::api::{ApiCatalog, ApiOptions};
use crate::input::Input;
use std::path::Path;

#[test]
fn prepared_and_lazy_plans_preserve_all_facts_across_worker_counts() {
    let mut input = Input::read(Path::new("tests/fixtures/exception/classes"))
        .unwrap()
        .into_classes();
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&input, &ApiOptions::default())
        .unwrap();
    // A failed method scan is still assembled in its original position, with
    // the same diagnostic, while unrelated method summaries remain available.
    let mut invalid = input[0]
        .methods
        .iter()
        .find(|method| method.code.is_some())
        .unwrap()
        .clone();
    invalid.name = "invalid".into();
    invalid.code.as_mut().unwrap().bytes = vec![0xff];
    input[0].methods.push(invalid);
    for record_provenance in [false, true] {
        let mut expected_program = Program::new(&input, &api.classes).unwrap();
        if record_provenance {
            expected_program.enable_effect_provenance();
        }
        let expected = expected_program.throws_plan();
        assert!(!expected.diagnostics.is_empty());

        for threads in [1, 2, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let mut prepared = Program::new(&input, &api.classes).unwrap();
            if record_provenance {
                prepared.enable_effect_provenance();
            }
            prepared.prepare_throws(&pool);
            assert_eq!(prepared.throws_plan(), expected, "{threads} workers");
            prepared.prepare_throws(&pool);
            assert_eq!(prepared.throws_plan(), expected);

            // Library callers can still request the lazy plan from class workers.
            // Its serial initializer never forks while holding the OnceLock.
            let mut lazy = Program::new(&input, &api.classes).unwrap();
            if record_provenance {
                lazy.enable_effect_provenance();
            }
            let (left, right) =
                pool.install(|| rayon::join(|| lazy.throws_plan(), || lazy.throws_plan()));
            assert_eq!(left, expected);
            assert_eq!(right, expected);
        }
    }
}

#[test]
fn provenance_opt_in_rebuilds_only_diagnostic_history() {
    let mut input = Input::read(Path::new("tests/fixtures/exception/classes"))
        .unwrap()
        .into_classes();
    let index = input[0]
        .methods
        .iter()
        .position(|method| method.access_flags & 0x0008 != 0)
        .unwrap();
    input[0].methods[index]
        .exceptions
        .push("java/io/IOException".into());
    let member = MemberId { class: 0, index };
    let effect = InferredEffect::Declared("java/io/IOException".into());
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&input, &ApiOptions::default())
        .unwrap();
    let mut program = Program::new(&input, &api.classes).unwrap();
    let default_engine = Engine::new(&program);
    assert!(default_engine.provenance.is_none());
    assert!(
        default_engine
            .nodes
            .iter()
            .all(|node| node.provenance.is_none())
    );
    let expected = ThrowsPlan::build(&program);
    assert!(expected.provenance.is_none());
    assert!(program.throws_plan().provenance.is_none());
    assert!(
        program
            .throws_plan()
            .explain_effect(member, &effect)
            .is_empty()
    );

    // Opting in after lazy analysis must not silently leave the cached plan
    // untraced. Enabling recording does not change any semantic facts.
    program.enable_effect_provenance();
    let traced = program.throws_plan();
    assert!(traced.provenance.is_some());
    assert!(!traced.explain_effect(member, &effect).is_empty());

    let mut actual = ThrowsPlan::build(&program);
    assert!(actual.provenance.take().is_some());
    assert_eq!(actual, expected);
}
