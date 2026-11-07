//! LVT is advisory spelling metadata, including hostile and overlapping ranges.
use crate::common::checked;
use crate::common::source::method_declaration as method;
use sporeflower::{
    input::Input,
    java::{JavaOptions, decompile_class},
    naming::{NameOverrides, NamePlan, RenameOptions},
    program::{MemberId, Program},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn compile(files: &[PathBuf], output: &Path) {
    fs::create_dir_all(output).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g", "-d"])
            .arg(output)
            .args(files),
    );
}

#[test]
fn debug_names_preserve_precise_bindings_and_compile_after_scope_reconstruction() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source_dir = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(&["tests/fixtures/java/DebugNames.java".into()], &original);
    let mut input = Input::read(&original).unwrap().into_classes();
    let class = input
        .iter_mut()
        .find(|class| class.name == "DebugNames")
        .unwrap();
    for method in &mut class.methods {
        let Some(code) = &mut method.code else {
            continue;
        };
        match method.name.as_str() {
            "<clinit>" => {
                for local in &mut code.local_variables {
                    if local.name == "computed" {
                        local.name = "resultConst".into();
                    }
                }
            }
            "duplicateParameters" => {
                for local in &mut code.local_variables {
                    local.name = "pageCount".into();
                }
            }
            "invalidParameter" => {
                let valid = code.local_variables[0].clone();
                for local in &mut code.local_variables {
                    local.name = "class".into();
                }
                code.local_variables.push(valid);
            }
            "ambiguousParameter" => {
                let mut conflicting = code.local_variables[0].clone();
                conflicting.name = "conflicting".into();
                code.local_variables.push(conflicting);
            }
            "legacyReservedParameters" => {
                for local in &mut code.local_variables {
                    local.name = if local.index == 0 { "this$0" } else { "class$" }.into();
                }
            }
            _ => {}
        }
    }
    let program = Program::new(&input, &[]).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    fs::create_dir_all(&source_dir).unwrap();
    let mut files = Vec::new();
    for id in 0..program.own_count() {
        let output = decompile_class(&program, &names, id, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        let path = source_dir.join(format!("{}.java", names.class_name(id)));
        fs::write(&path, output.source).unwrap();
        files.push(path);
    }
    let source = fs::read_to_string(source_dir.join("DebugNames.java")).unwrap();
    assert!(source.contains("int resultConst2 ="), "{source}");
    assert!(
        source.contains("wide(String resource, long timeout, double scale, int retries)"),
        "{source}"
    );
    assert!(
        source.contains("duplicateParameters(int pageCount, int pageCount2)"),
        "{source}"
    );
    assert!(source.contains("invalidParameter(int p0)"), "{source}");
    assert!(source.contains("ambiguousParameter(int p0)"), "{source}");
    assert!(source.contains("helperParameter(int p0)"), "{source}");
    assert!(
        source.contains("legacyReservedParameters(int p0, int p1)"),
        "{source}"
    );
    assert!(
        source.contains("qualifiers(String System2, String java2)"),
        "{source}"
    );
    assert!(source.contains("DebugNames(int seed)"), "{source}");
    assert!(
        source.contains("super(seed <= 0 ? DebugNames.step(-seed) : DebugNames.step(seed), seed)"),
        "{source}"
    );
    assert!(!source.contains("$preinit"), "{source}");
    let switching = method(&source, "switching");
    assert!(
        switching.contains("int pageCount =") && switching.contains("int pageCount2 ="),
        "{switching}"
    );
    let call = method(&source, "callResult");
    assert!(
        call.contains("int height)") && !call.contains("height ="),
        "{call}"
    );
    let loop_body = method(&source, "derivedLoop");
    assert!(
        loop_body.contains("int footprint, int maxTileX)") && !loop_body.contains("footprint ="),
        "{loop_body}"
    );
    compile(&files, &rebuilt);
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("DebugNames"),
    )
    .stdout;
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("DebugNames"),
    )
    .stdout;
    assert_eq!(expected, actual);

    if let Some(legacy) = crate::common::legacy_javac() {
        let api = sporeflower::api::ApiCatalog::bundled()
            .unwrap()
            .select(&input, &Default::default())
            .unwrap();
        let api_jar = work.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        let legacy_classes = work.path().join("legacy");
        fs::create_dir_all(&legacy_classes).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api_jar)
                .arg("-d")
                .arg(legacy_classes)
                .args(files),
        );
    }
}

