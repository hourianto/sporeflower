//! Reviewed legacy bytecode keeps selected operands on the stack across case
//! joins. javac ordinarily emits separate calls, so author the shared consumers
//! explicitly and compare all case/guard/throw paths with rebuilt Java.
use crate::common::checked_stdout as checked;
use sporeflower::{classfile::ClassFile, ir, structure};
use std::{collections::BTreeMap, fs, process::Command};

#[derive(Default)]
struct Code {
    bytes: Vec<u8>,
    labels: BTreeMap<&'static str, usize>,
    branches: Vec<(usize, usize, &'static str, bool)>,
}

impl Code {
    fn label(&mut self, name: &'static str) {
        assert!(self.labels.insert(name, self.bytes.len()).is_none());
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
    }
    fn branch(&mut self, opcode: u8, target: &'static str) {
        let pc = self.bytes.len();
        self.emit(&[opcode, 0, 0]);
        self.branches.push((pc + 1, pc, target, false));
    }
    fn mark(&mut self, value: u8) {
        self.emit(&[0x10, value, 0xb8, 0, 13]); // bipush; SharedTailSupport.mark
    }
    fn switch(&mut self, targets: &[&'static str], default: &'static str) {
        let pc = self.bytes.len();
        self.emit(&[0xaa]);
        while !self.bytes.len().is_multiple_of(4) {
            self.emit(&[0]);
        }
        let position = self.bytes.len();
        self.emit(&[0; 4]);
        self.branches.push((position, pc, default, true));
        self.emit(&0i32.to_be_bytes());
        self.emit(&((targets.len() - 1) as i32).to_be_bytes());
        for &target in targets {
            let position = self.bytes.len();
            self.emit(&[0; 4]);
            self.branches.push((position, pc, target, true));
        }
    }
    fn finish(mut self) -> Vec<u8> {
        for (position, pc, target, wide) in self.branches {
            let offset = self.labels[target] as i32 - pc as i32;
            if wide {
                self.bytes[position..position + 4].copy_from_slice(&offset.to_be_bytes());
            } else {
                self.bytes[position..position + 2].copy_from_slice(&(offset as i16).to_be_bytes());
            }
        }
        self.bytes
    }
}

fn utf(bytes: &mut Vec<u8>, value: &str) {
    bytes.push(1);
    bytes.extend((value.len() as u16).to_be_bytes());
    bytes.extend(value.as_bytes());
}

fn shared_class() -> Vec<u8> {
    let mut code = Code::default();
    code.emit(&[0x02, 0x3d]); // result = -1
    code.label("try");
    code.mark(1);
    code.emit(&[0x57]);
    code.emit(&[0x1a]);
    code.switch(&["zero", "one", "two", "three", "four"], "cleanup");
    code.label("zero");
    code.emit(&[0x1b]);
    code.branch(0x9b, "cleanup"); // a guarded case skips both consumers
    code.mark(10);
    code.mark(20);
    code.branch(0xa7, "consume");
    code.label("one");
    code.mark(11);
    code.branch(0xa7, "right");
    code.label("two");
    code.mark(12);
    code.branch(0xa7, "right");
    code.label("three");
    code.mark(13);
    code.mark(23);
    code.branch(0xa7, "consume");
    code.label("four");
    code.mark(14);
    code.emit(&[0x57]);
    code.branch(0xa7, "cleanup");
    code.label("right");
    code.mark(21); // only cases one/two evaluate this operand
    code.label("consume");
    code.emit(&[0xb8, 0, 16, 0x3d]); // consume(left, right); result = value
    code.label("try_end");
    code.branch(0xa7, "cleanup");
    code.label("catch");
    code.emit(&[0x57]);
    code.mark(99);
    code.emit(&[0x57, 0x10, 0xfe, 0x3d]); // result = -2
    code.label("cleanup");
    code.mark(90);
    code.emit(&[0x57, 0x1c, 0xac]);
    let handlers = [(
        code.labels["try"],
        code.labels["try_end"],
        code.labels["catch"],
        18,
    )];
    let code = code.finish();

    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 49, 0, 20];
    utf(&mut bytes, "SharedTailCases"); // 1
    bytes.extend([7, 0, 1]); // 2 Class
    utf(&mut bytes, "java/lang/Object"); // 3
    bytes.extend([7, 0, 3]); // 4 Class
    utf(&mut bytes, "run"); // 5
    utf(&mut bytes, "(II)I"); // 6
    utf(&mut bytes, "Code"); // 7
    utf(&mut bytes, "protectedRun"); // 8
    utf(&mut bytes, "SharedTailSupport"); // 9
    bytes.extend([7, 0, 9]); // 10 Class
    utf(&mut bytes, "mark"); // 11
    // mark's descriptor is added last, after the exception class.
    bytes.extend([12, 0, 11, 0, 19]); // 12 NameAndType
    bytes.extend([10, 0, 10, 0, 12]); // 13 Methodref
    utf(&mut bytes, "consume"); // 14
    bytes.extend([12, 0, 14, 0, 6]); // 15 NameAndType
    bytes.extend([10, 0, 10, 0, 15]); // 16 Methodref
    utf(&mut bytes, "java/lang/RuntimeException"); // 17
    bytes.extend([7, 0, 17]); // 18 Class
    utf(&mut bytes, "(I)I"); // 19
    bytes.extend([0, 0x31, 0, 2, 0, 4, 0, 0, 0, 0, 0, 2]);
    for (name, handlers) in [(5, &[][..]), (8, &handlers[..])] {
        bytes.extend([0, 9, 0, name, 0, 6, 0, 1, 0, 7]);
        bytes.extend(((code.len() + 12 + handlers.len() * 8) as u32).to_be_bytes());
        bytes.extend([0, 3, 0, 3]);
        bytes.extend((code.len() as u32).to_be_bytes());
        bytes.extend(&code);
        bytes.extend((handlers.len() as u16).to_be_bytes());
        for &(start, end, target, kind) in handlers {
            for value in [start, end, target, kind] {
                bytes.extend((value as u16).to_be_bytes());
            }
        }
        bytes.extend([0, 0]);
    }
    bytes.extend([0, 0]);
    bytes
}

fn choice_class() -> Vec<u8> {
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 49, 0, 23];
    utf(&mut bytes, "ChoiceConsumers"); // 1
    bytes.extend([7, 0, 1]);
    utf(&mut bytes, "java/lang/Object"); // 3
    bytes.extend([7, 0, 3]);
    utf(&mut bytes, "store"); // 5
    utf(&mut bytes, "(Z[I[II)I"); // 6
    utf(&mut bytes, "Code"); // 7
    utf(&mut bytes, "protectedStore"); // 8
    utf(&mut bytes, "ChoiceConsumerSupport"); // 9
    bytes.extend([7, 0, 9]);
    utf(&mut bytes, "mark"); // 11
    bytes.extend([12, 0, 11, 0, 19, 10, 0, 10, 0, 12]);
    utf(&mut bytes, "consume"); // 14
    bytes.extend([12, 0, 14, 0, 20, 10, 0, 10, 0, 15]);
    utf(&mut bytes, "java/lang/RuntimeException"); // 17
    bytes.extend([7, 0, 17]);
    utf(&mut bytes, "(I)I"); // 19
    utf(&mut bytes, "(II)I"); // 20
    utf(&mut bytes, "choose"); // 21
    utf(&mut bytes, "protectedChoose"); // 22
    bytes.extend([0, 0x31, 0, 2, 0, 4, 0, 0, 0, 0, 0, 4]);
    for (name, store, protected) in [
        (5, true, false),
        (8, true, true),
        (21, false, false),
        (22, false, true),
    ] {
        let mut code = Code::default();
        code.emit(&[0x02, 0x36, 4]); // state = -1
        code.label("try");
        code.emit(&[0x1a]);
        code.branch(0x99, "right");
        for (side, array, marker) in [(0, 0x2b, 1), (1, 0x2c, 2)] {
            if side == 1 {
                code.label("right");
            }
            code.mark(marker);
            code.emit(&[0x36, 4]);
            if store {
                code.emit(&[array, 0x03, 0x15, 4, 0x4f]); // prior arm-owned store
                code.emit(&[array, 0x1d, 0x03, if side == 0 { 0x60 } else { 0x64 }]);
            } else {
                code.mark(marker * 10 + 1);
            }
            code.mark(marker * 10);
            if side == 0 {
                code.branch(0xa7, "consume");
            }
        }
        code.label("consume");
        if store {
            code.emit(&[0x4f]);
        } else {
            code.emit(&[0xb8, 0, 16]);
        }
        code.label("try_end");
        code.emit(&[0x15, 4]);
        if !store {
            code.emit(&[0x60]);
        }
        code.emit(&[0xac]);
        code.label("catch");
        code.emit(&[0x57, 0x15, 4, 0xac]);
        let handler = [
            code.labels["try"],
            code.labels["try_end"],
            code.labels["catch"],
            18,
        ];
        let code = code.finish();
        bytes.extend([0, 9, 0, name, 0, 6, 0, 1, 0, 7]);
        bytes.extend(((code.len() + 12 + if protected { 8 } else { 0 }) as u32).to_be_bytes());
        bytes.extend([0, 3, 0, 5]);
        bytes.extend((code.len() as u32).to_be_bytes());
        bytes.extend(code);
        bytes.extend([0, u8::from(protected)]);
        if protected {
            for value in handler {
                bytes.extend((value as u16).to_be_bytes());
            }
        }
        bytes.extend([0, 0]);
    }
    bytes.extend([0, 0]);
    bytes
}

