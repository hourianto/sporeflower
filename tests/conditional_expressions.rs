//! Lazy expression regions retain selected effects, shared values and catch state.
use crate::common::source::method;
use crate::common::{checked, sources};
use std::{fs, path::Path, process::Command};

#[test]
fn loop_carried_conditional_effects_roundtrip() {
    crate::common::fixture_roundtrip("ConditionalEffects");
}

#[test]
fn return_ladders_remain_statements_while_stack_choices_remain_expressions() {
    let work = tempfile::tempdir().unwrap();
    for debug in [false, true] {
        let root = work.path().join(if debug { "debug" } else { "stripped" });
        let original = root.join("original");
        let source = root.join("source");
        let api = root.join("api.jar");
        fs::create_dir_all(&original).unwrap();
        checked(
            crate::common::compile_command()
                .arg(if debug { "-g" } else { "-g:none" })
                .arg("-d")
                .arg(&original)
                .arg("tests/fixtures/java/ReturnChoices.java"),
        );
        let expected = crate::common::execute(&original, "ReturnChoices");
        checked(
            Command::new(env!("CARGO_BIN_EXE_sporeflower"))
                .arg(&original)
                .arg(&source)
                .arg("--stubs")
                .arg(&api)
                .args(["--threads", "1"]),
        );
        let text = fs::read_to_string(source.join("ReturnChoices.java")).unwrap();
        for name in ["guardedReturns", "nestedReturns", "referenceReturns"] {
            let body = method(&text, name);
            assert!(body.matches("if (").count() >= 2, "{name}: {body}");
            assert!(!body.contains(" ? "), "{name}: {body}");
        }
        let joined = method(&text, "joinedChoice");
        assert_eq!(joined.matches(" ? ").count(), 2, "{joined}");
        assert!(!joined.contains("if ("), "{joined}");
        let single = method(&text, "singleChoice");
        assert!(single.contains(" ? "), "{single}");
        let large = method(&text, "largeReturns");
        assert!(large.contains("if ("), "{large}");
        assert!(!large.contains('?'), "{large}");
        let large_joined = method(&text, "largeJoined");
        assert!(!large_joined.contains("if ("), "{large_joined}");
        assert!(
            large_joined.contains("? new ReturnChoices("),
            "{large_joined}"
        );
        let mixed = method(&text, "mixedChoice");
        assert!(mixed.contains("if ("), "{mixed}");
        assert_eq!(mixed.matches(" ? ").count(), 1, "{mixed}");
        let boolean = method(&text, "booleanReturns");
        assert!(!boolean.contains("if ("), "{boolean}");
        for legacy in [false, true] {
            let rebuilt = root.join(if legacy { "legacy" } else { "modern" });
            if legacy {
                if !crate::common::legacy_compile(&sources(&source), &rebuilt, &api) {
                    continue;
                }
            } else {
                crate::common::compile(&sources(&source), &rebuilt, None);
            }
            assert_eq!(
                expected,
                crate::common::execute(&rebuilt, "ReturnChoices"),
                "debug={debug}, legacy={legacy}\n{text}"
            );
        }
    }
}

fn run(classes: &Path) -> Vec<u8> {
    checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(classes)
            .arg("ConditionalExpressions"),
    )
    .stdout
}