#[test]
fn authored_parameter_names_replace_debug_names_in_headers_and_wide_local_slots() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source_dir = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(&["tests/fixtures/java/DebugNames.java".into()], &original);
    let input = Input::read(&original).unwrap().into_classes();
    let program = Program::new(&input, &[]).unwrap();
    let owner = program.id("DebugNames").unwrap();
    let mut overrides = NameOverrides::default();
    for (index, method) in program.class(owner).methods.iter().enumerate() {
        let names = match method.name.as_str() {
            "wide" => vec!["text", "ticks", "ratio", "attempts"],
            "<init>" => vec!["initial"],
            _ => continue,
        };
        overrides.parameters.insert(
            MemberId {
                class: owner,
                index,
            },
            names.into_iter().map(Into::into).collect(),
        );
    }
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    fs::create_dir_all(&source_dir).unwrap();
    let mut files = Vec::new();
    for id in 0..program.own_count() {
        let output = decompile_class(&program, &names, id, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        if id == owner {
            assert!(
                output
                    .source
                    .contains("wide(String text, long ticks, double ratio, int attempts)"),
                "{}",
                output.source
            );
            assert!(
                output.source.contains("DebugNames(int initial)"),
                "{}",
                output.source
            );
            assert!(!method(&output.source, "wide").contains("timeout"));
        }
        let path = source_dir.join(format!("{}.java", names.class_name(id)));
        fs::write(&path, output.source).unwrap();
        files.push(path);
    }
    compile(&files, &rebuilt);
    let run = |directory: &Path| {
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(directory)
                .arg("DebugNames"),
        )
        .stdout
    };
    assert_eq!(run(&original), run(&rebuilt));
}