#[test]
fn selected_consumers_own_complete_operations_and_catch_snapshots() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::write(original.join("ChoiceConsumers.class"), choice_class()).unwrap();
    crate::common::compile(
        &["tests/fixtures/java/ChoiceConsumerSupport.java".into()],
        &original,
        None,
    );
    let expected = crate::common::execute(&original, "ChoiceConsumerSupport");
    crate::common::decompile(&original, &source);
    let text = fs::read_to_string(source.join("ChoiceConsumers.java")).unwrap();
    assert!(
        !text.contains("int[] v"),
        "selected array must stay in its arm: {text}"
    );
    assert_eq!(
        text.matches("ChoiceConsumerSupport.consume(").count(),
        4,
        "{text}"
    );
    crate::common::compile(&crate::common::sources(&source), &rebuilt, None);
    assert_eq!(
        expected,
        crate::common::execute(&rebuilt, "ChoiceConsumerSupport")
    );
}

#[test]
fn shared_stack_consumers_preserve_operands_effects_and_handler_coverage() {
    let work = tempfile::tempdir().unwrap();
    let original = work.path().join("original");
    let source = work.path().join("source");
    let rebuilt = work.path().join("rebuilt");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&rebuilt).unwrap();
    let bytes = shared_class();
    let class = ClassFile::parse(&bytes).unwrap();
    for method in &class.methods {
        let ir = ir::decompile_method(&class, method).unwrap();
        let prepared = structure::PreparedIr::new(ir);
        let tree = structure::structure(&prepared).unwrap();
        assert_eq!(
            tree.statistics.labeled_regions, 0,
            "{}: {tree:#?}",
            method.name
        );
        assert_eq!(
            tree.statistics.dispatch_regions, 0,
            "{}: {tree:#?}",
            method.name
        );
    }
    fs::write(original.join("SharedTailCases.class"), bytes).unwrap();
    checked(
        crate::common::compile_command()
            .args(["-g:none", "-cp"])
            .arg(&original)
            .arg("-d")
            .arg(&original)
            .arg("tests/fixtures/java/SharedTailSupport.java"),
    );
    let expected = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&original)
            .arg("SharedTailSupport"),
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
    let sources = [
        source.join("SharedTailCases.java"),
        source.join("SharedTailSupport.java"),
    ];
    let rendered = fs::read_to_string(&sources[0]).unwrap();
    assert!(!rendered.contains("label"), "{rendered}");
    checked(
        crate::common::compile_command()
            .args(["-d"])
            .arg(&rebuilt)
            .args(&sources),
    );
    let actual = checked(
        Command::new(crate::common::java())
            .args(["-Xverify:all", "-cp"])
            .arg(&rebuilt)
            .arg("SharedTailSupport"),
    );
    assert_eq!(actual, expected);
    if let Some(legacy) = crate::common::legacy_javac() {
        let output = work.path().join("legacy");
        fs::create_dir_all(&output).unwrap();
        checked(
            crate::common::legacy_command(legacy, &api)
                .arg("-d")
                .arg(&output)
                .args(&sources),
        );
        let actual = checked(
            Command::new(crate::common::java())
                .args(["-Xverify:all", "-cp"])
                .arg(&output)
                .arg("SharedTailSupport"),
        );
        assert_eq!(actual, expected);
    }
}
