//! Small declaration-only classfile writer used for catalogs and inferred exports.
//!
//! The output contains no SDK implementation. Concrete methods throw immediately,
//! including constructors, which need not initialize `this` before throwing.
use super::schema;
use anyhow::{Result, bail, ensure};
use std::collections::HashMap;

#[derive(Default)]
struct Pool {
    entries: Vec<Vec<u8>>,
    indices: HashMap<Vec<u8>, u16>,
}

impl Pool {
    fn insert(&mut self, bytes: Vec<u8>, wide: bool) -> Result<u16> {
        if let Some(&index) = self.indices.get(&bytes) {
            return Ok(index);
        }
        ensure!(
            self.entries.len() + usize::from(wide) < 65_534,
            "stub constant pool exceeds u16"
        );
        let index = self.entries.len() as u16 + 1;
        self.indices.insert(bytes.clone(), index);
        self.entries.push(bytes);
        if wide {
            self.entries.push(Vec::new());
        }
        Ok(index)
    }

    fn utf8(&mut self, text: &str) -> Result<u16> {
        let mut encoded = Vec::with_capacity(text.len());
        for unit in text.encode_utf16() {
            match unit {
                1..=0x7f => encoded.push(unit as u8),
                0..=0x7ff => {
                    encoded.push(0xc0 | (unit >> 6) as u8);
                    encoded.push(0x80 | (unit & 63) as u8);
                }
                _ => {
                    encoded.push(0xe0 | (unit >> 12) as u8);
                    encoded.push(0x80 | ((unit >> 6) & 63) as u8);
                    encoded.push(0x80 | (unit & 63) as u8);
                }
            }
        }
        let length = u16::try_from(encoded.len())
            .map_err(|_| anyhow::anyhow!("stub UTF-8 constant too long"))?;
        let mut result = vec![1];
        u16be(&mut result, length);
        result.extend(encoded);
        self.insert(result, false)
    }

    fn class(&mut self, name: &str) -> Result<u16> {
        let name = self.utf8(name)?;
        self.insert([&[7][..], &name.to_be_bytes()].concat(), false)
    }

    fn name_type(&mut self, name: &str, descriptor: &str) -> Result<u16> {
        let name = self.utf8(name)?;
        let descriptor = self.utf8(descriptor)?;
        self.insert(
            [&[12][..], &name.to_be_bytes(), &descriptor.to_be_bytes()].concat(),
            false,
        )
    }

    fn constant(&mut self, descriptor: &str, text: &str) -> Result<u16> {
        let (bytes, wide) = match descriptor {
            "J" => (
                [&[5][..], &text.parse::<i64>()?.to_be_bytes()].concat(),
                true,
            ),
            "F" => (
                [&[4][..], &u32::from_str_radix(text, 16)?.to_be_bytes()].concat(),
                false,
            ),
            "D" => (
                [&[6][..], &u64::from_str_radix(text, 16)?.to_be_bytes()].concat(),
                true,
            ),
            "Ljava/lang/String;" => {
                let index = self.utf8(text)?;
                ([&[8][..], &index.to_be_bytes()].concat(), false)
            }
            "Z" | "B" | "C" | "S" | "I" => (
                [&[3][..], &text.parse::<i32>()?.to_be_bytes()].concat(),
                false,
            ),
            _ => bail!("unsupported constant descriptor {descriptor}"),
        };
        self.insert(bytes, wide)
    }
}

fn u16be(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_be_bytes());
}
fn u32be(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}

fn attribute(pool: &mut Pool, name: &str, body: Vec<u8>) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len() + 6);
    u16be(&mut out, pool.utf8(name)?);
    u32be(&mut out, body.len().try_into()?);
    out.extend(body);
    Ok(out)
}

fn signature(pool: &mut Pool, value: &Option<String>, attrs: &mut Vec<Vec<u8>>) -> Result<()> {
    if let Some(value) = value {
        let index = pool.utf8(value)?;
        attrs.push(attribute(pool, "Signature", index.to_be_bytes().to_vec())?);
    }
    Ok(())
}

fn deprecated(pool: &mut Pool, access: u32, attrs: &mut Vec<Vec<u8>>) -> Result<()> {
    if access & 0x20000 != 0 {
        attrs.push(attribute(pool, "Deprecated", Vec::new())?);
    }
    Ok(())
}

fn attributes(out: &mut Vec<u8>, attrs: Vec<Vec<u8>>) -> Result<()> {
    u16be(out, attrs.len().try_into()?);
    for attr in attrs {
        out.extend(attr);
    }
    Ok(())
}

