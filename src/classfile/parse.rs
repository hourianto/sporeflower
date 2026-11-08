use super::{binary::Reader, *};
use anyhow::{Context, bail, ensure};

pub fn parse(bytes: &[u8]) -> Result<ClassFile> {
    let mut input = Reader::new(bytes);
    ensure!(input.u4()? == 0xcafe_babe, "invalid class-file magic");
    let minor_version = input.u2()?;
    let major_version = input.u2()?;
    let constant_pool = parse_pool(&mut input)?;
    let access_flags = input.u2()?;
    let name = constant_pool.class_name(input.u2()?)?.to_owned();
    let super_index = input.u2()?;
    let super_name = if super_index == 0 {
        None
    } else {
        Some(constant_pool.class_name(super_index)?.to_owned())
    };
    let mut interfaces = Vec::new();
    for _ in 0..input.u2()? {
        interfaces.push(constant_pool.class_name(input.u2()?)?.to_owned());
    }
    let mut warnings = Vec::new();
    let mut fields = Vec::new();
    for _ in 0..input.u2()? {
        fields.push(
            parse_field(&mut input, &constant_pool, &mut warnings)
                .with_context(|| format!("field in {name}"))?,
        );
    }
    let mut methods = Vec::new();
    for _ in 0..input.u2()? {
        methods.push(
            parse_method(&mut input, &constant_pool, &mut warnings)
                .with_context(|| format!("method in {name}"))?,
        );
    }
    let mut source_file = None;
    let mut inner_classes = Vec::new();
    for _ in 0..input.u2()? {
        let (attr_name, mut attribute) = attribute(&mut input, &constant_pool, &mut warnings)?;
        let result = match attr_name {
            "SourceFile" => (|| -> Result<()> {
                let value = constant_pool.utf8(attribute.u2()?)?.to_owned();
                attribute.end()?;
                source_file = Some(value);
                Ok(())
            })(),
            "InnerClasses" => (|| -> Result<()> {
                let mut entries = Vec::new();
                for _ in 0..attribute.u2()? {
                    let inner = attribute.u2()?;
                    let outer = attribute.u2()?;
                    let inner_name = attribute.u2()?;
                    let flags = attribute.u2()?;
                    if inner == 0 {
                        continue;
                    }
                    let entry = InnerClass {
                        inner: constant_pool.class_name(inner)?.to_owned(),
                        outer: if outer == 0 {
                            None
                        } else {
                            Some(constant_pool.class_name(outer)?.to_owned())
                        },
                        inner_name: if inner_name == 0 {
                            None
                        } else {
                            Some(constant_pool.utf8(inner_name)?.to_owned())
                        },
                        access_flags: flags,
                    };
                    entries.push(entry);
                }
                attribute.end()?;
                inner_classes.extend(entries);
                Ok(())
            })(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            warnings.push(format!(
                "ignored malformed {attr_name} attribute: {error:#}"
            ));
        }
    }
    // Trailing padding occurs in obfuscated archives; it has no executable meaning.
    if input.remaining() != 0 {
        warnings.push(format!("{} trailing class-file bytes", input.remaining()));
    }
    Ok(ClassFile {
        minor_version,
        major_version,
        access_flags,
        name,
        super_name,
        interfaces,
        constant_pool,
        fields,
        methods,
        source_file,
        inner_classes,
        warnings,
    })
}

fn attribute<'a, 'b>(
    input: &mut Reader<'a>,
    pool: &'b ConstantPool,
    warnings: &mut Vec<String>,
) -> Result<(&'b str, Reader<'a>)> {
    let name_index = input.u2()?;
    let length = input.u4()? as usize;
    let attribute = input.sub(length)?;
    let name = match pool.utf8(name_index) {
        Ok(name) => name,
        Err(error) => {
            warnings.push(format!("ignored unnamed attribute: {error:#}"));
            ""
        }
    };
    Ok((name, attribute))
}

fn parse_field(
    input: &mut Reader<'_>,
    pool: &ConstantPool,
    warnings: &mut Vec<String>,
) -> Result<Field> {
    let access_flags = input.u2()?;
    let name = pool.utf8(input.u2()?)?.to_owned();
    let descriptor = pool.utf8(input.u2()?)?.to_owned();
    let mut constant_value = None;
    let mut synthetic = access_flags & 0x1000 != 0;
    for _ in 0..input.u2()? {
        let (attr_name, mut attribute) = attribute(input, pool, warnings)?;
        let result = match attr_name {
            "ConstantValue" => (|| -> Result<()> {
                let index = attribute.u2()?;
                pool.get(index)?;
                attribute.end()?;
                constant_value = Some(index);
                Ok(())
            })(),
            "Synthetic" => {
                synthetic = true;
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            warnings.push(format!(
                "field {name}: ignored malformed {attr_name}: {error:#}"
            ));
        }
    }
    Ok(Field {
        access_flags,
        name,
        descriptor,
        constant_value,
        synthetic,
    })
}

fn parse_method(
    input: &mut Reader<'_>,
    pool: &ConstantPool,
    warnings: &mut Vec<String>,
) -> Result<Method> {
    let access_flags = input.u2()?;
    let name = pool.utf8(input.u2()?)?.to_owned();
    let descriptor = pool.utf8(input.u2()?)?.to_owned();
    let mut code = None;
    let mut exceptions = Vec::new();
    let mut synthetic = access_flags & 0x1000 != 0;
    for _ in 0..input.u2()? {
        let (attr_name, mut attribute) = attribute(input, pool, warnings)?;
        match attr_name {
            "Code" => {
                ensure!(
                    code.is_none(),
                    "duplicate Code attribute for {name}{descriptor}"
                );
                code = Some(
                    parse_code(&mut attribute, pool, warnings)
                        .with_context(|| format!("Code of {name}{descriptor}"))?,
                );
            }
            "Exceptions" => {
                // Unlike exception-table entries in Code, throws declarations are
                // metadata. Invalid links must not discard a perfectly usable body.
                let result = (|| -> Result<()> {
                    let mut entries = Vec::new();
                    for _ in 0..attribute.u2()? {
                        let index = attribute.u2()?;
                        match pool.class_name(index) {
                            Ok(name) => entries.push(name.to_owned()),
                            Err(error) => warnings.push(format!(
                                "method {name}: ignored throws entry #{index}: {error:#}"
                            )),
                        }
                    }
                    attribute.end()?;
                    exceptions.extend(entries);
                    Ok(())
                })();
                if let Err(error) = result {
                    warnings.push(format!(
                        "method {name}: ignored malformed Exceptions: {error:#}"
                    ));
                }
            }
            "Synthetic" => synthetic = true,
            _ => (),
        }
    }
    Ok(Method {
        access_flags,
        name,
        descriptor,
        code,
        exceptions,
        synthetic,
    })
}

fn parse_code(
    input: &mut Reader<'_>,
    pool: &ConstantPool,
    warnings: &mut Vec<String>,
) -> Result<Code> {
    let max_stack = input.u2()?;
    let max_locals = input.u2()?;
    let length = input.u4()? as usize;
    ensure!(
        length > 0 && length <= 65535,
        "invalid Code length {length}"
    );
    let bytes = input.take(length)?.to_vec();
    let mut exception_table = Vec::new();
    for _ in 0..input.u2()? {
        let start_pc = input.u2()? as u32;
        let end_pc = input.u2()? as u32;
        let handler_pc = input.u2()? as u32;
        let catch_type = input.u2()?;
        ensure!(
            start_pc < end_pc && end_pc <= length as u32 && handler_pc < length as u32,
            "invalid exception range {start_pc}..{end_pc} -> {handler_pc}"
        );
        exception_table.push(ExceptionHandler {
            start_pc,
            end_pc,
            handler_pc,
            catch_type: if catch_type == 0 {
                None
            } else {
                Some(pool.class_name(catch_type)?.to_owned())
            },
        });
    }
    let mut line_numbers = Vec::new();
    let mut local_variables = Vec::new();
    for _ in 0..input.u2()? {
        let (attr_name, mut attribute) = attribute(input, pool, warnings)?;
        let result = match attr_name {
            "LineNumberTable" => (|| -> Result<()> {
                let mut entries = Vec::new();
                for _ in 0..attribute.u2()? {
                    let start_pc = attribute.u2()?;
                    let line = attribute.u2()?;
                    if (start_pc as usize) < length {
                        entries.push(LineNumber { start_pc, line });
                    }
                }
                attribute.end()?;
                line_numbers.extend(entries);
                Ok(())
            })(),
            "LocalVariableTable" => (|| -> Result<()> {
                let mut entries = Vec::new();
                for _ in 0..attribute.u2()? {
                    let start_pc = attribute.u2()?;
                    let range_length = attribute.u2()?;
                    let name = attribute.u2()?;
                    let descriptor = attribute.u2()?;
                    let index = attribute.u2()?;
                    if (start_pc as usize + range_length as usize) <= length && index < max_locals {
                        entries.push(LocalVariable {
                            start_pc,
                            length: range_length,
                            name: pool.utf8(name)?.to_owned(),
                            descriptor: pool.utf8(descriptor)?.to_owned(),
                            index,
                        });
                    }
                }
                attribute.end()?;
                local_variables.extend(entries);
                Ok(())
            })(),
            // CLDC StackMap and later StackMapTable are advisory. Frames are
            // reconstructed from code, so stale obfuscator metadata is harmless.
            _ => Ok(()),
        };
        if let Err(error) = result {
            warnings.push(format!("ignored malformed {attr_name} in Code: {error:#}"));
        }
    }
    input.end()?;
    Ok(Code {
        max_stack,
        max_locals,
        bytes,
        exception_table,
        line_numbers,
        local_variables,
    })
}

fn parse_pool(input: &mut Reader<'_>) -> Result<ConstantPool> {
    let mut raw_utf16 = HashMap::new();
    let entries = input.pool(|index, tag, bytes| {
        let mut input = Reader::new(bytes);
        Ok(match tag {
            1 => {
                let units = decode_modified_utf8(bytes)
                    .with_context(|| format!("UTF-8 constant #{index}"))?;
                let text = match String::from_utf16(&units) {
                    Ok(text) => text,
                    Err(_) => {
                        let text = String::from_utf16_lossy(&units);
                        raw_utf16.insert(index, units);
                        text
                    }
                };
                Constant::Utf8(text)
            }
            3 => Constant::Integer(input.u4()? as i32),
            4 => Constant::Float(f32::from_bits(input.u4()?)),
            5 => Constant::Long(input.u8()? as i64),
            6 => Constant::Double(f64::from_bits(input.u8()?)),
            7 => Constant::Class(input.u2()?),
            8 => Constant::String(input.u2()?),
            9 => Constant::Fieldref {
                class_index: input.u2()?,
                name_and_type_index: input.u2()?,
            },
            10 => Constant::Methodref {
                class_index: input.u2()?,
                name_and_type_index: input.u2()?,
            },
            11 => Constant::InterfaceMethodref {
                class_index: input.u2()?,
                name_and_type_index: input.u2()?,
            },
            12 => Constant::NameAndType {
                name_index: input.u2()?,
                descriptor_index: input.u2()?,
            },
            15 => Constant::MethodHandle {
                reference_kind: input.u1()?,
                reference_index: input.u2()?,
            },
            16 => Constant::MethodType(input.u2()?),
            17 => Constant::Dynamic {
                bootstrap_index: input.u2()?,
                name_and_type_index: input.u2()?,
            },
            18 => Constant::InvokeDynamic {
                bootstrap_index: input.u2()?,
                name_and_type_index: input.u2()?,
            },
            19 => Constant::Module(input.u2()?),
            20 => Constant::Package(input.u2()?),
            _ => unreachable!("constant-pool framing validated the tag"),
        })
    })?;
    Ok(ConstantPool { entries, raw_utf16 })
}

impl ConstantPool {
    pub fn get(&self, index: u16) -> Result<&Constant> {
        match self.entries.get(index as usize) {
            Some(Constant::Unusable) | None => bail!("invalid constant-pool index {index}"),
            Some(value) => Ok(value),
        }
    }
    pub fn utf8(&self, index: u16) -> Result<&str> {
        match self.get(index)? {
            Constant::Utf8(value) => Ok(value),
            _ => bail!("constant #{index} is not UTF-8"),
        }
    }
    pub fn class_name(&self, index: u16) -> Result<&str> {
        match self.get(index)? {
            Constant::Class(name) => self.utf8(*name),
            _ => bail!("constant #{index} is not a class"),
        }
    }
    pub fn name_and_type(&self, index: u16) -> Result<(&str, &str)> {
        match self.get(index)? {
            Constant::NameAndType {
                name_index,
                descriptor_index,
            } => Ok((self.utf8(*name_index)?, self.utf8(*descriptor_index)?)),
            _ => bail!("constant #{index} is not a name and type"),
        }
    }
    pub fn member_ref(&self, index: u16) -> Result<MemberRef<'_>> {
        let (class, signature, interface) = match self.get(index)? {
            Constant::Fieldref {
                class_index,
                name_and_type_index,
            }
            | Constant::Methodref {
                class_index,
                name_and_type_index,
            } => (*class_index, *name_and_type_index, false),
            Constant::InterfaceMethodref {
                class_index,
                name_and_type_index,
            } => (*class_index, *name_and_type_index, true),
            _ => bail!("constant #{index} is not a member reference"),
        };
        let (name, descriptor) = self.name_and_type(signature)?;
        Ok(MemberRef {
            owner: self.class_name(class)?,
            name,
            descriptor,
            interface,
        })
    }
    /// Read a CONSTANT_String without losing unpaired UTF-16 surrogates.
    pub fn string_utf16(&self, index: u16) -> Result<Vec<u16>> {
        match self.get(index)? {
            Constant::String(utf8) => self.utf16(*utf8),
            _ => bail!("constant #{index} is not a string"),
        }
    }
    pub fn utf16(&self, index: u16) -> Result<Vec<u16>> {
        if let Some(units) = self.raw_utf16.get(&index) {
            return Ok(units.clone());
        }
        Ok(self.utf8(index)?.encode_utf16().collect())
    }
}

/// Java's modified UTF-8 encodes UTF-16 code units, including lone surrogates.
pub fn decode_modified_utf8(bytes: &[u8]) -> Result<Vec<u16>> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        let first = bytes[position];
        position += 1;
        let unit = match first {
            0x00..=0x7f => u16::from(first),
            0xc0..=0xdf => {
                let second = *bytes
                    .get(position)
                    .context("truncated modified UTF-8 pair")?;
                position += 1;
                ensure!(second & 0xc0 == 0x80, "invalid modified UTF-8 continuation");
                let unit = (u16::from(first & 0x1f) << 6) | u16::from(second & 0x3f);
                ensure!(unit == 0 || unit >= 0x80, "overlong modified UTF-8 pair");
                unit
            }
            0xe0..=0xef => {
                let second = *bytes
                    .get(position)
                    .context("truncated modified UTF-8 triple")?;
                let third = *bytes
                    .get(position + 1)
                    .context("truncated modified UTF-8 triple")?;
                position += 2;
                ensure!(
                    second & 0xc0 == 0x80 && third & 0xc0 == 0x80,
                    "invalid modified UTF-8 continuation"
                );
                let unit = (u16::from(first & 15) << 12)
                    | (u16::from(second & 63) << 6)
                    | u16::from(third & 63);
                ensure!(unit >= 0x800, "overlong modified UTF-8 triple");
                unit
            }
            _ => bail!("invalid modified UTF-8 lead byte {first:02x}"),
        };
        result.push(unit);
    }
    Ok(result)
}
