use crate::common::{compile, execute, files, legacy_compile, source};
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    classfile::ClassFile,
    program::Program,
};
use std::{fs, path::PathBuf};

#[test]
fn legal_direct_rethrows_use_emitted_declarations_and_lexical_catches() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let output = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    compile(
        &[PathBuf::from("tests/fixtures/java/DirectRethrows.java")],
        &original,
        None,
    );
    let expected = execute(&original, "DirectRethrows");
    let mut classes: Vec<_> = files(&original, "class")
        .iter()
        .map(|path| ClassFile::parse(&fs::read(path).unwrap()).unwrap())
        .collect();
    // Obfuscators drop these attributes. The declared() witness still obtains
    // Exception from its explicit parameter throw; the other declarations must
    // be inferred from the actual bytecode and emitted catches.
    for class in &mut classes {
        for method in &mut class.methods {
            method.exceptions.clear();
        }
    }
    let api = ApiCatalog::bundled()
        .unwrap()
        .select(&classes, &ApiOptions::default())
        .unwrap();
    let program = Program::new(&classes, &api.classes).unwrap();
    let names = sporeflower::naming::NamePlan::build(&program, &Default::default()).unwrap();
    fs::create_dir_all(&output).unwrap();
    let mut sources = Vec::new();
    for (class, definition) in classes.iter().enumerate() {
        let emitted =
            sporeflower::java::decompile_class(&program, &names, class, &Default::default())
                .unwrap();
        assert_eq!(emitted.failed_methods, 0, "{:?}", emitted.diagnostics);
        if definition.name == "DirectRethrows" {
            for method in ["declared", "caught"] {
                let body = source::method(&emitted.source, method);
                assert!(!body.contains("$sf$rethrow"), "{body}");
                assert!(body.contains("throw "), "{body}");
            }
            for method in ["withFinally", "siblings", "fromCatch", "narrowCatch"] {
                let body = source::method(&emitted.source, method);
                assert!(body.contains("$sf$rethrow"), "{body}");
            }
        }
        let path = output.join(format!("{}.java", names.class_name(class)));
        fs::write(&path, emitted.source).unwrap();
        sources.push(path);
    }
    let api_jar = work.path().join("api.jar");
    api.write_jar(&api_jar).unwrap();
    if !legacy_compile(&sources, &rebuilt, &api_jar) {
        compile(&sources, &rebuilt, None);
    }
    assert_eq!(expected, execute(&rebuilt, "DirectRethrows"));
}
