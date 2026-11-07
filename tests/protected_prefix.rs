//! Pure assignments before a protected effect remain visible to catch bodies.
use crate::common::checked;
use sporeflower::{
    input::Input,
    ir::{self, Statement, Terminator, Value, ValueKind},
    structure,
};
use std::{fs, path::Path, process::Command};

fn compile(source: &Path, output: &Path) {
    fs::create_dir_all(output).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(output)
            .arg(source),
    );
}

#[test]
fn protected_prefixes_preserve_effect_order_handlers_values_and_loop_entries() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let generated = work.path().join("generated");
    let rebuilt = work.path().join("rebuilt");
    compile(
        Path::new("tests/fixtures/java/ProtectedPrefix.java"),
        &original,
    );
    let input = Input::read(&original).unwrap();
    let class = input
        .classes()
        .iter()
        .find(|class| class.name == "ProtectedPrefix")
        .unwrap();
    let nested = class
        .methods
        .iter()
        .find(|method| method.name == "nested")
        .unwrap();
    let nested_ir = ir::decompile_method(class, nested).unwrap();
    let before_count = nested_ir.blocks.len();
    let prepared = structure::PreparedIr::new(nested_ir);
    let nested_ir = prepared.ir();
    assert!(
        nested_ir.blocks.len() > before_count,
        "outer monitor entry must move to an appended tail"
    );
    let tree = structure::structure(&prepared).unwrap();
    let monitors = nested_ir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| matches!(statement, Statement::Monitor { .. }))
        .count();
    assert_eq!(
        tree.suppressed_monitors.len(),
        monitors,
        "validated nesting must not depend on arena block IDs"
    );
    let method = class
        .methods
        .iter()
        .find(|method| method.name == "terminal")
        .unwrap();
    let mut ir = ir::decompile_method(class, method).unwrap();
    let before = ir.clone();
    structure::split_nonthrowing_prefixes(&mut ir);
    assert!(
        ir.blocks.len() > before.blocks.len(),
        "fixture must exercise a mixed protected block"
    );
    assert_eq!(format!("{:?}", before.values), format!("{:?}", ir.values));
    for (block, original) in before.blocks.iter_enumerated() {
        if let Terminator::Goto(tail) = ir.blocks[block].terminator
            && !before.blocks.contains_id(tail)
        {
            assert!(ir.blocks[block].handlers.is_empty());
            assert_eq!(original.handlers, ir.blocks[tail].handlers);
            assert_eq!(original.pc, ir.blocks[block].pc);
            assert_eq!(original.pc, ir.blocks[tail].pc);
            assert_eq!(
                before.blocks[block].caught_exception,
                ir.blocks[block].caught_exception
            );
            assert_eq!(ir.blocks[tail].caught_exception, None);
        }
    }
    let once = format!("{:?}", ir.blocks);
    structure::split_nonthrowing_prefixes(&mut ir);
    assert_eq!(
        once,
        format!("{:?}", ir.blocks),
        "preparation must be idempotent"
    );

    // An inline capture evaluates its own RHS even when that ValueId also has
    // a local binding. It cannot be mistaken for a pure read and leave the try.
    let mut capture = before;
    let call = capture
        .values
        .iter_enumerated()
        .find_map(|(id, value)| {
            matches!(&value.kind, ValueKind::Call { method, .. } if method.name == "effect")
                .then_some(id)
        })
        .unwrap();
    let block = capture.values[call].block;
    let local = capture
        .locals
        .iter_enumerated()
        .find_map(|(id, local)| (!local.is_this).then_some(id))
        .unwrap();
    capture.values[call].local = Some(local);
    let value = capture.values.next_id();
    capture.values.push(Value {
        local: None,
        kind: ValueKind::InlineAssign { local, value: call },
        ty: capture.values[call].ty.clone(),
        pc: capture.values[call].pc,
        block,
    });
    capture.blocks[block].statements.insert(
        0,
        Statement::Assign {
            local,
            value: sporeflower::ir::ValueUse::Reference(value),
        },
    );
    let handlers = capture.blocks[block].handlers.clone();
    structure::split_nonthrowing_prefixes(&mut capture);
    assert!(!handlers.is_empty());
    assert_eq!(capture.blocks[block].handlers, handlers);

    checked(
        Command::new(env!("CARGO_BIN_EXE_sporeflower"))
            .arg(&original)
            .arg(&generated)
            .args(["--no-api", "--threads", "1"]),
    );
    let source_path = generated.join("ProtectedPrefix.java");
    let source = fs::read_to_string(&source_path).unwrap();
    let start = source.find("static int terminal(").unwrap();
    let end = start + source[start..].find("\n    }").unwrap();
    let terminal = &source[start..end];
    assert!(
        terminal.find(" + 2;").unwrap() < terminal.find("try {").unwrap(),
        "{terminal}"
    );
    assert!(
        !terminal.contains(" = 0;"),
        "a default would conceal the real initialization: {terminal}"
    );
    let start = source.find("static int store(").unwrap();
    let end = start + source[start..].find("\n    }").unwrap();
    let store = &source[start..end];
    assert!(store.contains("++] = ProtectedPrefix.effect("), "{store}");
    compile(&source_path, &rebuilt);
    let expected = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&original)
            .arg("ProtectedPrefix"),
    )
    .stdout;
    let actual = checked(
        Command::new(crate::common::java())
            .arg("-cp")
            .arg(&rebuilt)
            .arg("ProtectedPrefix"),
    )
    .stdout;
    assert_eq!(expected, actual);
}
