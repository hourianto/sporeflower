use super::*;
use crate::classfile::{ClassFile, Code, Constant, ConstantPool, Field, Method};
use crate::program::{MemberId, Program};

fn class(name: &str) -> ClassFile {
    ClassFile {
        minor_version: 0,
        major_version: 49,
        access_flags: 1,
        name: name.into(),
        super_name: None,
        interfaces: vec![],
        constant_pool: ConstantPool::default(),
        fields: vec![],
        methods: vec![],
        source_file: None,
        inner_classes: vec![],
        warnings: vec![],
    }
}
fn field(name: &str, descriptor: &str) -> Field {
    Field {
        access_flags: 1,
        name: name.into(),
        descriptor: descriptor.into(),
        constant_value: None,
        synthetic: false,
    }
}
fn method(name: &str, descriptor: &str) -> Method {
    Method {
        access_flags: 1,
        name: name.into(),
        descriptor: descriptor.into(),
        code: None,
        exceptions: vec![],
        synthetic: false,
    }
}
fn source(text: &str) -> MappingSource {
    MappingSource::new("case.map", text)
}

#[test]
fn mapped_types_and_wide_parameter_ordinals_keep_original_identities() {
    let mut first = class("old/a");
    first.fields.push(field("b", "[Lold/b;"));
    first.methods.push(method("a", "(JLold/b;)Lold/b;"));
    first.methods.push(method("<init>", "(I)V"));
    let own = [first, class("old/b")];
    let program = Program::new(&own, &[]).unwrap();
    let plan = parse_sources(
        &program,
        &[source(
            r#"
        package renamed;
        class Item /* was old/b */ {}
        class Owner /* was old/a */ {
            Item[] items /* was b */;
            Item find(long elapsed, Item selected) /* was a */;
            Owner(int capacity) {}
        }
    "#,
        )],
    )
    .unwrap();
    assert_eq!(plan.names.classes[&0], "renamed/Owner");
    assert_eq!(plan.names.fields[&MemberId { class: 0, index: 0 }], "items");
    assert_eq!(
        plan.names.parameters[&MemberId { class: 0, index: 0 }],
        ["elapsed", "selected"]
    );
    assert_eq!(
        program.method(MemberId { class: 0, index: 0 }).descriptor,
        "(JLold/b;)Lold/b;"
    );
}

