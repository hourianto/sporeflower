use super::*;
use crate::{
    index::test_ids::*,
    java::source::{Case, Catch},
};

fn binding(local: LocalId, name: &str) -> Binding {
    Binding {
        local,
        name: name.into(),
        ty: JvmType::Int,
        type_name: "int".into(),
        synthetic: false,
        canonical_boolean: false,
        storage: false,
    }
}

fn call(name: &str) -> Statement {
    Statement::Simple(Simple::Expression(Expr::atom(name).invoke(vec![])))
}

fn declaration(local: LocalId, name: &str) -> Statement {
    Statement::Simple(Simple::Assign {
        binding: binding(local, name),
        value: AssignmentValue::Value(Expr::integer(0, JvmType::Int, "0")),
        declaration: true,
    })
}

fn render(body: Body) -> String {
    let plan = layout(body, Default::default());
    let mut out = CodeWriter::new(1);
    out.open("void run() {");
    out.body(&plan);
    out.close("}");
    assert!(
        out.text.lines().all(|line| line.trim_end() == line),
        "{}",
        out.text
    );
    assert!(!out.text.contains("\n\n\n"), "{}", out.text);
    assert!(!out.text.contains("{\n\n"), "{}", out.text);
    assert!(!out.text.contains("\n\n    }"), "{}", out.text);
    out.text
}

#[test]
fn declaration_groups_stay_together_and_separate_from_executable_work() {
    let text = render(vec![
        call("start"),
        Statement::Simple(Simple::Declare {
            binding: binding(L0, "first"),
            initialize: false,
        }),
        declaration(L1, "second"),
        call("prepare"),
        call("finish"),
    ]);
    assert!(
        text.contains(
            "start();\n\n        int first;\n        int second = 0;\n\n        prepare();\n        finish();"
        ),
        "{text}"
    );
}

#[test]
fn short_methods_and_single_statement_guards_stay_compact() {
    let text = render(vec![
        declaration(L0, "result"),
        Statement::Return(Some(binding(L0, "result").expression())),
    ]);
    assert!(!text.contains("\n\n"), "{text}");

    let text = render(vec![
        Statement::If {
            condition: Expr::atom("done"),
            yes: vec![Statement::Return(None)],
            no: vec![],
        },
        call("finish"),
    ]);
    assert!(!text.contains("\n\n"), "{text}");
}

#[test]
fn substantial_control_flow_is_separated_from_surrounding_work() {
    let text = render(vec![
        call("prepare"),
        Statement::Loop {
            label: Some("outer".into()),
            header: LoopHeader::While(Expr::atom("ready")),
            body: vec![call("read"), call("consume")],
        },
        call("finish"),
    ]);
    assert!(
        text.contains("prepare();\n\n        outer: while (ready) {"),
        "{text}"
    );
    assert!(
        text.contains("read();\n            consume();\n        }\n\n        finish();"),
        "{text}"
    );
}

#[test]
fn single_declarations_stay_with_their_following_control_flow() {
    let text = render(vec![
        declaration(L0, "count"),
        Statement::Loop {
            label: None,
            header: LoopHeader::While(Expr::atom("ready")),
            body: vec![Statement::If {
                condition: Expr::atom("done"),
                yes: vec![Statement::Break(None)],
                no: vec![],
            }],
        },
        call("finish"),
    ]);
    assert!(
        text.contains("int count = 0;\n        while (ready) {"),
        "{text}"
    );
    assert!(text.contains("}\n        }\n\n        finish();"), "{text}");
}

#[test]
fn loop_transfers_stay_attached_to_their_body() {
    for (transfer, text) in [
        (Statement::Break(None), "break;"),
        (Statement::Continue(None), "continue;"),
    ] {
        let output = render(vec![Statement::Loop {
            label: None,
            header: LoopHeader::While(Expr::atom("ready")),
            body: vec![
                Statement::If {
                    condition: Expr::atom("needsUpdate"),
                    yes: vec![call("read"), call("consume")],
                    no: vec![],
                },
                transfer,
            ],
        }]);
        assert!(
            output.contains(&format!("}}\n            {text}")),
            "{output}"
        );
    }
}

