//! Type syntax and expression qualifiers obey different Java shadowing rules.
use crate::common::{checked_stdout as checked, sources};
use sporeflower::{
    api::ApiCatalog,
    input::Input,
    java::{JavaOptions, decompile_class},
    naming::{NamePlan, RenameOptions},
    program::Program,
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
fn imports_follow_source_bindings_instead_of_constant_pool_references() {
    use sporeflower::classfile::Constant;

    let work = tempfile::tempdir().unwrap();
    let fixture = work.path().join("fixture");
    fs::create_dir_all(fixture.join("consumer")).unwrap();
    fs::create_dir_all(fixture.join("library")).unwrap();
    fs::write(
        fixture.join("consumer/ImportUsage.java"),
        r#"
package consumer;
public class ImportUsage extends library.Base implements library.Contract {
    int Active;
    public void run() { library.Active.touch(); }
    public int read() {
        library.Result result = library.Factory.make();
        return result.read();
    }
    public static String text() { return "Ghost Result Active"; }
    public static void main(String[] args) {
        ImportUsage value = new ImportUsage();
        value.run();
        System.out.println(value.read());
    }
}
"#,
    )
    .unwrap();
    for (name, body) in [
        ("Base", "public class Base {}"),
        ("Contract", "public interface Contract { void run(); }"),
        (
            "Active",
            "public class Active { public static void touch() {} }",
        ),
        (
            "Factory",
            "public class Factory { public static Result make() { return new Result(); } }",
        ),
        (
            "Result",
            "public class Result { public int read() { return 17; } }",
        ),
        ("Ghost", "public class Ghost {}"),
    ] {
        fs::write(
            fixture.join(format!("library/{name}.java")),
            format!("package library; {body}"),
        )
        .unwrap();
    }
    let original = work.path().join("original");
    compile(&sources(&fixture), &original);
    let mut input = Input::read(&original).unwrap().into_classes();
    let consumer = input
        .iter_mut()
        .find(|class| class.name == "consumer/ImportUsage")
        .unwrap();
    let name = consumer.constant_pool.entries.len() as u16;
    consumer
        .constant_pool
        .entries
        .push(Constant::Utf8("library/Ghost".into()));
    consumer.constant_pool.entries.push(Constant::Class(name));
    for method in &mut consumer.methods {
        if let Some(code) = &mut method.code {
            code.local_variables.clear();
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&input, &Default::default())
        .unwrap();
    let program = Program::new(&input, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let source = work.path().join("source");
    for id in 0..program.own_count() {
        let rendered = decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
        if program.class(id).name == "consumer/ImportUsage" {
            for required in [
                "import library.Base;",
                "import library.Contract;",
                "import library.Factory;",
            ] {
                assert!(rendered.source.contains(required), "{}", rendered.source);
            }
            for unused in [
                "import library.Ghost;",
                "import library.Result;",
                "import library.Active;",
                "import java.io.PrintStream;",
            ] {
                assert!(!rendered.source.contains(unused), "{}", rendered.source);
            }
            assert!(
                rendered.source.contains("library.Active.touch()"),
                "{}",
                rendered.source
            );
        }
        let path = source.join(format!("{}.java", names.class_name(id)));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, rendered.source).unwrap();
    }
    let files = sources(&source);
    let rebuilt = work.path().join("rebuilt");
    compile(&files, &rebuilt);
    assert_eq!(
        crate::common::execute(&original, "consumer.ImportUsage"),
        crate::common::execute(&rebuilt, "consumer.ImportUsage")
    );
    let api_jar = work.path().join("api.jar");
    api.write_jar(&api_jar).unwrap();
    crate::common::legacy_compile(&files, &work.path().join("legacy"), &api_jar);
}

#[test]
fn generated_constructor_types_share_import_binding_with_declared_types() {
    let work = tempfile::tempdir().unwrap();
    let fixture = work.path().join("fixture");
    fs::create_dir_all(fixture.join("other")).unwrap();
    fs::write(
        fixture.join("ConstructorBindings.java"),
        r#"
public class ConstructorBindings extends BindingBase {
    other.Token shadow;
    ConstructorBindings() throws BindingException { super((Token) null); }
}
class FailedBindings extends BindingBase {
    other.Token shadow;
    FailedBindings() throws BindingException { super((Token) null); }
}
class BindingBase {
    BindingBase(Token value) throws BindingException {}
    BindingBase(other.Unused value, int ignored) {}
}
class Token {}
class BindingException extends Exception {}
"#,
    )
    .unwrap();
    fs::write(
        fixture.join("other/Token.java"),
        "package other; public class Token {}",
    )
    .unwrap();
    fs::write(
        fixture.join("other/Unused.java"),
        "package other; public class Unused {}",
    )
    .unwrap();
    let original = work.path().join("original");
    compile(&sources(&fixture), &original);
    let mut input = Input::read(&original).unwrap().into_classes();
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&input, &Default::default())
        .unwrap();
    for class in &mut input {
        if class.name == "ConstructorBindings" {
            class.methods.retain(|method| method.name != "<init>");
            class.constant_pool = Default::default();
        } else if class.name == "FailedBindings" {
            // No recoverable original delegation: select and bind the fallback.
            class.constant_pool = Default::default();
            class.methods[0].code.as_mut().unwrap().bytes = vec![0xca];
        }
    }
    let program = Program::new(&input, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let source = work.path().join("source");
    for id in 0..program.own_count() {
        let rendered = decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(
            rendered.failed_methods,
            usize::from(program.class(id).name == "FailedBindings"),
            "{:?}",
            rendered.diagnostics
        );
        if matches!(
            program.class(id).name.as_str(),
            "ConstructorBindings" | "FailedBindings"
        ) {
            assert!(
                rendered.source.contains("super((Token) null);"),
                "{}",
                rendered.source
            );
            assert!(
                rendered.source.contains("other.Token shadow;"),
                "{}",
                rendered.source
            );
            assert!(
                !rendered.source.contains("import other.Token;"),
                "{}",
                rendered.source
            );
            assert!(
                !rendered.source.contains("import other.Unused;"),
                "{}",
                rendered.source
            );
        }
        let path = source.join(format!("{}.java", names.class_name(id)));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, rendered.source).unwrap();
    }
    let files = sources(&source);
    compile(&files, &work.path().join("rebuilt"));
    let api_jar = work.path().join("api.jar");
    api.write_jar(&api_jar).unwrap();
    crate::common::legacy_compile(&files, &work.path().join("legacy"), &api_jar);
}

#[test]
fn same_package_types_win_collisions_under_a_shadowed_package_root() {
    let work = tempfile::tempdir().unwrap();
    let fixture = work.path().join("fixture");
    fs::create_dir_all(fixture.join("Class")).unwrap();
    fs::create_dir_all(fixture.join("media")).unwrap();
    fs::create_dir_all(fixture.join("client")).unwrap();
    fs::write(
        fixture.join("Class/World.java"),
        "package Class; public class World { Player player = new Player(); media.Player audio; public int read() { return Player.read(); } } class Shadowed { Object Player; int read() { return ((Player) null).read(); } }",
    ).unwrap();
    fs::write(
        fixture.join("Class/Player.java"),
        "package Class; public class Player { public static int read() { return 7; } }",
    )
    .unwrap();
    fs::write(fixture.join("client/Consumer.java"), "package client; import Class.Player; public class Consumer { Player local; media.Player other; public int read() { return Player.read(); } }").unwrap();
    fs::write(
        fixture.join("media/Player.java"),
        "package media; public class Player {}",
    )
    .unwrap();
    let original = work.path().join("original");
    compile(&sources(&fixture), &original);
    let input = Input::read(&original).unwrap().into_classes();
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&input, &Default::default())
        .unwrap();
    let program = Program::new(&input, &api.classes).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let source = work.path().join("source");
    for id in 0..program.own_count() {
        let rendered = decompile_class(&program, &names, id, &Default::default()).unwrap();
        assert_eq!(rendered.failed_methods, 0, "{:?}", rendered.diagnostics);
        if program.class(id).name == "Class/World" {
            for spelling in [
                "Player player;",
                "new Player()",
                "Player.read()",
                "media.Player audio;",
            ] {
                assert!(
                    rendered.source.contains(spelling),
                    "missing {spelling}: {}",
                    rendered.source
                );
            }
            assert!(
                !rendered.source.contains("Class.Player"),
                "{}",
                rendered.source
            );
        }
        if program.class(id).name == "Class/Shadowed" {
            assert!(
                rendered.source.contains("((Player) null).read()"),
                "{}",
                rendered.source
            );
        }
        if program.class(id).name == "client/Consumer" {
            assert!(
                rendered.source.contains("import Class.Player;"),
                "{}",
                rendered.source
            );
            assert!(
                rendered.source.contains("media.Player other;"),
                "{}",
                rendered.source
            );
        }
        let path = source.join(format!("{}.java", names.class_name(id)));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, rendered.source).unwrap();
    }
    let files = sources(&source);
    compile(&files, &work.path().join("rebuilt"));
    let api_jar = work.path().join("api.jar");
    api.write_jar(&api_jar).unwrap();
    crate::common::legacy_compile(&files, &work.path().join("legacy"), &api_jar);
}

