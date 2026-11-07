//! Class-file and descriptor model. Executable data is strict; optional metadata is advisory.
use crate::Result;
use std::collections::HashMap;

pub(crate) const PUBLIC: u16 = 0x0001;
pub(crate) const PRIVATE: u16 = 0x0002;
pub(crate) const PROTECTED: u16 = 0x0004;
pub(crate) const STATIC: u16 = 0x0008;
pub(crate) const NATIVE: u16 = 0x0100;
pub(crate) const INTERFACE: u16 = 0x0200;

pub(crate) fn package(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(package, _)| package)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JvmType {
    Void,
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Object(String),
    Array(Box<JvmType>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodDescriptor {
    pub parameters: Vec<JvmType>,
    pub return_type: JvmType,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassFile {
    pub minor_version: u16,
    pub major_version: u16,
    pub access_flags: u16,
    pub name: String,
    pub super_name: Option<String>,
    pub interfaces: Vec<String>,
    pub constant_pool: ConstantPool,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
    pub source_file: Option<String>,
    pub inner_classes: Vec<InnerClass>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub access_flags: u16,
    pub name: String,
    pub descriptor: String,
    pub constant_value: Option<u16>,
    pub synthetic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Method {
    pub access_flags: u16,
    pub name: String,
    pub descriptor: String,
    pub code: Option<Code>,
    pub exceptions: Vec<String>,
    pub synthetic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub max_stack: u16,
    pub max_locals: u16,
    pub bytes: Vec<u8>,
    pub exception_table: Vec<ExceptionHandler>,
    pub line_numbers: Vec<LineNumber>,
    pub local_variables: Vec<LocalVariable>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionHandler {
    pub start_pc: u32,
    pub end_pc: u32,
    pub handler_pc: u32,
    pub catch_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineNumber {
    pub start_pc: u16,
    pub line: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalVariable {
    pub start_pc: u16,
    pub length: u16,
    pub name: String,
    pub descriptor: String,
    pub index: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InnerClass {
    pub inner: String,
    pub outer: Option<String>,
    pub inner_name: Option<String>,
    pub access_flags: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Unusable,
    Utf8(String),
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Class(u16),
    String(u16),
    Fieldref {
        class_index: u16,
        name_and_type_index: u16,
    },
    Methodref {
        class_index: u16,
        name_and_type_index: u16,
    },
    InterfaceMethodref {
        class_index: u16,
        name_and_type_index: u16,
    },
    NameAndType {
        name_index: u16,
        descriptor_index: u16,
    },
    MethodHandle {
        reference_kind: u8,
        reference_index: u16,
    },
    MethodType(u16),
    Dynamic {
        bootstrap_index: u16,
        name_and_type_index: u16,
    },
    InvokeDynamic {
        bootstrap_index: u16,
        name_and_type_index: u16,
    },
    Module(u16),
    Package(u16),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConstantPool {
    pub entries: Vec<Constant>,
    raw_utf16: HashMap<u16, Vec<u16>>,
}

#[derive(Clone, Copy, Debug)]
pub struct MemberRef<'a> {
    pub owner: &'a str,
    pub name: &'a str,
    pub descriptor: &'a str,
    pub interface: bool,
}

mod descriptor;
mod parse;
pub use parse::parse;

impl ClassFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        parse(bytes)
    }
}
