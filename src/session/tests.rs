use super::*;

const BYTES: &[u8] =
    include_bytes!("../../tests/fixtures/legacy/classes/pkg/TestConsecutiveReads.class");
const NAME: &str = "pkg/TestConsecutiveReads";

fn session() -> Session {
    let mut session = Session::default();
    assert_eq!(
        session.add_class("unrelated/path.class", BYTES).unwrap(),
        NAME
    );
    session
}

#[test]
fn prepared_session_reuses_original_identity_and_rejects_duplicate_input() {
    let mut session = session();
    assert!(session.decompile(NAME).is_err());
    let metadata = session.prepare(SessionOptions::default()).unwrap();
    assert_eq!(metadata.classes[0].name, NAME);
    assert!(metadata.api_classes > 0);
    let first = session.decompile(NAME).unwrap();
    assert_eq!(first.failed_methods, 0);
    assert!(session.add_class("duplicate.class", BYTES).is_err());
    assert_eq!(session.decompile(NAME).unwrap().source, first.source);
    session.prepare(SessionOptions::default()).unwrap();
    assert_eq!(session.decompile(NAME).unwrap().source, first.source);
}

#[test]
fn failed_mapping_resolution_keeps_bytecode_for_repair() {
    let mut session = session();
    let mut inputs = SessionMappings {
        sources: vec![MappingText {
            path: "user/game.map".into(),
            source: "class Broken /* was absent */ {}".into(),
        }],
        ..SessionMappings::default()
    };
    session.set_mappings(inputs.clone()).unwrap();
    let error = session.prepare(SessionOptions::default()).unwrap_err();
    assert!(format!("{error:#}").contains("user/game.map:1"));
    inputs.sources[0].source = "package pkg; class Reads /* was pkg/TestConsecutiveReads */ { int readCount /* was calls */; }".into();
    session.set_mappings(inputs).unwrap();
    session.prepare(SessionOptions::default()).unwrap();
    let output = session.decompile(NAME).unwrap();
    assert!(output.source.contains("class Reads"), "{}", output.source);
    assert!(output.source.contains("readCount"));
    assert!(session.decompile("pkg/Reads").is_err());
    let tiny = session.mapping_tiny().unwrap();
    assert!(tiny.contains("c\tpkg/TestConsecutiveReads\tpkg/Reads\n"));
    assert!(tiny.contains("\tf\tI\tcalls\treadCount\n"));

    session.set_mappings(SessionMappings::default()).unwrap();
    assert!(session.mapping_tiny().is_err());
    session.prepare(SessionOptions::default()).unwrap();
    assert!(
        session
            .decompile(NAME)
            .unwrap()
            .source
            .contains("class TestConsecutiveReads")
    );
}

#[test]
fn portable_renames_share_native_conflict_validation() {
    let mut session = session();
    session
        .set_mappings(SessionMappings {
            sources: vec![MappingText {
                path: "authored.map".into(),
                source: "package pkg; class TestConsecutiveReads { int counter /* was calls */; }"
                    .into(),
            }],
            fields: vec![FieldRename {
                owner: NAME.into(),
                name: "calls".into(),
                descriptor: None,
                dst: "conflict".into(),
            }],
            ..SessionMappings::default()
        })
        .unwrap();
    let error = session.prepare(SessionOptions::default()).unwrap_err();
    assert!(format!("{error:#}").contains("conflicting authored names"));
}

#[test]
fn portable_parameter_names_use_jvm_slots_for_wide_values() {
    let mut session = session();
    session.classes[0].methods.push(crate::classfile::Method {
        access_flags: 0x0101,
        name: "wide".into(),
        descriptor: "(JDI)V".into(),
        code: None,
        exceptions: vec![],
        synthetic: false,
    });
    let mut inputs = SessionMappings {
        methods: vec![MethodRename {
            owner: NAME.into(),
            name: "wide".into(),
            descriptor: "(JDI)V".into(),
            dst: None,
            parameters: vec![ParameterRename {
                index: 2,
                name: "wrongHalf".into(),
            }],
        }],
        ..SessionMappings::default()
    };
    session.set_mappings(inputs.clone()).unwrap();
    let error = session.prepare(SessionOptions::default()).unwrap_err();
    assert!(format!("{error:#}").contains("slot 2 does not name a parameter"));
    inputs.methods[0].parameters = vec![
        ParameterRename {
            index: 1,
            name: "elapsed".into(),
        },
        ParameterRename {
            index: 3,
            name: "scale".into(),
        },
        ParameterRename {
            index: 5,
            name: "count".into(),
        },
    ];
    session.set_mappings(inputs).unwrap();
    session.prepare(SessionOptions::default()).unwrap();
    let tiny = session.mapping_tiny().unwrap();
    assert!(tiny.contains("\t\tp\t1\t\telapsed\n"));
    assert!(tiny.contains("\t\tp\t3\t\tscale\n"));
    assert!(tiny.contains("\t\tp\t5\t\tcount\n"));
}