#[test]
fn type_names_ignore_fields_while_expression_qualifiers_preserve_binding() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(&sources(Path::new("tests/fixtures/type_names")), &original);
    let mut input = Input::read(&original).unwrap().into_classes();
    // A library name is fixed. Unlike an own class, it cannot be renamed away
    // from the package prefix whose qualified references must remain usable.
    let external = input
        .iter()
        .position(|class| class.name == "typesupport/prefix")
        .unwrap();
    let external = input.remove(external);
    let api = ApiCatalog::bundled()
        .unwrap()
        .select_with_classpath(&input, std::slice::from_ref(&external), &Default::default())
        .unwrap();
    let mut libraries = api.classes.clone();
    libraries.push(external);
    let program = Program::new(&input, &libraries).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    for id in 0..program.own_count() {
        let output = decompile_class(&program, &names, id, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        let path = source.join(format!("{}.java", names.class_name(id)));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, output.source).unwrap();
    }
    let text = fs::read_to_string(source.join("typescope/Contexts.java")).unwrap();
    for spelling in [
        "import typesupport.Widget;",
        "import typesupport.Token;",
        "Widget current;",
        "Widget create()",
        "new Widget(",
        "Widget cast(Object Widget)",
        "(Widget) Widget",
        "Token inherited(Object Token)",
        "(Token) Token",
        "Widget[] array(Widget Widget)",
        "new Widget[]",
        "instanceof Widget",
        "Widget.class",
        "String copy(String String)",
        "new String(String)",
        "java.lang.String.valueOf(String)",
        "typesupport.Widget.total",
        "typesupport.Widget.read()",
        "typesupport.Token.number",
        "typesupport.Token.read()",
        "typesupport.prefix value",
        "prefix.left.Item left",
        "prefix.right.Item right",
        "typesupport.prefix.number",
    ] {
        assert!(text.contains(spelling), "missing {spelling:?}\n{text}");
    }
    let create = crate::common::source::method(&text, "create");
    assert!(create.contains("new Widget(Trace.value "), "{create}");
    assert_eq!(create.matches('?').count(), 1, "{create}");
    for argument in ["Trace.record(2)", "Trace.record(3)"] {
        assert!(create.contains(argument), "{create}");
    }
    assert!(!text.contains("import typesupport.prefix;"), "{text}");
    assert!(
        !text.contains("$sf$initialize$"),
        "the intact allocation evaluates before its conditional argument: {text}"
    );
    let mut files = sources(&source);
    files.push("tests/fixtures/type_names/typesupport/prefix.java".into());
    compile(&files, &rebuilt);
    let execute = |path: &Path| {
        checked(
            Command::new(crate::common::java())
                .arg("-cp")
                .arg(path)
                .arg("typescope.Driver"),
        )
    };
    let expected = execute(&original);
    assert_eq!(expected, execute(&rebuilt));

    if let Some(compiler) = crate::common::legacy_javac() {
        let api_jar = work.path().join("api.jar");
        api.write_jar(&api_jar).unwrap();
        let legacy = work.path().join("legacy");
        fs::create_dir_all(&legacy).unwrap();
        checked(
            crate::common::legacy_command(compiler, &api_jar)
                .arg("-d")
                .arg(&legacy)
                .args(&files),
        );
        assert_eq!(expected, execute(&legacy));
    }
}