#[test]
fn branch_and_exception_continuations_remain_attached_to_their_blocks() {
    let text = render(vec![
        Statement::If {
            condition: Expr::atom("first"),
            yes: vec![call("prepare"), call("read")],
            no: vec![Statement::If {
                condition: Expr::atom("second"),
                yes: vec![call("retry")],
                no: vec![call("skip")],
            }],
        },
        Statement::Try {
            body: vec![call("open"), call("read")],
            catches: vec![Catch {
                local: None,
                catch_type: "java/lang/Exception".into(),
                ty: "Exception".into(),
                name: "failure".into(),
                body: vec![call("recover")],
            }],
            cleanup: Some(vec![call("close")]),
        },
    ]);
    for continuation in [
        "} else if (second) {",
        "} else {",
        "} catch (Exception failure) {",
        "} finally {",
    ] {
        assert!(text.contains(continuation), "{text}");
    }
    assert!(
        text.contains("skip();\n        }\n\n        try {"),
        "{text}"
    );
    assert!(
        text.contains("recover();\n        } finally {\n            close();"),
        "{text}"
    );
}

#[test]
fn switch_labels_and_do_while_conditions_stay_attached() {
    let text = render(vec![
        Statement::Switch {
            label: None,
            value: Selector {
                value: Expr::atom("kind"),
                keys: BTreeMap::from([
                    (1, Expr::integer(1, JvmType::Int, "1")),
                    (2, Expr::integer(2, JvmType::Int, "2")),
                ]),
            },
            cases: vec![
                Case {
                    keys: vec![1, 2],
                    default: false,
                    scoped: false,
                    body: vec![call("consume"), Statement::Break(None)],
                },
                Case {
                    keys: vec![],
                    default: true,
                    scoped: false,
                    body: vec![Statement::Break(None)],
                },
            ],
        },
        Statement::Loop {
            label: None,
            header: LoopHeader::DoWhile(Expr::atom("ready")),
            body: vec![call("read"), call("consume")],
        },
        call("finish"),
    ]);
    assert!(
        text.contains("case 1:\n            case 2:\n                consume();"),
        "{text}"
    );
    assert!(
        text.contains("} while (ready);\n\n        finish();"),
        "{text}"
    );
}

#[test]
fn leading_constants_are_separated_from_state_without_reordering_fields() {
    use crate::{
        classfile::{ClassFile, Constant, Field},
        java::decompile_class,
        naming::NamePlan,
        program::Program,
    };

    let mut class = ClassFile::parse(include_bytes!(
        "../fixtures/legacy/classes/pkg/TestConsecutiveReads.class"
    ))
    .unwrap();
    class.methods.clear();
    let constant = class.constant_pool.entries.len() as u16;
    class.constant_pool.entries.push(Constant::Integer(3));
    class.fields = [
        ("READY", 0x0019, true),
        ("DONE", 0x0019, true),
        ("shared", 0x0009, false),
        ("count", 0x0002, false),
        ("LATE", 0x0019, true),
    ]
    .into_iter()
    .map(|(name, access_flags, initialized)| Field {
        access_flags,
        name: name.into(),
        descriptor: "I".into(),
        constant_value: initialized.then_some(constant),
        synthetic: false,
    })
    .collect();
    let classes = [class];
    let program = Program::new(&classes, &[]).unwrap();
    let names = NamePlan::build(&program, &Default::default()).unwrap();
    let text = decompile_class(&program, &names, 0, &Default::default())
        .unwrap()
        .source;
    assert!(
        text.contains("READY = 3;\n    public static final int DONE = 3;\n\n    public static int shared;\n    private int count;\n    public static final int LATE = 3;"),
        "{text}"
    );
}
