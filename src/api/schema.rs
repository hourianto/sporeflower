//! The checked-in declaration format; shared by the build script and its tests.
use serde::Deserialize;

#[derive(Debug, Deserialize)]
// The catalog envelope is read by build.rs; runtime inference only writes Class.
#[allow(dead_code)]
pub struct Catalog {
    #[serde(default)]
    pub fallback: bool,
    pub classes: Vec<Class>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Class {
    pub name: String,
    pub version: u32,
    pub access: u32,
    pub super_name: Option<String>,
    #[serde(default)]
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub inner_classes: Vec<(String, Option<String>, Option<String>, u32)>,
    pub signature: Option<String>,
    pub enclosing: Option<(String, Option<String>, Option<String>)>,
    #[serde(default)]
    pub fields: Vec<Field>,
    #[serde(default)]
    pub methods: Vec<Method>,
}

#[derive(Debug, Deserialize)]
pub struct Field {
    pub name: String,
    pub descriptor: String,
    pub access: u32,
    pub signature: Option<String>,
    pub constant: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Method {
    pub name: String,
    pub descriptor: String,
    pub access: u32,
    pub signature: Option<String>,
    #[serde(default)]
    pub exceptions: Vec<String>,
}