#[test]
fn semantic_imports_follow_bound_constants_without_polluting_unrelated_classes() {
    use sporeflower::{
        java::domain_sources,
        mappings::{MappingSource, parse_sources},
    };

    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let generated = work.path().join("generated");
    let probe = work.path().join("Probe.java");
    let external = work.path().join("ExternalConstants.java");
    fs::write(&probe, "package imports; public class Probe { public static int generated() { return 7; } public static int existing() { return 9; } } class Plain { int[] values; int ordinary() { return 42; } }").unwrap();
    fs::write(
        &external,
        "package external; public class ExternalConstants { public static final int NINE = 9; }",
    )
    .unwrap();
    compile(&[probe, external.clone()], &original);
    let mut input = Input::read(&original).unwrap().into_classes();
    let external_index = input
        .iter()
        .position(|class| class.name == "external/ExternalConstants")
        .unwrap();
    let external_class = input.remove(external_index);
    let program = Program::new(&input, std::slice::from_ref(&external_class)).unwrap();
    let probe = program.id("imports/Probe").unwrap();
    let plain = program.id("imports/Plain").unwrap();
    let mappings = parse_sources(&program, &[
        MappingSource::new("domains.map", "package semantic; @ValueDomain interface Kind { int SEVEN = 7; } @ValueDomain interface External {}"),
        MappingSource::new("external.map", "package external; @External class ExternalConstants { @DomainValue(semantic.External.class) int NINE; }"),
        MappingSource::new("probe.map", "package imports; class Probe { @Domain(semantic.Kind.class) int generated() /* was generated */; @Domain(semantic.External.class) int existing() /* was existing */; }"),
    ]).unwrap();
    let names =
        NamePlan::build_with_overrides(&program, &RenameOptions::default(), &mappings.names)
            .unwrap();
    let baseline = decompile_class(&program, &names, plain, &JavaOptions::default()).unwrap();
    let options = JavaOptions {
        mappings: Some(&mappings),
        ..JavaOptions::default()
    };
    let unrelated = decompile_class(&program, &names, plain, &options).unwrap();
    assert_eq!(unrelated.failed_methods, 0, "{:?}", unrelated.diagnostics);
    assert_eq!(
        unrelated.source, baseline.source,
        "unused semantic owners must not add imports or change ordinary source"
    );
    let consumer = decompile_class(&program, &names, probe, &options).unwrap();
    assert_eq!(consumer.failed_methods, 0, "{:?}", consumer.diagnostics);
    for spelling in [
        "import semantic.Kind;",
        "import external.ExternalConstants;",
        "return Kind.SEVEN;",
        "return ExternalConstants.NINE;",
    ] {
        assert!(
            consumer.source.contains(spelling),
            "missing {spelling:?}\n{}",
            consumer.source
        );
    }
    for (name, source) in [
        (names.class_name(plain).to_owned(), unrelated.source),
        (names.class_name(probe).to_owned(), consumer.source),
    ]
    .into_iter()
    .chain(domain_sources(&mappings, &program, &names).unwrap())
    {
        let path = generated.join(format!("{name}.java"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let mut files = sources(&generated);
    files.push(external);
    compile(&files, &work.path().join("rebuilt"));
}
