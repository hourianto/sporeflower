//! Java test plumbing. Every test owns its output directories; only tool paths
//! are cached. Use JDK 17 (Java 7 source/target support is required).
//! Missing tools and invalid explicit overrides fail tests.
pub mod source;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};

pub fn javac() -> &'static Path {
    static JAVAC: OnceLock<PathBuf> = OnceLock::new();
    JAVAC.get_or_init(|| java_tool("JAVAC", "javac"))
}

pub fn java() -> &'static Path {
    static JAVA: OnceLock<PathBuf> = OnceLock::new();
    JAVA.get_or_init(|| java_tool("JAVA", "java"))
}

fn java_tool(variable: &str, executable: &str) -> PathBuf {
    std::env::var_os(variable).map_or_else(
        || {
            std::env::var_os("JAVA_HOME").map_or_else(
                || executable.into(),
                |home| PathBuf::from(home).join("bin").join(executable),
            )
        },
        PathBuf::from,
    )
}

pub fn checked(command: &mut Command) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{command:?}: {error}"));
    assert!(
        output.status.success(),
        "{command:?}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

pub fn checked_stdout(command: &mut Command) -> Vec<u8> {
    checked(command).stdout
}

/// Callers add fixture-specific debug, output, and classpath options.
pub fn compile_command() -> Command {
    let mut command = Command::new(javac());
    command.args([
        "-source",
        "7",
        "-target",
        "7",
        "-Xlint:-options",
        "-encoding",
        "UTF-8",
    ]);
    command
}

pub fn compile(sources: &[PathBuf], output: &Path, classpath: Option<&Path>) {
    fs::create_dir_all(output).unwrap();
    let mut command = compile_command();
    command.arg("-d").arg(output);
    // An explicit path also prevents the ambient CLASSPATH/current directory
    // from making a broken recompile resolve classes from another test.
    command.arg("-classpath").arg(classpath.unwrap_or(output));
    checked(command.args(sources));
}

pub fn files(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(files(&path, extension));
        } else if path.extension().is_some_and(|suffix| suffix == extension) {
            result.push(path);
        }
    }
    result.sort();
    result
}

pub fn sources(root: &Path) -> Vec<PathBuf> {
    files(root, "java")
}

pub fn execute(classes: &Path, main: &str) -> Vec<u8> {
    checked_stdout(
        Command::new(java())
            .args(["-Xverify:all", "-cp"])
            .arg(classes)
            .arg(main),
    )
}

pub fn decompile(input: &Path, output: &Path) {
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(input)
            .arg(output)
            .args(["--no-api", "--threads", "1"]),
    );
}

pub fn fixture_roundtrip(name: &str) {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from(format!("tests/fixtures/java/{name}.java"))],
        &original,
        None,
    );
    let expected = execute(&original, name);
    decompile(&original, &source);
    compile(&sources(&source), &rebuilt, None);
    assert_eq!(expected, execute(&rebuilt, name), "{name} runtime trace");
}

/// Compiler JAR for optional Java 1.3 checks, selected by LEGACY_JAVAC.
pub fn legacy_javac() -> Option<&'static Path> {
    static LEGACY: OnceLock<Option<PathBuf>> = OnceLock::new();
    LEGACY
        .get_or_init(|| {
            if let Some(path) = std::env::var_os("LEGACY_JAVAC") {
                let path = PathBuf::from(path);
                assert!(
                    path.is_file(),
                    "LEGACY_JAVAC is not a compiler jar: {}",
                    path.display()
                );
                return Some(path);
            }
            eprintln!(
                "optional Java 1.3 checks skipped: set LEGACY_JAVAC to the compiler JAR path (see docs/testing.md)"
            );
            None
        })
        .as_deref()
}

pub fn legacy_command(compiler: &Path, api: &Path) -> Command {
    let mut command = Command::new(java());
    command
        .arg("-cp")
        .arg(compiler)
        .args([
            "j2me.thirdparty.legacyjavac.Main",
            "-source",
            "1.3",
            "-target",
            "1.1",
            "-bootclasspath",
        ])
        .arg(api)
        .arg("-classpath")
        .arg(api);
    command
}

pub fn legacy_compile(sources: &[PathBuf], output: &Path, api: &Path) -> bool {
    let Some(compiler) = legacy_javac() else {
        return false;
    };
    fs::create_dir_all(output).unwrap();
    checked(
        legacy_command(compiler, api)
            .arg("-d")
            .arg(output)
            .args(sources),
    );
    true
}
