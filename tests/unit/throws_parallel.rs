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
    let expected_program = Program::new(&input, &api.classes).unwrap();
    let expected = expected_program.throws_plan();
    assert!(!expected.diagnostics.is_empty());

    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let mut prepared = Program::new(&input, &api.classes).unwrap();
        prepared.prepare_throws(&pool);
        assert_eq!(prepared.throws_plan(), expected, "{threads} workers");
        prepared.prepare_throws(&pool);
        assert_eq!(prepared.throws_plan(), expected);

        // Library callers can still request the lazy plan from class workers.
        // Its serial initializer never forks while holding the OnceLock.
        let lazy = Program::new(&input, &api.classes).unwrap();
        let (left, right) =
            pool.install(|| rayon::join(|| lazy.throws_plan(), || lazy.throws_plan()));
        assert_eq!(left, expected);
        assert_eq!(right, expected);
    }
}