#[test]
fn authored_parameters_preserve_type_and_final_field_bindings() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source_dir = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &crate::common::sources(Path::new("tests/fixtures/authored_bindings")),
        &original,
    );
    // Move NEW before an independent effect so preserving its initialization
    // boundary requires a generated static trigger, not just an allocation
    // with an inline argument. That helper owner is shadowed by a parameter.
    let path = original.join("AuthoredBindings.class");
    let mut bytes = fs::read(&path).unwrap();
    let parsed = sporeflower::classfile::ClassFile::parse(&bytes).unwrap();
    let code = &parsed
        .methods
        .iter()
        .find(|method| method.name == "initializeTarget")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let instructions = sporeflower::bytecode::decode(code).unwrap();
    let allocation = instructions
        .iter()
        .position(|instruction| instruction.opcode == 0xbb)
        .unwrap();
    assert_eq!(instructions[allocation + 1].opcode, 0x59);
    let begin = instructions[allocation].pc as usize;
    let end = instructions[allocation + 2].pc as usize;
    let reordered: Vec<_> = code[begin..end]
        .iter()
        .chain(&code[..begin])
        .copied()
        .collect();
    let offset = bytes
        .windows(code.len())
        .position(|part| part == code)
        .unwrap();
    bytes[offset..offset + end].copy_from_slice(&reordered);
    let max_stack = u16::from_be_bytes(bytes[offset - 8..offset - 6].try_into().unwrap());
    bytes[offset - 8..offset - 6].copy_from_slice(&(max_stack + 2).to_be_bytes());
    // JVM call operands use the int stack category for byte parameters. Omit
    // javac's redundant conversion (the field is always 1) so source binding
    // must reintroduce a cast when its conditional is no longer byte-typed.
    let conditional = &parsed
        .methods
        .iter()
        .find(|method| method.name == "conditional")
        .unwrap()
        .code
        .as_ref()
        .unwrap()
        .bytes;
    let narrow = sporeflower::bytecode::decode(conditional)
        .unwrap()
        .into_iter()
        .find(|instruction| instruction.opcode == 0x91)
        .unwrap();
    let offset = bytes
        .windows(conditional.len())
        .position(|part| part == conditional)
        .unwrap();
    bytes[offset + narrow.pc as usize] = 0x00;
    fs::write(path, bytes).unwrap();
    let mut input = Input::read(&original).unwrap().into_classes();
    // Model GETSTATIC of a ConstantValue field, which ordinary javac would
    // inline. Its numeric value is unchanged, but source qualification through
    // a primary expression no longer permits constant conditional narrowing.
    let constants = input.iter_mut().find(|class| class.name == "x").unwrap();
    let constant = constants
        .fields
        .iter()
        .find(|field| field.name == "fixed")
        .unwrap()
        .constant_value;
    let field = constants
        .fields
        .iter_mut()
        .find(|field| field.name == "other")
        .unwrap();
    field.access_flags |= 0x0010;
    field.constant_value = constant;
    let class = input
        .iter_mut()
        .find(|class| class.name == "AuthoredBindings")
        .unwrap();
    // A blank static final assignment requires a simple field name. Its debug
    // local still needs repair, while a parameter in another method may use
    // exactly the authored field spelling.
    for local in class
        .methods
        .iter_mut()
        .find(|method| method.name == "<clinit>")
        .unwrap()
        .code
        .as_mut()
        .unwrap()
        .local_variables
        .iter_mut()
    {
        if local.name == "computed" {
            local.name = "bound".into();
        }
    }
    let program = Program::new(&input, &[]).unwrap();
    let mut overrides = NameOverrides::default();
    for (owner, method, descriptor, parameters) in [
        ("AuthoredBindings", "<init>", "(I)V", vec!["total"]),
        ("AuthoredBindings", "sameFinal", "(I)I", vec!["total"]),
        ("AuthoredBindings", "sameField", "(I)I", vec!["bound"]),
        ("AuthoredBindings", "sameType", "(I)I", vec!["x"]),
        (
            "AuthoredBindings",
            "sameOwner",
            "(I)I",
            vec!["AuthoredBindings"],
        ),
        (
            "AuthoredBindings",
            "sameLibrary",
            "(II)I",
            vec!["Integer", "java"],
        ),
        (
            "AuthoredBindings",
            "initializeTarget",
            "(I)I",
            vec!["Initialized"],
        ),
        (
            "AuthoredBindings",
            "conditional",
            "(ZBI)I",
            vec!["flag", "value", "x"],
        ),
        ("named/Probe", "run", "(I)I", vec!["x"]),
        ("named/Probe", "sameType", "(Lnamed/x;)Lnamed/x;", vec!["x"]),
    ] {
        let member = program
            .declared_method(program.id(owner).unwrap(), method, descriptor)
            .unwrap();
        overrides
            .parameters
            .insert(member, parameters.into_iter().map(str::to_owned).collect());
    }
    let names = NamePlan::build_with_overrides(&program, &Default::default(), &overrides).unwrap();
    for id in 0..program.own_count() {
        let output = decompile_class(&program, &names, id, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        let path = source_dir.join(format!("{}.java", names.class_name(id)));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, output.source).unwrap();
    }
    let source = fs::read_to_string(source_dir.join("AuthoredBindings.java")).unwrap();
    for spelling in [
        "AuthoredBindings(int total)",
        "this.total = total;",
        "sameFinal(int total)",
        "this.total + total",
        "sameField(int bound)",
        "((AuthoredBindings) null).bound + bound",
        "sameType(int x)",
        "((x) null).read() + x",
        "sameOwner(int AuthoredBindings)",
        "((AuthoredBindings) null).sameType(AuthoredBindings)",
        "sameLibrary(int Integer, int java)",
        "((Integer) null).parseInt(\"7\")",
        "initializeTarget(int Initialized)",
        "((Initialized) null).$sf$initialize$",
        "int bound2 =",
        "bound = bound2 * bound2;",
    ] {
        assert!(source.contains(spelling), "missing {spelling:?}\n{source}");
    }
    let packaged = fs::read_to_string(source_dir.join("named/Probe.java")).unwrap();
    for spelling in ["run(int x)", "named.x.read() + x", "x sameType(x x)"] {
        assert!(
            packaged.contains(spelling),
            "missing {spelling:?}\n{packaged}"
        );
    }
    let files = crate::common::sources(&source_dir);
    compile(&files, &rebuilt);
    let expected = crate::common::execute(&original, "AuthoredBindings");
    assert_eq!(
        expected,
        crate::common::execute(&rebuilt, "AuthoredBindings")
    );
    if let Some(legacy) = crate::common::legacy_javac() {
        let api = sporeflower::api::ApiCatalog::bundled()
            .unwrap()
            .select(&input, &Default::default())
            .unwrap();
        let api_jar = work.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        let legacy_classes = work.path().join("legacy");
        fs::create_dir_all(&legacy_classes).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api_jar)
                .arg("-d")
                .arg(&legacy_classes)
                .args(files),
        );
        assert_eq!(
            expected,
            crate::common::execute(&legacy_classes, "AuthoredBindings")
        );
    }
}