#[test]
fn selected_effects_and_constructor_arguments_roundtrip() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    for path in [&original, &rebuilt] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ConditionalExpressions.java"),
    );
    let expected = run(&original);
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("ConditionalExpressions.java")).unwrap();
    for name in [
        "calls",
        "ordered",
        "conditionEffect",
        "preceding",
        "following",
        "nested",
        "reused",
        "allocation",
        "guarded",
    ] {
        let body = method(&text, name);
        assert!(
            body.contains(" ? "),
            "{name} did not recover a conditional: {text}"
        );
        assert!(!body.contains("if ("), "{name} retained a branch: {text}");
    }
    assert!(method(&text, "caught").contains(" ? "), "{text}");
    for (name, operator) in [("selectedTruth", "&&"), ("selectedZero", "||")] {
        let body = method(&text, name);
        assert!(body.contains(operator), "numeric truth consumer: {body}");
        assert!(
            !body.contains('?'),
            "numeric choice survived its truth test: {body}"
        );
    }
    assert!(
        method(&text, "selectedNonzeroArms").contains("number("),
        "an effectful selector must survive equal truth outcomes: {text}"
    );
    assert_eq!(
        method(&text, "preceding")
            .lines()
            .filter(|line| line.trim_start().starts_with("int "))
            .count(),
        1,
        "only the unconditional predecessor needs a local: {text}"
    );
    assert!(method(&text, "catchMutation").contains("if ("), "{text}");
    assert!(method(&text, "shared").contains("if ("), "{text}");
    assert!(!method(&text, "shared").contains(" ? "), "{text}");
    let shared = method(&text, "shared");
    let definition = shared
        .lines()
        .find(|line| line.contains("= ConditionalExpressions.checked(6);"))
        .unwrap();
    let local = definition
        .split('=')
        .next()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap();
    assert!(
        shared.contains(&format!(
            "combine({local}, {local}, ConditionalExpressions.number(7))"
        )),
        "the selected definition must be evaluated once and reused: {shared}"
    );
    assert!(
        shared.find(definition.trim()).unwrap() < shared.find("combine(").unwrap(),
        "the shared value needs its declaration before the consumer: {shared}"
    );
    assert!(method(&text, "reordered").contains("if ("), "{text}");
    for name in ["sharedAlternatives", "sharedAfterChoice"] {
        assert!(
            method(&text, name).contains("if ("),
            "common definitions need an enclosing scope: {text}"
        );
        assert!(
            method(&text, name).contains(" ? "),
            "the inner choice remains an expression: {text}"
        );
    }
    let constructor = fs::read_to_string(source.join("ConditionalMessage.java")).unwrap();
    assert_eq!(constructor.matches(" ? ").count(), 3, "{constructor}");
    assert!(
        constructor
            .split_whitespace()
            .collect::<String>()
            .contains("super(("),
        "{constructor}"
    );
    assert!(!constructor.contains("$preinit"), "{constructor}");
    assert!(!constructor.contains("StringBuffer"), "{constructor}");
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(sources(&source)),
    );
    assert_eq!(expected, run(&rebuilt));
    if let Some(compiler) = crate::common::legacy_javac() {
        let legacy = work.path().join("legacy");
        fs::create_dir_all(&legacy).unwrap();
        checked(
            crate::common::legacy_command(compiler, &api)
                .arg("-d")
                .arg(&legacy)
                .args(sources(&source)),
        );
        assert_eq!(expected, run(&legacy));
    }
}

