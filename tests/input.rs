use sporeflower::input::{parse_manifest, source_path};
use std::path::Path;

#[test]
fn unfolds_manifest_main_section_only() {
    let attrs = parse_manifest(
        "Manifest-Version: 1.0\r\nMIDlet-1: Game, icon.png,\r\n pkg.Main\r\n\r\nName: other\r\n",
    );
    assert_eq!(attrs["midlet-1"], "Game, icon.png,pkg.Main");
    assert!(!attrs.contains_key("name"));
}

#[test]
fn directory_metadata_is_shared_by_source_and_remap_loading() {
    use sporeflower::input::Input;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("META-INF")).unwrap();
    std::fs::write(
        dir.path().join("META-INF/MANIFEST.MF"),
        b"Manifest-Version: 1.0\r\nMicroEdition-Configuration: CLDC-1.0\r\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("arbitrary.class"),
        include_bytes!("fixtures/legacy/classes/pkg/TestConsecutiveReads.class"),
    )
    .unwrap();
    let source = Input::read(dir.path()).unwrap();
    let native = Input::read_for_remap(dir.path()).unwrap();
    assert_eq!(source.manifest["microedition-configuration"], "CLDC-1.0");
    assert_eq!(source.manifest, native.manifest);
    assert_eq!(source.classes(), native.classes());
    assert_eq!(source.warnings, native.warnings);
}

// APFS rejects non-UTF-8 filenames before the reader can see them.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn source_loading_uses_binary_identity_with_non_utf8_filenames() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(OsStr::from_bytes(b"\xff.class")),
        include_bytes!("fixtures/legacy/classes/pkg/TestConsecutiveReads.class"),
    )
    .unwrap();
    let input = sporeflower::input::Input::read(dir.path()).unwrap();
    assert_eq!(input.classes()[0].name, "pkg/TestConsecutiveReads");
}

#[test]
fn source_paths_cannot_escape_destination() {
    for name in [
        "../Escape",
        "/Absolute",
        "a//B",
        "a/../../B",
        "a\\B",
        "C:/B",
    ] {
        assert!(source_path(Path::new("out"), name).is_err(), "{name}");
    }
    assert_eq!(
        source_path(Path::new("out"), "pkg/Foo$Inner").unwrap(),
        Path::new("out/pkg/Foo$Inner.java")
    );
}

#[test]
fn class_named_resources_are_not_parsed_as_bytecode() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resources.jar");
    let mut jar = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    jar.start_file("main.class", options).unwrap();
    jar.write_all(b"\x89PNG\r\n\x1a\nicon").unwrap();
    jar.start_file("pkg/TestConsecutiveReads.class", options)
        .unwrap();
    jar.write_all(include_bytes!(
        "fixtures/legacy/classes/pkg/TestConsecutiveReads.class"
    ))
    .unwrap();
    jar.finish().unwrap();
    let input = sporeflower::input::Input::read(&path).unwrap();
    assert_eq!(input.classes().len(), 1);
    assert_eq!(input.classes()[0].name, "pkg/TestConsecutiveReads");
    assert_eq!(input.warnings.len(), 1);
}

#[test]
fn manifest_unfolds_utf8_bytes_and_tolerates_invalid_descriptions() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    for (description, expected, warnings) in [
        (&b"\xe2\r\n \x82\xac"[..], "\u{20ac}", 0),
        (
            &b"\x93Old handset text\x94"[..],
            "\u{fffd}Old handset text\u{fffd}",
            1,
        ),
    ] {
        let path = dir.path().join("metadata.jar");
        let mut jar = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        jar.start_file("pkg/TestConsecutiveReads.class", options)
            .unwrap();
        jar.write_all(include_bytes!(
            "fixtures/legacy/classes/pkg/TestConsecutiveReads.class"
        ))
        .unwrap();
        jar.start_file("META-INF/MANIFEST.MF", options).unwrap();
        jar.write_all(b"Manifest-Version: 1.0\r\nMIDlet-Description: ")
            .unwrap();
        jar.write_all(description).unwrap();
        jar.write_all(
            b"\r\nMIDlet-1: Title, icon.png, pkg.TestConsecutiveReads\r\n\r\nName: unrelated\r\n",
        )
        .unwrap();
        jar.finish().unwrap();
        let input = sporeflower::input::Input::read(&path).unwrap();
        assert_eq!(input.classes().len(), 1);
        assert_eq!(input.manifest["midlet-description"], expected);
        assert_eq!(
            input.manifest["midlet-1"],
            "Title, icon.png, pkg.TestConsecutiveReads"
        );
        assert!(!input.manifest.contains_key("name"));
        assert_eq!(input.warnings.len(), warnings);
    }
}

#[test]
fn native_input_rejects_unsafe_jar_entries_before_remapping() {
    use std::io::Write;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("unsafe.jar");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
    archive
        .start_file("../escape", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"resource").unwrap();
    archive.finish().unwrap();
    let error = sporeflower::input::Input::read_for_remap(&source)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("unsafe JAR entry"));
}

#[test]
fn application_scope_comes_from_supplied_manifest_entries() {
    use sporeflower::{
        input::Input,
        program::{AnalysisScope, Program},
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("META-INF")).unwrap();
    std::fs::write(
        dir.path().join("arbitrary.class"),
        include_bytes!("fixtures/legacy/classes/pkg/TestConsecutiveReads.class"),
    )
    .unwrap();
    for (manifest, expected) in [
        (
            "MicroEdition-Configuration: CLDC-1.0\n",
            AnalysisScope::Partial,
        ),
        (
            "MIDlet-1: Example, , pkg.TestConsecutiveReads\n",
            AnalysisScope::Application,
        ),
        (
            "Main-Class: pkg.TestConsecutiveReads\n",
            AnalysisScope::Application,
        ),
        (
            "MIDlet-1: Example, , missing.Entry\n",
            AnalysisScope::Partial,
        ),
        (
            "MIDlet-1: Example, , pkg.TestConsecutiveReads\nMIDlet-2: malformed\n",
            AnalysisScope::Partial,
        ),
    ] {
        std::fs::write(dir.path().join("META-INF/MANIFEST.MF"), manifest).unwrap();
        let input = Input::read(dir.path()).unwrap();
        assert_eq!(input.analysis_scope(), expected);
        Program::from_input(&input, &[]).unwrap();
    }
}
