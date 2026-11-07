//! One compilation/decompilation/runtime batch per domain, shared by its shape tests.
pub use crate::common::source::{assert_no_word, method, word_count};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

pub struct Fixture {
    sources: BTreeMap<String, String>,
}

impl Fixture {
    pub fn build(directory: &str, main: &str) -> Self {
        Self::build_with_debug(directory, main, "-g")
    }

    pub fn build_stripped(directory: &str, main: &str) -> Self {
        Self::build_with_debug(directory, main, "-g:none")
    }

    fn build_with_debug(directory: &str, main: &str, debug: &str) -> Self {
        Self::build_transformed(directory, main, debug, |_| {})
    }

    pub fn build_transformed(
        directory: &str,
        main: &str,
        debug: &str,
        transform: impl FnOnce(&Path),
    ) -> Self {
        let work = tempfile::tempdir().unwrap();
        let original = work.path().join("original");
        fs::create_dir_all(&original).unwrap();
        crate::common::checked(
            crate::common::compile_command()
                .args([debug, "-d"])
                .arg(&original)
                .arg("-classpath")
                .arg(&original)
                .args(crate::common::sources(Path::new(directory))),
        );
        transform(&original);
        Self::from_classes(&original, main)
    }

    /// Run the same checks for hand-built or otherwise prepared class files.
    pub fn from_classes(original: &Path, main: &str) -> Self {
        let work = tempfile::tempdir().unwrap();
        let generated = work.path().join("source");
        let modern = work.path().join("modern");
        let legacy = work.path().join("legacy");
        let api = work.path().join("api.jar");
        let expected = crate::common::execute(original, main);
        assert!(!expected.is_empty(), "{main} must exercise its fixture");
        crate::common::checked(
            Command::new(env!("CARGO_BIN_EXE_sporeflower"))
                .arg(original)
                .arg(&generated)
                .arg("--stubs")
                .arg(&api)
                .args(["--threads", "1"]),
        );
        let files = crate::common::sources(&generated);
        crate::common::compile(&files, &modern, None);
        assert_eq!(
            expected,
            crate::common::execute(&modern, main),
            "{main}: modern recompile runtime trace"
        );
        if crate::common::legacy_compile(&files, &legacy, &api) {
            assert_eq!(
                expected,
                crate::common::execute(&legacy, main),
                "{main}: Java 1.3 recompile runtime trace"
            );
        }
        let sources = files
            .into_iter()
            .map(|path| {
                let class = path
                    .strip_prefix(&generated)
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, ".");
                (class, fs::read_to_string(path).unwrap())
            })
            .collect();
        Self { sources }
    }

    pub fn source(&self, class: &str) -> &str {
        self.sources
            .get(class)
            .unwrap_or_else(|| panic!("missing generated class {class}"))
    }
}

/// For small syntax fragments only; never use this to snapshot entire methods.
pub fn compact(source: &str) -> String {
    let mut result = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in source.chars() {
        if let Some(delimiter) = quote {
            result.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
            result.push(character);
        } else if !character.is_whitespace() {
            result.push(character);
        }
    }
    result
}

pub fn assert_contains(body: &str, snippet: &str) {
    assert!(
        compact(body).contains(&compact(snippet)),
        "missing readable idiom {snippet:?}:\n{body}"
    );
}

pub fn assert_absent(body: &str, snippet: &str) {
    assert!(
        !compact(body).contains(&compact(snippet)),
        "unexpected lowering {snippet:?}:\n{body}"
    );
}