#[test]
fn shared_array_loads_bind_the_emitted_component_type() {
    use sporeflower::{
        classfile::Constant,
        input::Input,
        java::{JavaOptions, decompile_class},
        naming::{NamePlan, RenameOptions},
        program::Program,
    };
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let api = work.path().join("api.jar");
    crate::common::compile(
        &["tests/fixtures/java/ConditionalArrayTypes.java".into()],
        &original,
        None,
    );
    let expected = crate::common::execute(&original, "ConditionalArrayTypes");
    let mut input = Input::read(&original).unwrap().into_classes();
    let class = input
        .iter_mut()
        .find(|class| class.name == "ConditionalArrayTypes")
        .unwrap();
    let member_index = |name, descriptor| {
        class
            .constant_pool
            .entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| {
                if !matches!(
                    entry,
                    Constant::Fieldref { .. } | Constant::Methodref { .. }
                ) {
                    return None;
                }
                let member = class.constant_pool.member_ref(index as u16).unwrap();
                (member.name == name && member.descriptor == descriptor)
                    .then_some((index as u16).to_be_bytes())
            })
            .unwrap()
    };
    let overload = member_index("overload", "(Ljava/lang/Object;)I");
    let field = member_index("value", "Ljava/lang/Object;");
    for method in &mut class.methods {
        // Legacy optimizers share aaload after selecting both array and index.
        // Keep those operands on the stack, preserving the fixture's semantics.
        let mut bytes = vec![
            0x1a, 0x99, 0, 8, 0x2b, 0x1d, 0xa7, 0, 7, 0x2c, 0x1d, 0x04, 0x60, 0x32,
        ];
        match method.name.as_str() {
            "returned" | "mixed" | "arrays" | "related" => bytes.push(0xb0),
            "invoke" => bytes.extend([0xb8, overload[0], overload[1], 0xac]),
            "captured" => bytes.extend([
                0x3a,
                4,
                0x19,
                4,
                0xb8,
                overload[0],
                overload[1],
                0x19,
                4,
                0xb8,
                overload[0],
                overload[1],
                0x60,
                0xac,
            ]),
            "store" => {
                bytes = vec![
                    0x1b, 0x99, 0, 10, 0x2a, 0x2c, 0x15, 4, 0xa7, 0, 9, 0x2a, 0x2d, 0x15, 4, 0x04,
                    0x60, 0x32, 0xb5, field[0], field[1], 0xb1,
                ]
            }
            _ => continue,
        }
        let code = method.code.as_mut().unwrap();
        code.bytes = bytes;
        code.max_stack = 4;
        code.local_variables.clear();
        code.line_numbers.clear();
    }
    let selection = sporeflower::api::ApiCatalog::bundled()
        .unwrap()
        .select(&input, &Default::default())
        .unwrap();
    selection.write_jar(&api).unwrap();
    let program = Program::new(&input, &selection.classes).unwrap();
    let names = NamePlan::build(&program, &RenameOptions::default()).unwrap();
    fs::create_dir_all(&source).unwrap();
    for id in 0..program.own_count() {
        let output = decompile_class(&program, &names, id, &JavaOptions::default()).unwrap();
        assert_eq!(output.failed_methods, 0, "{:?}", output.diagnostics);
        fs::write(
            source.join(format!("{}.java", names.class_name(id))),
            output.source,
        )
        .unwrap();
    }
    let text = fs::read_to_string(source.join("ConditionalArrayTypes.java")).unwrap();
    for name in ["store", "returned", "mixed", "arrays", "related", "invoke"] {
        assert!(method(&text, name).contains(" ? "), "{name}: {text}");
    }
    for legacy in [false, true] {
        let rebuilt = work.path().join(if legacy { "legacy" } else { "modern" });
        if legacy {
            if !crate::common::legacy_compile(&sources(&source), &rebuilt, &api) {
                continue;
            }
        } else {
            crate::common::compile(&sources(&source), &rebuilt, None);
        }
        assert_eq!(
            expected,
            crate::common::execute(&rebuilt, "ConditionalArrayTypes"),
            "legacy={legacy}\n{text}"
        );
        let class = sporeflower::classfile::ClassFile::parse(
            &fs::read(rebuilt.join("ConditionalArrayTypes.class")).unwrap(),
        )
        .unwrap();
        for name in ["invoke", "captured"] {
            let method = class
                .methods
                .iter()
                .find(|method| method.name == name)
                .unwrap();
            let instructions =
                sporeflower::bytecode::decode(&method.code.as_ref().unwrap().bytes).unwrap();
            let descriptors: Vec<_> = instructions
                .iter()
                .filter_map(|instruction| {
                    if instruction.opcode != 0xb8 {
                        return None;
                    }
                    let sporeflower::bytecode::Operand::Constant(index) = instruction.operand
                    else {
                        return None;
                    };
                    let member = class.constant_pool.member_ref(index).unwrap();
                    (member.name == "overload").then_some(member.descriptor)
                })
                .collect();
            assert!(!descriptors.is_empty(), "{name}: {text}");
            assert!(
                descriptors
                    .iter()
                    .all(|descriptor| *descriptor == "(Ljava/lang/Object;)I"),
                "{name}: {descriptors:?}"
            );
        }
    }
}

#[test]
fn conditional_allocations_keep_original_new_boundaries() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    for path in [&original, &rebuilt] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&original)
            .arg("tests/fixtures/java/ConditionalAllocationOrder.java"),
    );
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("ConditionalAllocationOrder.java")).unwrap();
    assert!(method(&text, "insideShared").contains(" ? "), "{text}");
    for name in [
        "beforeShared",
        "beforeSingle",
        "beforeNested",
        "reorderedAllocation",
    ] {
        assert!(
            method(&text, name).contains("if ("),
            "allocation boundary requires statements in {name}: {text}"
        );
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(sources(&source)),
    );
    let mut outputs = vec![rebuilt];
    if let Some(compiler) = crate::common::legacy_javac() {
        let legacy = work.path().join("legacy");
        fs::create_dir_all(&legacy).unwrap();
        checked(
            crate::common::legacy_command(compiler, &api)
                .arg("-d")
                .arg(&legacy)
                .args(sources(&source)),
        );
        outputs.push(legacy);
    }
    let mut cases = Vec::new();
    for name in ["shared", "single", "nested", "inside", "reordered"] {
        cases.push((name, "0", "false"));
        for failure in ["0", "1", "2", "3"] {
            cases.push((name, failure, "true"));
        }
    }
    cases.extend([
        ("nested", "4", "true"),
        ("nested", "5", "true"),
        ("reordered", "6", "true"),
    ]);
    for (name, failure, selected) in cases {
        let execute = |classes: &Path| {
            checked(
                Command::new(crate::common::java())
                    .arg("-cp")
                    .arg(classes)
                    .arg("ConditionalAllocationOrder")
                    .args([name, failure, selected]),
            )
            .stdout
        };
        let expected = execute(&original);
        if selected == "false" {
            assert_eq!(expected, b"empty\n0\n");
        }
        if failure == "0" && selected == "true" {
            let trace = match name {
                "shared" | "single" => "123",
                "nested" => "14235",
                "inside" => "213",
                "reordered" => "2136",
                _ => unreachable!(),
            };
            assert_eq!(expected, format!("created\n{trace}\n").as_bytes());
        }
        for classes in &outputs {
            assert_eq!(
                expected,
                execute(classes),
                "{name}, failure {failure}, selected {selected}, {}",
                classes.display()
            );
        }
    }
}