#[test]
fn all_contract_shapes_are_parsed_and_typed() {
    let mut owner = class("a");
    for (name, descriptor) in [
        ("a", "[[I"),
        ("b", "[I"),
        ("c", "[I"),
        ("d", "I"),
        ("e", "Ljava/util/Vector;"),
        ("f", "Ljava/util/Hashtable;"),
        ("g", "Ljava/lang/String;"),
    ] {
        owner.fields.push(field(name, descriptor));
    }
    for (name, descriptor) in [("a", "(I)I"), ("b", "([II)I"), ("c", "(II)I")] {
        owner.methods.push(method(name, descriptor));
    }
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let plan=parse_sources(&program,&[source(r#"
        @ValueDomain interface Kind { int ONE = 1; int TWO = 2; }
        @FlagDomain(exclusiveMasks = {3}) interface Mask { int A = 1; int B = 2; int ENABLED = 4; }
        @SlotDomain interface Slot { @SlotValue(Kind.class) int KIND = 0; int COUNT = 1; }
        @PackedDomain @BitField(name = "kind", value = Kind.class, bits = 2) interface Packed {}
        @NumericDomain(format = "fixed", fractionBits = 8, unit = "px") interface Fixed {}
        @StringDomain interface Token { String EMPTY = ""; String LINE = "a\nb"; }
        class Example /* was a */ {
            @IndexDomain(value=Kind.class, dimension=0) @Slots(value=Slot.class,dimension=1)
            @Domain(Kind.class) int[][] rows /* was a */;
            @Records(value=Slot.class,stride=2) int[] records /* was b */;
            @Planes(value=Slot.class,stride=100) int[] planes /* was c */;
            @Domain(Packed.class) int packed /* was d */;
            @Elements(Kind.class) java.util.Vector elements /* was e */;
            @Keys(Token.class) @Values(Kind.class) java.util.Hashtable table /* was f */;
            @ClassName String className /* was g */;
            @DomainFromParameter(0) int identity(int value) /* was a */;
            @DomainFromSlot(parameter=0,slot=1) int lookup(int[] table,@DomainFromSlot(parameter=0,slot=0) int key) /* was b */;
            @DomainWhen(value=Kind.class,parameter=0,equals=1) @DomainWhen(value=Kind.class,parameter=0,otherwise=true)
            int conditional(int selector,@DomainWhen(value=Kind.class,parameter=0,notEquals=0) int payload) /* was c */;
        }
    "#)]).unwrap();
    assert_eq!(plan.domains.len(), 6);
    assert_eq!(plan.bindings.len(), 12);
    let rows = plan
        .contract(Target::Field(MemberId { class: 0, index: 0 }))
        .unwrap();
    assert!(rows.leaf.is_some());
    assert!(rows.arrays[&0].index.is_some());
    assert!(rows.arrays[&1].slots.is_some());
    assert!(matches!(
        plan.domains[4].format,
        Some(NumberFormat::Fixed {
            fraction_bits: 8,
            ..
        })
    ));
    assert_eq!(
        plan.domains[5].values[1].value,
        ConstantValue::String("a\nb".into())
    );
}

#[test]
fn inheritance_replaces_whole_contract_and_conflicting_interfaces_stay_unbound() {
    let mut base = class("Base");
    base.methods.push(method("m", "([I)I"));
    let mut child = class("Child");
    child.super_name = Some("Base".into());
    child.methods.push(method("m", "([I)I"));
    let mut left = class("Left");
    left.access_flags |= 0x200;
    left.methods.push(method("m", "([I)I"));
    let mut right = class("Right");
    right.access_flags |= 0x200;
    right.methods.push(method("m", "([I)I"));
    let mut ambiguous = class("Ambiguous");
    ambiguous.interfaces = vec!["Left".into(), "Right".into()];
    ambiguous.methods.push(method("m", "([I)I"));
    let own = [base, child, left, right, ambiguous];
    let program = Program::new(&own, &[]).unwrap();
    let plan=parse_sources(&program,&[source(r#"
        @ValueDomain interface A { int X = 1; }
        @ValueDomain interface B { int Y = 2; }
        class Base { @Domain(A.class) int m(@Domain(A.class) int[] a) /* was m */; }
        class Child extends Base { int m(@IndexDomain(value=A.class,dimension=0) int[] a) /* was m */; }
        interface Left { @Domain(A.class) int m(int[] a) /* was m */; }
        interface Right { @Domain(B.class) int m(int[] a) /* was m */; }
    "#)]).unwrap();
    assert!(
        plan.contract(Target::Return(MemberId { class: 1, index: 0 }))
            .unwrap()
            .leaf
            .is_some()
    );
    let child_arg = plan
        .contract(Target::Parameter {
            method: MemberId { class: 1, index: 0 },
            index: 0,
        })
        .unwrap();
    assert!(child_arg.leaf.is_none());
    assert!(child_arg.arrays[&0].index.is_some());
    assert!(
        plan.contract(Target::Return(MemberId { class: 4, index: 0 }))
            .is_none()
    );
    assert_eq!(plan.diagnostics.len(), 1);
    assert!(plan.diagnostics[0].warning);
    assert_eq!(plan.diagnostics[0].origins.len(), 2);
    assert!(plan.diagnostics[0].message.contains("return unbound"));
}

#[test]
fn malformed_contracts_report_file_and_line() {
    let mut owner = class("a");
    owner.fields.push(field("x", "I"));
    owner.fields.push(field("y", "[I"));
    owner.methods.push(method("m", "(I)I"));
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    for (mapping, expected) in [
        (
            "@ValueDomain interface K { int X=1; int Y=1; }",
            "duplicate semantic value",
        ),
        (
            "@ValueDomain interface K {} class A /* was a */ { @Flags(K.class) int x /* was x */; }",
            "incompatible domain",
        ),
        (
            "@ValueDomain interface K {} class A /* was a */ { @IndexDomain(value=K.class,dimension=1) int[] y /* was y */; }",
            "outside the array",
        ),
        (
            "@ValueDomain interface K {} class A /* was a */ { @DomainWhen(value=K.class,parameter=0,otherwise=true) int m(int p) /* was m */; }",
            "requires equality cases",
        ),
        (
            "@PackedDomain @BitField(value=K.class,bits=2) interface K {}",
            "cyclic packed domain",
        ),
        (
            "@NumericDomain(format=\"fixed\",fractionBits=63) interface K {}",
            "0..62",
        ),
        (
            "class A /* was a */ { @MadeUp int x /* was x */; }",
            "unknown mapping annotation",
        ),
        ("class A /* was a */ { int x; }", "missing /* was"),
    ] {
        let error = parse_sources(&program, &[source(mapping)]).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains(expected), "expected {expected} in {text}");
    }
    let error = parse_sources(
        &program,
        &[source(
            "class A /* was a */ {\n @Domain(Unknown.class) int x /* was x */;\n}",
        )],
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("case.map:2"));
}

#[test]
fn call_contracts_require_exact_invocation_offsets_and_argument_ordinals() {
    let mut owner = class("a");
    owner.constant_pool.entries = vec![
        Constant::Unusable,
        Constant::Utf8("a".into()),
        Constant::Class(1),
        Constant::Utf8("callee".into()),
        Constant::Utf8("(I)I".into()),
        Constant::NameAndType {
            name_index: 3,
            descriptor_index: 4,
        },
        Constant::Methodref {
            class_index: 2,
            name_and_type_index: 5,
        },
    ];
    let mut caller = method("caller", "()V");
    caller.code = Some(Code {
        max_stack: 1,
        max_locals: 1,
        bytes: vec![0x04, 0xb8, 0, 6, 0x57, 0xb1],
        exception_table: vec![],
        line_numbers: vec![],
        local_variables: vec![],
    });
    owner.methods = vec![caller, method("callee", "(I)I")];
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let mapping = |offset, parameter| {
        format!(
            "@ValueDomain interface K {{ int ONE=1; }} class A /* was a */ {{ @CallDomain(value=K.class,offset={offset}{parameter}) void caller() /* was caller */; }}"
        )
    };
    let plan = parse_sources(&program, &[source(&mapping(1, ",parameter=0"))]).unwrap();
    assert_eq!(
        plan.call_domain(MemberId { class: 0, index: 0 }, 1, Some(0)),
        Some(DomainId(0))
    );
    assert!(parse_sources(&program, &[source(&mapping(2, ""))]).is_err());
    assert!(parse_sources(&program, &[source(&mapping(1, ",parameter=1"))]).is_err());
}

#[test]
fn lexer_keeps_was_identity_and_java_literal_values() {
    let parsed=parser::parse(&source(r#"
        package example;
        // Documentation may mention @notAnAnnotation or /* was text.
        @ValueDomain interface N { int LOW = 0x80000000; long ALL = 0xffffffffffffffffL; char TAB = '\t'; }
        import java.lang.String;
        class Child extends Parent /* was do */ { static @Domain(N.class) void use(final @Domain(N.class) int... values) /* was if */; }
    "#)).unwrap();
    assert_eq!(parsed.declarations[1].original.as_deref(), Some("do"));
    assert_eq!(
        parsed.declarations[1].members[0]
            .parameters
            .as_ref()
            .unwrap()[0]
            .ty
            .dimensions,
        1
    );
    assert!(matches!(
        parsed.declarations[0].members[0].initializer,
        Some(parser::Value::Integer(-2147483648))
    ));
}

#[test]
fn api_roots_include_real_constant_providers_before_binding() {
    let mut owner = class("a");
    owner
        .fields
        .push(field("graphics", "Ljavax/microedition/lcdui/Graphics;"));
    let own = [owner];
    let inputs = MappingInputs::default();
    let roots = inputs.required_api_roots(&own, true).unwrap();
    assert!(
        roots
            .iter()
            .any(|name| name == "javax/microedition/lcdui/game/Sprite")
    );
    assert!(
        !roots
            .iter()
            .any(|name| name.starts_with("javax/microedition/m3g/"))
    );
    let libraries = crate::api::ApiCatalog::bundled()
        .unwrap()
        .select(
            &own,
            &crate::api::ApiOptions {
                additional_roots: roots,
                ..Default::default()
            },
        )
        .unwrap();
    let program = Program::new(&own, &libraries.classes).unwrap();
    let plan = inputs.resolve(&program, true).unwrap();
    let transform = plan
        .domains
        .iter()
        .find(|domain| domain.owner == "javax/microedition/lcdui/game/SpriteTransform")
        .unwrap();
    assert_eq!(transform.values.len(), 8);
    assert!(transform.values.iter().all(|value| value.source.is_some()));
    assert!(inputs.required_api_roots(&own, false).unwrap().is_empty());
}

#[test]
fn authored_builtin_imports_request_providers_without_original_bytecode_references() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("Contract.map");
    std::fs::write(&path, "import javax.microedition.lcdui.game.SpriteTransform; class A /* was a */ { @Domain(SpriteTransform.class) int transform /* was x */; }").unwrap();
    let mut owner = class("a");
    owner.fields.push(field("x", "I"));
    let own = [owner];
    let inputs = MappingInputs::read(&[path]).unwrap();
    let roots = inputs.required_api_roots(&own, true).unwrap();
    assert!(
        roots
            .iter()
            .any(|name| name == "javax/microedition/lcdui/game/Sprite")
    );
    assert!(!roots.iter().any(|name| name.ends_with("SpriteTransform")));
    let libraries = crate::api::ApiCatalog::bundled()
        .unwrap()
        .select(
            &own,
            &crate::api::ApiOptions {
                additional_roots: roots,
                ..Default::default()
            },
        )
        .unwrap();
    let program = Program::new(&own, &libraries.classes).unwrap();
    let plan = inputs.resolve(&program, true).unwrap();
    assert!(
        plan.contract(Target::Field(MemberId { class: 0, index: 0 }))
            .unwrap()
            .leaf
            .is_some()
    );
}

#[test]
fn builtin_authority_and_partial_api_members_are_independent_of_input_order() {
    let mut library = class("api/Device");
    library.methods.push(method("set", "(I)V"));
    let libraries = [library];
    let program = Program::new(&[], &libraries).unwrap();
    let authored = source(
        "package api; @ValueDomain interface UserMeaning { int X=1; } @External class Device { void set(@Domain(UserMeaning.class) int value); }",
    );
    let builtin = MappingSource { path: "builtin/test.map".into(), builtin: true, source: "package api; @ValueDomain interface ApiMeaning { int X=1; } @External class Device { void set(@Domain(ApiMeaning.class) int value); void missing(UnavailableType value); }".into() };
    let plan = parse_sources(&program, &[authored, builtin]).unwrap();
    let contract = plan
        .contract(Target::Parameter {
            method: MemberId { class: 0, index: 0 },
            index: 0,
        })
        .unwrap();
    assert_eq!(plan.domain(contract.leaf.unwrap()).owner, "api/ApiMeaning");
    assert_eq!(plan.diagnostics.len(), 1);
    assert!(plan.diagnostics[0].warning);
    assert_eq!(plan.diagnostics[0].origins.len(), 2);
    assert!(
        plan.diagnostics[0]
            .to_string()
            .contains("--no-builtin-mappings")
    );
}

#[test]
fn syntax_errors_recover_at_members_and_accumulate_across_files() {
    let program = Program::new(&[], &[]).unwrap();
    let error = parse_sources(
        &program,
        &[
            MappingSource::new("first.map", "class A {\n  int ;\n  String ;\n}"),
            MappingSource::new("second.map", "class B {\n  void ;\n}"),
        ],
    )
    .unwrap_err();
    let batch = error.downcast_ref::<MappingDiagnostics>().unwrap();
    assert_eq!(batch.issues.len(), 3);
    let rendered = error.to_string();
    assert!(rendered.contains("first.map:2:7"), "{rendered}");
    assert!(rendered.contains("second.map:2:8"), "{rendered}");
    assert!(rendered.contains("expected identifier; found ';'"));
    assert!(rendered.contains("int ;\n     |       ^"));
}

#[test]
fn missing_members_show_each_declaration_and_real_overloads() {
    let mut owner = class("a");
    owner.methods.push(method("m", "(I)I"));
    owner.methods.push(method("m", "(J)I"));
    owner.fields.push(field("x", "I"));
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let error = parse_sources(&program, &[source("class A /* was a */ {\n  int run(String text) /* was m */;\n  long count /* was x */;\n}")]).unwrap_err();
    // String must itself be known before the method lookup can be attempted.
    assert!(error.to_string().contains("unknown type/domain String"));
    let error = parse_sources(&program, &[source("class A /* was a */ {\n  int run(double value) /* was m */;\n  long count /* was x */;\n}")]).unwrap_err();
    let batch = error.downcast_ref::<MappingDiagnostics>().unwrap();
    assert_eq!(batch.issues.len(), 2);
    let rendered = error.to_string();
    for expected in [
        "case.map:2:3",
        "case.map:3:3",
        "candidate: a.m(I)I",
        "candidate: a.m(J)I",
        "candidate: a.x:I",
        "hint:",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
}

#[test]
fn domain_and_annotation_errors_point_at_the_offending_member_or_annotation() {
    let program = Program::new(&[], &[]).unwrap();
    let error = parse_sources(
        &program,
        &[source(
            "@StringDomain\ninterface Text {\n  int WRONG = 1;\n  int ALSO_WRONG = 2;\n}",
        )],
    )
    .unwrap_err();
    let batch = error.downcast_ref::<MappingDiagnostics>().unwrap();
    assert_eq!(batch.issues.len(), 2);
    assert_eq!(batch.issues[0].origins[0].line, 3);
    assert_eq!(batch.issues[1].origins[0].line, 4);

    let mut owner = class("a");
    owner.fields.push(field("x", "I"));
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let error = parse_sources(&program, &[source("@ValueDomain interface Kind {}\nclass A /* was a */ {\n  @Domain(Kindd.class)\n  int value /* was x */;\n}")]).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("case.map:3:3"), "{rendered}");
    assert!(rendered.contains("candidate: Kind"), "{rendered}");
}

#[test]
fn invalid_owners_stop_dependent_member_cascades() {
    let program = Program::new(&[], &[]).unwrap();
    let error = parse_sources(
        &program,
        &[
            MappingSource::new("one.map", "class First /* was absent1 */ { Missing type; }"),
            MappingSource::new(
                "two.map",
                "class Second /* was absent2 */ { Missing type; }",
            ),
        ],
    )
    .unwrap_err();
    let batch = error.downcast_ref::<MappingDiagnostics>().unwrap();
    assert_eq!(batch.issues.len(), 2);
    assert!(
        batch
            .issues
            .iter()
            .all(|issue| issue.message.contains("class owner not found"))
    );
}

#[test]
fn duplicate_domain_values_report_both_constants_and_collect_independent_domains() {
    let program = Program::new(&[], &[]).unwrap();
    let error = parse_sources(&program, &[source("@ValueDomain interface A {\n  int ONE = 1;\n  int ALSO_ONE = 1;\n}\n@StringDomain interface B {\n  String FIRST = \"same\";\n  String SECOND = \"same\";\n}")]).unwrap_err();
    let batch = error.downcast_ref::<MappingDiagnostics>().unwrap();
    assert_eq!(batch.issues.len(), 2);
    assert_eq!(
        batch.issues[0]
            .origins
            .iter()
            .map(|origin| origin.line)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        batch.issues[1]
            .origins
            .iter()
            .map(|origin| origin.line)
            .collect::<Vec<_>>(),
        [6, 7]
    );
}

#[test]
fn package_access_errors_retain_the_origins_of_both_placements() {
    let mut hidden = class("p/Hidden");
    hidden.access_flags = 0;
    let mut user = class("p/User");
    user.fields.push(field("value", "Lp/Hidden;"));
    let own = [hidden, user];
    let program = Program::new(&own, &[]).unwrap();
    let plan = parse_sources(
        &program,
        &[
            MappingSource::new(
                "hidden.map",
                "package hidden; class Hidden /* was p/Hidden */ {}",
            ),
            MappingSource::new("user.map", "package user; class User /* was p/User */ {}"),
        ],
    )
    .unwrap();
    let error = crate::naming::NamePlan::build_with_mapping_origins(
        &program,
        &Default::default(),
        &plan.names,
        &plan.origins,
    )
    .unwrap_err();
    let issue = error.downcast_ref::<MappingDiagnostic>().unwrap();
    assert_eq!(issue.origins.len(), 2);
    assert!(issue.message.contains("removes access"));
}

#[test]
fn conflicting_names_preserve_both_authored_origins() {
    let mut owner = class("a");
    owner.fields.push(field("x", "I"));
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let error = parse_sources(
        &program,
        &[
            MappingSource::new("one.map", "class A /* was a */ { int first /* was x */; }"),
            MappingSource::new("two.map", "class A /* was a */ { int second /* was x */; }"),
        ],
    )
    .unwrap_err();
    let batch = error.downcast_ref::<MappingDiagnostics>().unwrap();
    assert_eq!(batch.issues[0].origins.len(), 2);
    let rendered = error.to_string();
    assert!(rendered.contains("one.map:1:"));
    assert!(rendered.contains("two.map:1:"));
}

#[test]
fn name_placement_retains_origins_for_portable_class_and_java_method_collisions() {
    let own = [class("a"), class("b")];
    let program = Program::new(&own, &[]).unwrap();
    let plan = parse_sources(
        &program,
        &[
            MappingSource::new("one.map", "class Item /* was a */ {}"),
            MappingSource::new("two.map", "class item /* was b */ {}"),
        ],
    )
    .unwrap();
    let error = crate::naming::NamePlan::build_with_mapping_origins(
        &program,
        &Default::default(),
        &plan.names,
        &plan.origins,
    )
    .unwrap_err();
    let issue = error.downcast_ref::<MappingDiagnostic>().unwrap();
    assert_eq!(issue.origins.len(), 2);
    assert!(issue.message.contains("collides"));

    let mut owner = class("a");
    owner.methods.push(method("x", "()I"));
    owner.methods.push(method("y", "()J"));
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let plan = parse_sources(
        &program,
        &[source(
            "class A /* was a */ {\n  int same() /* was x */;\n  long same() /* was y */;\n}",
        )],
    )
    .unwrap();
    let error = crate::naming::NamePlan::build_with_mapping_origins(
        &program,
        &Default::default(),
        &plan.names,
        &plan.origins,
    )
    .unwrap_err();
    let issue = error.downcast_ref::<MappingDiagnostic>().unwrap();
    assert_eq!(issue.origins.len(), 2);
    assert!(issue.hint.as_deref().unwrap().contains("return types"));
}

#[test]
fn class_name_array_relocation_and_index_meaning_are_independent() {
    let mut owner = class("a");
    owner.fields.push(field("x", "[Ljava/lang/String;"));
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let plan = parse_sources(&program, &[source("@ValueDomain interface Kind { int FIRST=0; } class A /* was a */ { @ClassName @IndexDomain(value=Kind.class,dimension=0) String[] names /* was x */; }")]).unwrap();
    let contract = plan
        .contract(Target::Field(MemberId { class: 0, index: 0 }))
        .unwrap();
    assert!(contract.class_name);
    assert!(contract.arrays[&0].index.is_some());
}

#[test]
fn legacy_unqualified_mapping_outputs_share_the_defpackage_namespace() {
    let helper = class("a");
    let mut owner = class("raw/Owner");
    owner.fields.push(field("x", "La;"));
    let own = [helper, owner];
    let program = Program::new(&own, &[]).unwrap();
    let plan = parse_sources(
        &program,
        &[
            source("@ValueDomain interface Kind { int FIRST=0; } class Helper /* was a */ {}"),
            source(
                "package readable; class Owner /* was raw/Owner */ { Helper helper /* was x */; }",
            ),
        ],
    )
    .unwrap();
    assert_eq!(plan.names.classes[&0], "defpackage/Helper");
    assert_eq!(plan.domains[0].owner, "defpackage/Kind");
    assert!(plan.names.reserved_classes.contains("defpackage/Kind"));
    let names =
        crate::naming::NamePlan::build_with_overrides(&program, &Default::default(), &plan.names)
            .unwrap();
    assert_eq!(names.class_name(0), "defpackage/Helper");
    assert_eq!(names.class_name(1), "readable/Owner");
    assert_eq!(program.class(0).name, "a");
}

#[test]
fn all_binding_kinds_share_authority_and_duplicate_diagnostics() {
    let mut library = class("api/Device");
    library.constant_pool.entries = vec![
        Constant::Unusable,
        Constant::Utf8("api/Device".into()),
        Constant::Class(1),
        Constant::Utf8("callee".into()),
        Constant::Utf8("(I)I".into()),
        Constant::NameAndType {
            name_index: 3,
            descriptor_index: 4,
        },
        Constant::Methodref {
            class_index: 2,
            name_and_type_index: 5,
        },
        Constant::Integer(1),
    ];
    let mut constant = field("VALUE", "I");
    constant.access_flags = 0x19;
    constant.constant_value = Some(7);
    library.fields = vec![field("target", "I"), constant];
    let mut caller = method("caller", "()V");
    caller.code = Some(Code {
        max_stack: 1,
        max_locals: 1,
        bytes: vec![0x04, 0xb8, 0, 6, 0x57, 0xb1],
        exception_table: vec![],
        line_numbers: vec![],
        local_variables: vec![],
    });
    library.methods = vec![caller, method("callee", "(I)I")];
    let libraries = [library];
    let program = Program::new(&[], &libraries).unwrap();
    let declaration = |domain| {
        format!(
            "@External class Device {{ @Domain({domain}.class) int target; @DomainValue({domain}.class) int VALUE; @CallDomain(value={domain}.class,offset=1,parameter=0) void caller(); }}"
        )
    };
    let builtin = MappingSource {
        path: "builtin.map".into(),
        builtin: true,
        source: format!(
            "package api; @ValueDomain interface Built {{}} {}",
            declaration("Built")
        ),
    };
    let authored = MappingSource::new(
        "authored.map",
        format!(
            "package api; @ValueDomain interface User {{}} {}",
            declaration("User")
        ),
    );
    for sources in [
        [authored.clone(), builtin.clone()],
        [builtin.clone(), authored],
    ] {
        let plan = parse_sources(&program, &sources).unwrap();
        let domain = plan
            .contract(Target::Field(MemberId { class: 0, index: 0 }))
            .unwrap()
            .leaf
            .unwrap();
        assert_eq!(plan.domain(domain).owner, "api/Built");
        assert_eq!(
            plan.real_value(MemberId { class: 0, index: 1 }),
            Some(domain)
        );
        assert_eq!(
            plan.call_domain(MemberId { class: 0, index: 0 }, 1, Some(0)),
            Some(domain)
        );
        assert_eq!(plan.domain(domain).values.len(), 1);
        assert!(
            plan.domains()
                .iter()
                .find(|domain| domain.owner == "api/User")
                .unwrap()
                .values
                .is_empty()
        );
        assert_eq!(plan.diagnostics.len(), 3);
        assert!(
            plan.diagnostics
                .iter()
                .all(|issue| issue.warning && issue.origins.len() == 2)
        );
    }
    let same = MappingSource::new("same.map", format!("package api; {}", declaration("Built")));
    let plan = parse_sources(&program, &[same.clone(), builtin.clone()]).unwrap();
    assert!(plan.diagnostics.is_empty());
    assert_eq!(plan.domains()[0].values.len(), 1);

    let authored_first = MappingSource {
        builtin: false,
        ..builtin
    };
    let error = parse_sources(&program, &[authored_first, same]).unwrap_err();
    let issues = &error.downcast_ref::<MappingDiagnostics>().unwrap().issues;
    assert_eq!(issues.len(), 3);
    for (issue, binding) in issues.iter().zip([
        "semantic target binding",
        "@DomainValue binding",
        "scoped call contract",
    ]) {
        assert_eq!(issue.message, format!("duplicate {binding}"));
        assert_eq!(issue.origins.len(), 2);
    }
}

#[test]
fn synthetic_and_real_domain_values_retain_their_own_origins_until_validation() {
    let mut owner = class("a");
    owner.constant_pool.entries = vec![Constant::Unusable, Constant::Integer(1)];
    let mut constant = field("x", "I");
    constant.access_flags = 0x19;
    constant.constant_value = Some(1);
    owner.fields.push(constant);
    let own = [owner];
    let program = Program::new(&own, &[]).unwrap();
    let error = parse_sources(
        &program,
        &[
            MappingSource::new(
                "domain.map",
                "@ValueDomain interface Kind {\n int ONE = 1;\n}",
            ),
            MappingSource::new(
                "field.map",
                "class A /* was a */ {\n @DomainValue(Kind.class) int value /* was x */;\n}",
            ),
        ],
    )
    .unwrap_err();
    let issues = &error.downcast_ref::<MappingDiagnostics>().unwrap().issues;
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("duplicate semantic value 1"));
    assert_eq!(
        issues[0]
            .origins
            .iter()
            .map(|origin| (origin.path.as_ref(), origin.line))
            .collect::<Vec<_>>(),
        [("domain.map", 2), ("field.map", 2)]
    );
}

#[test]
fn invalid_domains_stop_dependent_member_cascades() {
    let own = [class("a")];
    let program = Program::new(&own, &[]).unwrap();
    let error = parse_sources(&program, &[source("@ValueDomain interface Kind { String BAD = 1; } class A /* was a */ { @Domain(Kind.class) Missing value /* was absent */; }")]).unwrap_err();
    let issues = &error.downcast_ref::<MappingDiagnostics>().unwrap().issues;
    assert_eq!(issues.len(), 1);
    assert!(
        issues[0]
            .message
            .contains("semantic constants require an integral type")
    );
}
