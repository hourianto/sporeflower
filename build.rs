#[path = "src/api/schema.rs"]
mod schema;
#[path = "src/api/writer.rs"]
mod writer;

use anyhow::{Context, Result};
use std::{fs, io::Write, path::PathBuf};

fn text(out: &mut Vec<u8>, text: &str) -> Result<()> {
    out.extend(u32::try_from(text.len())?.to_be_bytes());
    out.extend(text.as_bytes());
    Ok(())
}

fn generate() -> Result<()> {
    println!("cargo:rerun-if-changed=api");
    println!("cargo:rerun-if-changed=src/api/schema.rs");
    println!("cargo:rerun-if-changed=src/api/writer.rs");
    let mut paths = fs::read_dir("api")?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    paths.sort();
    let mut out = b"SFAPI\0\x01\0".to_vec();
    out.extend(u32::try_from(paths.len())?.to_be_bytes());
    for path in paths {
        let mut catalog: schema::Catalog = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| path.display().to_string())?;
        text(&mut out, path.file_stem().unwrap().to_str().unwrap())?;
        out.push(u8::from(catalog.fallback));
        catalog.classes.sort_by(|a, b| a.name.cmp(&b.name));
        out.extend(u32::try_from(catalog.classes.len())?.to_be_bytes());
        for class in catalog.classes {
            text(&mut out, &class.name)?;
            out.extend((class.access as u16).to_be_bytes());
            let bytes = writer::class_bytes(&class)
                .with_context(|| format!("{} in {}", class.name, path.display()))?;
            out.extend(u32::try_from(bytes.len())?.to_be_bytes());
            out.extend(bytes);
            out.extend(
                u32::try_from(class.interfaces.len() + usize::from(class.super_name.is_some()))?
                    .to_be_bytes(),
            );
            if let Some(parent) = &class.super_name {
                text(&mut out, parent)?;
            }
            for parent in &class.interfaces {
                text(&mut out, parent)?;
            }
            let member_count = class
                .fields
                .iter()
                .filter(|field| field.access & 2 == 0)
                .count()
                + class
                    .methods
                    .iter()
                    .filter(|method| method.access & 2 == 0)
                    .count();
            out.extend(u32::try_from(member_count)?.to_be_bytes());
            for field in &class.fields {
                if field.access & 2 != 0 {
                    continue;
                }
                text(&mut out, &field.name)?;
                text(&mut out, &field.descriptor)?;
                out.push(1 | if field.access & 8 != 0 { 2 } else { 0 });
            }
            for method in &class.methods {
                if method.access & 2 != 0 {
                    continue;
                }
                text(&mut out, &method.name)?;
                text(&mut out, &method.descriptor)?;
                out.push(if method.access & 8 != 0 { 2 } else { 0 });
            }
        }
    }
    let path =
        PathBuf::from(std::env::var_os("OUT_DIR").context("missing OUT_DIR")?).join("j2me-api.bin");
    fs::File::create(path)?.write_all(&out)?;
    Ok(())
}

fn main() {
    generate().expect("generate bundled J2ME API declarations");
}