/// Check bytecode order as well as runtime behavior: successful array allocation
/// is not directly observable by the fixture, but its position is part of the
/// reconstructed expression's throwing/allocation boundary.
fn allocation_precedes_effects(classes: &Path, name: &str) -> bool {
    use sporeflower::{bytecode, classfile::ClassFile};
    let class = ClassFile::parse(&fs::read(classes.join("AllocationBoundaryOrder.class")).unwrap())
        .unwrap();
    let method = class
        .methods
        .iter()
        .find(|method| method.name == name)
        .unwrap();
    let instructions = bytecode::decode(&method.code.as_ref().unwrap().bytes).unwrap();
    let allocation = instructions
        .iter()
        .find(|instruction| matches!(instruction.opcode, 0xbb | 0xbd))
        .unwrap()
        .pc;
    let calls: Vec<_> = instructions
        .iter()
        .filter_map(|instruction| {
            if instruction.opcode != 0xb8 {
                return None;
            }
            let bytecode::Operand::Constant(index) = instruction.operand else {
                return None;
            };
            let member = class.constant_pool.member_ref(index).unwrap();
            matches!(member.name, "effect" | "text").then_some(instruction.pc)
        })
        .collect();
    assert_eq!(calls.len(), 2, "two alternative effects in {name}");
    assert!(
        calls.iter().all(|&pc| pc < allocation) || calls.iter().all(|&pc| pc > allocation),
        "allocation split the conditional in {name}"
    );
    allocation < calls[0]
}

#[test]
fn expression_operations_keep_allocation_before_operands_and_after_precomputed_choices() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    for path in [&original, &rebuilt] {
        fs::create_dir_all(path).unwrap();
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&original)
            .arg("tests/fixtures/java/AllocationBoundaryOrder.java"),
    );
    let api = work.path().join("api.jar");
    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&source)
            .arg("--stubs")
            .arg(&api)
            .args(["--threads", "1"]),
    );
    let text = fs::read_to_string(source.join("AllocationBoundaryOrder.java")).unwrap();
    for name in [
        "beforeObject",
        "insideObject",
        "beforeArray",
        "insideArray",
        "arrayInChoice",
    ] {
        assert!(
            method(&text, name).contains(" ? "),
            "the safe conditional remains readable in {name}: {text}"
        );
    }
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(sources(&source)),
    );
    let mut outputs = vec![rebuilt];
    if let Some(compiler) = crate::common::legacy_javac() {
        let legacy = work.path().join("legacy");
        fs::create_dir_all(&legacy).unwrap();
        checked(
            crate::common::legacy_command(compiler, &api)
                .arg("-d")
                .arg(&legacy)
                .args(sources(&source)),
        );
        outputs.push(legacy);
    }
    for name in ["beforeObject", "insideObject", "beforeArray", "insideArray"] {
        let expected = allocation_precedes_effects(&original, name);
        assert_eq!(expected, name.starts_with("inside"));
        for classes in &outputs {
            assert_eq!(
                allocation_precedes_effects(classes, name),
                expected,
                "{name}: {text}"
            );
        }
    }
    for name in [
        "beforeObject",
        "insideObject",
        "beforeArray",
        "insideArray",
        "arrayInChoice",
    ] {
        for selected in ["false", "true"] {
            for failure in ["0", "1", "2", "4"] {
                let execute = |classes: &Path| {
                    checked(
                        Command::new(crate::common::java())
                            .arg("-cp")
                            .arg(classes)
                            .arg("AllocationBoundaryOrder")
                            .args([name, failure, selected]),
                    )
                    .stdout
                };
                let expected = execute(&original);
                for classes in &outputs {
                    assert_eq!(
                        execute(classes),
                        expected,
                        "{name}, selected {selected}, failure {failure}: {text}"
                    );
                }
            }
        }
    }
}
