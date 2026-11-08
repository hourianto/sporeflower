//! Small declaration-only classfile writer used for catalogs and inferred exports.
//!
//! The output contains no SDK implementation. Concrete methods throw immediately,
//! including constructors, which need not initialize `this` before throwing.
use super::schema;
use crate::classfile::{MethodDescriptor, binary::Pool};
use anyhow::{Result, bail};

impl Pool {
    fn constant(&mut self, descriptor: &str, text: &str) -> Result<u16> {
        let (tag, bytes) = match descriptor {
            "J" => (5, text.parse::<i64>()?.to_be_bytes().to_vec()),
            "F" => (4, u32::from_str_radix(text, 16)?.to_be_bytes().to_vec()),
            "D" => (6, u64::from_str_radix(text, 16)?.to_be_bytes().to_vec()),
            "Ljava/lang/String;" => {
                let index = self.text(text)?;
                (8, index.to_be_bytes().to_vec())
            }
            "Z" | "B" | "C" | "S" | "I" => (3, text.parse::<i32>()?.to_be_bytes().to_vec()),
            _ => bail!("unsupported constant descriptor {descriptor}"),
        };
        self.insert(tag, bytes)
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
    u16be(&mut out, pool.text(name)?);
    u32be(&mut out, body.len().try_into()?);
    out.extend(body);
    Ok(out)
}

fn signature(pool: &mut Pool, value: &Option<String>, attrs: &mut Vec<Vec<u8>>) -> Result<()> {
    if let Some(value) = value {
        let index = pool.text(value)?;
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
        u16be(&mut body, pool.text(&field.name)?);
        u16be(&mut body, pool.text(&field.descriptor)?);
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
        u16be(&mut body, pool.text(&method.name)?);
        u16be(&mut body, pool.text(&method.descriptor)?);
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
                MethodDescriptor::parse(&method.descriptor)?.parameter_slots()
                    + u16::from(method.access & 8 == 0),
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
                    Some(name) => pool.text(name)?,
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
            (Some(name), Some(descriptor)) => pool.pair(name, descriptor)?,
            _ => 0,
        };
        u16be(&mut enclosing, method);
        attrs.push(attribute(&mut pool, "EnclosingMethod", enclosing)?);
    }
    attributes(&mut body, attrs)?;
    let mut result = Vec::new();
    u32be(&mut result, 0xcafebabe);
    u16be(&mut result, (class.version >> 16) as u16);
    u16be(&mut result, class.version as u16);
    pool.write(&mut result);
    result.extend(body);
    Ok(result)
}