fn parameter_slots(descriptor: &str) -> Result<u16> {
    let bytes = descriptor.as_bytes();
    ensure!(bytes.first() == Some(&b'('), "invalid method descriptor");
    let mut index = 1;
    let mut slots = 0u16;
    while bytes.get(index) != Some(&b')') {
        let mut array = false;
        while bytes.get(index) == Some(&b'[') {
            array = true;
            index += 1;
        }
        let Some(&tag) = bytes.get(index) else {
            bail!("truncated method descriptor")
        };
        index += 1;
        slots = slots
            .checked_add(if !array && matches!(tag, b'J' | b'D') {
                2
            } else {
                1
            })
            .ok_or_else(|| anyhow::anyhow!("too many parameter slots"))?;
        match tag {
            b'L' => {
                let Some(end) = bytes[index..].iter().position(|b| *b == b';') else {
                    bail!("unterminated method descriptor")
                };
                index += end + 1;
            }
            b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' => {}
            _ => bail!("invalid parameter descriptor"),
        }
    }
    Ok(slots)
}

pub fn class_bytes(class: &schema::Class) -> Result<Vec<u8>> {
    let mut pool = Pool::default();
    let mut body = Vec::new();
    u16be(&mut body, class.access as u16);
    u16be(&mut body, pool.class(&class.name)?);
    u16be(
        &mut body,
        match &class.super_name {
            Some(name) => pool.class(name)?,
            None => 0,
        },
    );
    u16be(&mut body, class.interfaces.len().try_into()?);
    for interface in &class.interfaces {
        u16be(&mut body, pool.class(interface)?);
    }
    u16be(&mut body, class.fields.len().try_into()?);
    for field in &class.fields {
        u16be(&mut body, field.access as u16);
        u16be(&mut body, pool.utf8(&field.name)?);
        u16be(&mut body, pool.utf8(&field.descriptor)?);
        let mut attrs = Vec::new();
        deprecated(&mut pool, field.access, &mut attrs)?;
        signature(&mut pool, &field.signature, &mut attrs)?;
        if let Some(value) = &field.constant {
            let index = pool.constant(&field.descriptor, value)?;
            attrs.push(attribute(
                &mut pool,
                "ConstantValue",
                index.to_be_bytes().to_vec(),
            )?);
        }
        attributes(&mut body, attrs)?;
    }
    u16be(&mut body, class.methods.len().try_into()?);
    for method in &class.methods {
        u16be(&mut body, method.access as u16);
        u16be(&mut body, pool.utf8(&method.name)?);
        u16be(&mut body, pool.utf8(&method.descriptor)?);
        let mut attrs = Vec::new();
        deprecated(&mut pool, method.access, &mut attrs)?;
        signature(&mut pool, &method.signature, &mut attrs)?;
        if !method.exceptions.is_empty() {
            let mut exceptions = Vec::new();
            u16be(&mut exceptions, method.exceptions.len().try_into()?);
            for exception in &method.exceptions {
                u16be(&mut exceptions, pool.class(exception)?);
            }
            attrs.push(attribute(&mut pool, "Exceptions", exceptions)?);
        }
        if method.access & (0x0400 | 0x0100) == 0 {
            let mut code = Vec::new();
            u16be(&mut code, 1);
            u16be(
                &mut code,
                parameter_slots(&method.descriptor)? + u16::from(method.access & 8 == 0),
            );
            u32be(&mut code, 2);
            code.extend([0x01, 0xbf]); // aconst_null; athrow
            u16be(&mut code, 0);
            u16be(&mut code, 0);
            attrs.push(attribute(&mut pool, "Code", code)?);
        }
        attributes(&mut body, attrs)?;
    }
    let mut attrs = Vec::new();
    deprecated(&mut pool, class.access, &mut attrs)?;
    signature(&mut pool, &class.signature, &mut attrs)?;
    if !class.inner_classes.is_empty() {
        let mut inner = Vec::new();
        u16be(&mut inner, class.inner_classes.len().try_into()?);
        for (name, outer, simple, access) in &class.inner_classes {
            u16be(&mut inner, pool.class(name)?);
            u16be(
                &mut inner,
                match outer {
                    Some(name) => pool.class(name)?,
                    None => 0,
                },
            );
            u16be(
                &mut inner,
                match simple {
                    Some(name) => pool.utf8(name)?,
                    None => 0,
                },
            );
            u16be(&mut inner, *access as u16);
        }
        attrs.push(attribute(&mut pool, "InnerClasses", inner)?);
    }
    if let Some((owner, name, descriptor)) = &class.enclosing {
        let mut enclosing = Vec::new();
        u16be(&mut enclosing, pool.class(owner)?);
        let method = match (name, descriptor) {
            (Some(name), Some(descriptor)) => pool.name_type(name, descriptor)?,
            _ => 0,
        };
        u16be(&mut enclosing, method);
        attrs.push(attribute(&mut pool, "EnclosingMethod", enclosing)?);
    }
    attributes(&mut body, attrs)?;
    let mut result =
        Vec::with_capacity(body.len() + pool.entries.iter().map(Vec::len).sum::<usize>() + 10);
    u32be(&mut result, 0xcafebabe);
    u16be(&mut result, (class.version >> 16) as u16);
    u16be(&mut result, class.version as u16);
    u16be(&mut result, (pool.entries.len() + 1).try_into()?);
    for entry in pool.entries {
        result.extend(entry);
    }
    result.extend(body);
    Ok(result)
}
