//! Lossless classfile envelope. Original UTF8 and NameAndType entries are never
//! edited: the same entry may also be a literal, attribute name, or unrelated
//! member's spelling. References acquire role-specific replacement edges.
use super::{descriptor, literals};
use crate::{
    Result,
    classfile::{ClassFile, Constant, MethodDescriptor},
    mappings::MappingPlan,
    naming::NamePlan,
    program::{MemberId, Program},
};
use anyhow::{Context, bail, ensure};
use std::collections::{BTreeMap, HashMap, HashSet};

struct Entry {
    tag: u8,
    data: Vec<u8>,
}

struct Pool {
    entries: Vec<Option<Entry>>,
    utf8: HashMap<Vec<u8>, u16>,
    pairs: HashMap<(u16, u16), u16>,
    strings: HashMap<u16, u16>,
}

impl Pool {
    fn read(input: &mut Reader<'_>) -> Result<Self> {
        let count = input.u2()? as usize;
        ensure!(count > 0, "empty constant pool");
        let mut pool = Self {
            entries: vec![None],
            utf8: HashMap::new(),
            pairs: HashMap::new(),
            strings: HashMap::new(),
        };
        while pool.entries.len() < count {
            let index = pool.entries.len() as u16;
            let tag = input.u1()?;
            let data = match tag {
                1 => {
                    let length = input.u2()? as usize;
                    input.take(length)?.to_vec()
                }
                3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => input.take(4)?.to_vec(),
                5 | 6 => input.take(8)?.to_vec(),
                7 | 8 | 16 | 19 | 20 => input.take(2)?.to_vec(),
                15 => input.take(3)?.to_vec(),
                _ => bail!("unknown constant-pool tag {tag}"),
            };
            if tag == 1 {
                pool.utf8.entry(data.clone()).or_insert(index);
            }
            if tag == 12 {
                pool.pairs
                    .entry((u2(&data, 0)?, u2(&data, 2)?))
                    .or_insert(index);
            }
            if tag == 8 {
                pool.strings.entry(u2(&data, 0)?).or_insert(index);
            }
            pool.entries.push(Some(Entry { tag, data }));
            if tag == 5 || tag == 6 {
                pool.entries.push(None);
            }
        }
        ensure!(
            pool.entries.len() == count,
            "wide constant outside constant pool"
        );
        Ok(pool)
    }

    fn append(&mut self, tag: u8, data: Vec<u8>) -> Result<u16> {
        ensure!(
            self.entries.len() < u16::MAX as usize,
            "remapping exceeds JVM constant-pool limit"
        );
        let index = self.entries.len() as u16;
        self.entries.push(Some(Entry { tag, data }));
        Ok(index)
    }

    fn text(&mut self, text: &str) -> Result<u16> {
        let bytes = modified_utf8(text);
        ensure!(
            u16::try_from(bytes.len()).is_ok(),
            "remapped UTF8 constant exceeds JVM length limit"
        );
        if let Some(&index) = self.utf8.get(&bytes) {
            return Ok(index);
        }
        let index = self.append(1, bytes.clone())?;
        self.utf8.insert(bytes, index);
        Ok(index)
    }

    fn pair(&mut self, name: &str, descriptor: &str) -> Result<u16> {
        let name = self.text(name)?;
        let descriptor = self.text(descriptor)?;
        self.pair_indices(name, descriptor)
    }

    fn pair_indices(&mut self, name: u16, descriptor: u16) -> Result<u16> {
        if let Some(&index) = self.pairs.get(&(name, descriptor)) {
            return Ok(index);
        }
        let mut bytes = name.to_be_bytes().to_vec();
        bytes.extend_from_slice(&descriptor.to_be_bytes());
        let index = self.append(12, bytes)?;
        self.pairs.insert((name, descriptor), index);
        Ok(index)
    }

    fn string(&mut self, text: &str) -> Result<u16> {
        let utf8 = self.text(text)?;
        if let Some(&index) = self.strings.get(&utf8) {
            return Ok(index);
        }
        let index = self.append(8, utf8.to_be_bytes().to_vec())?;
        self.strings.insert(utf8, index);
        Ok(index)
    }

    fn class(&mut self, name: &str) -> Result<u16> {
        let name = self.text(name)?;
        self.append(7, name.to_be_bytes().to_vec())
    }

    fn method(&mut self, owner: u16, name: &str, descriptor: &str) -> Result<u16> {
        let pair = self.pair(name, descriptor)?;
        let mut bytes = owner.to_be_bytes().to_vec();
        bytes.extend_from_slice(&pair.to_be_bytes());
        self.append(10, bytes)
    }

    fn write(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&(self.entries.len() as u16).to_be_bytes());
        for entry in self.entries.iter().flatten() {
            output.push(entry.tag);
            if entry.tag == 1 {
                output.extend_from_slice(&(entry.data.len() as u16).to_be_bytes());
            }
            output.extend_from_slice(&entry.data);
        }
    }
}

/// A reservation contains only new entries and redirects. Original pool bytes
/// remain immutable until every connected group has a legal width decision.
#[derive(Default)]
struct LiteralReservation {
    entries: Vec<Entry>,
    texts: HashMap<Vec<u8>, u16>,
    strings: HashMap<u16, u16>,
    redirects: BTreeMap<u16, u16>,
    relocated: HashSet<u16>,
}

impl LiteralReservation {
    fn append(&mut self, pool: &Pool, tag: u8, data: Vec<u8>) -> Result<u16> {
        let index = u16::try_from(pool.entries.len() + self.entries.len())
            .ok()
            .filter(|index| *index < u16::MAX)
            .context("remapping exceeds JVM constant-pool limit")?;
        self.entries.push(Entry { tag, data });
        Ok(index)
    }

    fn text(&mut self, pool: &Pool, text: &str) -> Result<u16> {
        let bytes = modified_utf8(text);
        ensure!(
            u16::try_from(bytes.len()).is_ok(),
            "remapped UTF8 constant exceeds JVM length limit"
        );
        if let Some(&index) = pool.utf8.get(&bytes).or_else(|| self.texts.get(&bytes)) {
            return Ok(index);
        }
        let index = self.append(pool, 1, bytes.clone())?;
        self.texts.insert(bytes, index);
        Ok(index)
    }

    fn string(&mut self, pool: &Pool, text: &str) -> Result<u16> {
        let utf8 = self.text(pool, text)?;
        if let Some(&index) = self.strings.get(&utf8).or_else(|| {
            pool.strings
                .get(&utf8)
                .filter(|index| !self.relocated.contains(index))
        }) {
            return Ok(index);
        }
        let index = self.append(pool, 8, utf8.to_be_bytes().to_vec())?;
        self.strings.insert(utf8, index);
        Ok(index)
    }

    fn replacement(&mut self, pool: &Pool, replacement: &literals::Replacement) -> Result<u16> {
        if self.relocated.contains(&replacement.original) {
            let text = self.text(pool, &replacement.text)?;
            self.redirects.insert(replacement.original, text);
            Ok(replacement.original)
        } else {
            self.string(pool, &replacement.text)
        }
    }

    fn commit(self, pool: &mut Pool) {
        pool.entries.extend(self.entries.into_iter().map(Some));
        pool.utf8.extend(self.texts);
        for (index, text) in self.redirects {
            pool.entries[index as usize].as_mut().unwrap().data = text.to_be_bytes().to_vec();
        }
        pool.strings.clear();
        for (index, entry) in pool.entries.iter().enumerate() {
            if let Some(entry) = entry.as_ref().filter(|entry| entry.tag == 8) {
                let text = u16::from_be_bytes(entry.data[..2].try_into().unwrap());
                pool.strings.entry(text).or_insert(index as u16);
            }
        }
    }
}

fn resolve_literals(
    pool: &mut Pool,
    parsed: &ClassFile,
    names: &NamePlan,
    mut plan: literals::Plan,
) -> Result<literals::ResolvedPlan> {
    // Empty semantic plans do not scan bytecode or allocate a pool snapshot.
    if plan.fields.is_empty()
        && plan
            .methods
            .values()
            .all(|method| method.producers.is_empty())
    {
        return Ok(plan.finish(names, literals::ResolvedPlan::default()));
    }
    let instructions: Vec<_> = parsed
        .methods
        .iter()
        .map(|method| {
            method.code.as_ref().map_or_else(
                || Ok(Vec::new()),
                |code| crate::bytecode::decode(&code.bytes),
            )
        })
        .collect::<Result<_>>()?;
    loop {
        let mut uses = HashMap::<u16, Option<&str>>::new();
        fn observe<'a>(
            uses: &mut HashMap<u16, Option<&'a str>>,
            parsed: &ClassFile,
            constant: u16,
            replacement: Option<&'a literals::Replacement>,
        ) {
            if !matches!(
                parsed.constant_pool.entries.get(constant as usize),
                Some(Constant::String(_))
            ) {
                return;
            }
            let text = replacement.map(|replacement| replacement.text.as_str());
            uses.entry(constant)
                .and_modify(|previous| {
                    if *previous != text {
                        *previous = None;
                    }
                })
                .or_insert(text);
        }
        for (index, field) in parsed.fields.iter().enumerate() {
            if let Some(constant) = field.constant_value {
                observe(&mut uses, parsed, constant, plan.fields.get(&index));
            }
        }
        for (method, instructions) in instructions.iter().enumerate() {
            for instruction in instructions {
                if let crate::bytecode::Operand::Constant(constant) = instruction.operand {
                    let replacement = plan.methods.get(&method).and_then(|method| {
                        method
                            .rewrites(instruction.pc)
                            .then(|| method.producers.get(&instruction.pc))
                            .flatten()
                    });
                    observe(&mut uses, parsed, constant, replacement);
                }
            }
        }
        let mut reservation = LiteralReservation {
            relocated: uses
                .into_iter()
                .filter_map(|(index, text)| text.map(|_| index))
                .collect(),
            ..Default::default()
        };
        let mut overflow = Vec::new();
        let mut candidate = literals::ResolvedPlan::default();
        for (&method, changes) in &plan.methods {
            for (&pc, change) in &changes.producers {
                if !changes.rewrites(pc) {
                    continue;
                }
                let index = reservation.replacement(pool, change)?;
                if index > 255
                    && parsed.methods[method].code.as_ref().unwrap().bytes[pc as usize] == 0x12
                {
                    overflow.push((method, pc));
                }
                candidate
                    .methods
                    .entry(method)
                    .or_default()
                    .insert(pc, index);
            }
        }
        for (&field, change) in &plan.fields {
            candidate
                .fields
                .insert(field, reservation.replacement(pool, change)?);
        }
        if overflow.is_empty() {
            reservation.commit(pool);
            return Ok(plan.finish(names, candidate));
        }
        // Monotone group decisions, no pool rollback and no recursive retry.
        // Every iteration promotes at least one producer component permanently.
        for (method, pc) in overflow {
            let method = plan.methods.get_mut(&method).unwrap();
            if method.rewrites(pc) {
                method.require_adapter(pc)?;
            }
        }
    }
}

pub(super) fn remap(
    bytes: &[u8],
    program: &Program<'_>,
    owner: usize,
    names: &NamePlan,
    mappings: &MappingPlan,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>> {
    let parsed = program.class(owner);
    let mut input = Reader::new(bytes);
    let header = input.take(8)?.to_vec();
    let mut pool = Pool::read(&mut input)?;
    let tail = input.take(bytes.len() - input.position)?.to_vec();
    let literal_plan = literals::plan(program, names, mappings, owner, warnings)?;
    // Resolve provenance and instruction width before changing any pool entry.
    let literal_plan = resolve_literals(&mut pool, parsed, names, literal_plan)?;
    let mut rewrite = Rewrite {
        program,
        names,
        parsed,
        owner,
        pool,
        tail,
        patches: BTreeMap::new(),
        literals: literal_plan,
        bridges: Vec::new(),
        bridge_names: HashSet::new(),
        method_end: 0,
    };
    rewrite.pool_names()?;
    rewrite.structure()?;
    for (offset, index) in rewrite.patches {
        rewrite.tail[offset..offset + 2].copy_from_slice(&index.to_be_bytes());
    }
    if !rewrite.bridges.is_empty() {
        let methods: Vec<u8> = rewrite.bridges.into_iter().flatten().collect();
        rewrite
            .tail
            .splice(rewrite.method_end..rewrite.method_end, methods);
    }
    let mut output = header;
    rewrite.pool.write(&mut output);
    output.extend_from_slice(&rewrite.tail);
    // Parsing is a final executable-structure check; it does not serialize data.
    let check = ClassFile::parse(&output).context("validate remapped class")?;
    ensure!(
        check.name == names.class_name(owner),
        "remapped class identity mismatch"
    );
    Ok(output)
}

struct Rewrite<'a, 'p> {
    program: &'a Program<'p>,
    names: &'a NamePlan,
    parsed: &'a ClassFile,
    owner: usize,
    pool: Pool,
    tail: Vec<u8>,
    patches: BTreeMap<usize, u16>,
    literals: literals::ResolvedPlan,
    bridges: Vec<Vec<u8>>,
    bridge_names: HashSet<String>,
    method_end: usize,
}

impl Rewrite<'_, '_> {
    fn pool_names(&mut self) -> Result<()> {
        for (index, constant) in self.parsed.constant_pool.entries.iter().enumerate() {
            match constant {
                Constant::Class(name) => {
                    let original = self.parsed.constant_pool.utf8(*name)?;
                    let mapped = if original.starts_with('[') {
                        descriptor(original, self.names)?
                    } else {
                        self.names.class_name_for(original).to_owned()
                    };
                    if mapped != original {
                        let edge = self.pool.text(&mapped)?;
                        self.pool.entries[index].as_mut().unwrap().data =
                            edge.to_be_bytes().to_vec();
                    }
                }
                Constant::Fieldref { .. }
                | Constant::Methodref { .. }
                | Constant::InterfaceMethodref { .. } => {
                    let member = self.parsed.constant_pool.member_ref(index as u16)?;
                    let name = if matches!(constant, Constant::Fieldref { .. }) {
                        self.names.resolved_field_name(
                            self.program,
                            member.owner,
                            member.name,
                            member.descriptor,
                        )
                    } else {
                        self.names.resolved_method_name(
                            self.program,
                            member.owner,
                            member.name,
                            member.descriptor,
                        )
                    };
                    let descriptor = descriptor(member.descriptor, self.names)?;
                    let original = u2(&self.pool.entries[index].as_ref().unwrap().data, 2)?;
                    let edge = self.mapped_pair(original, name, &descriptor)?;
                    self.pool.entries[index].as_mut().unwrap().data[2..]
                        .copy_from_slice(&edge.to_be_bytes());
                }
                Constant::MethodType(index_utf8) => {
                    let original = self.parsed.constant_pool.utf8(*index_utf8)?;
                    let mapped = descriptor(original, self.names)?;
                    if mapped != original {
                        let edge = self.pool.text(&mapped)?;
                        self.pool.entries[index].as_mut().unwrap().data =
                            edge.to_be_bytes().to_vec();
                    }
                }
                Constant::Dynamic { .. }
                | Constant::InvokeDynamic { .. }
                | Constant::Module(_)
                | Constant::Package(_) => {
                    bail!(
                        "dynamic constants, invokedynamic and modules are outside J2ME bytecode remapping"
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn text_edge(&mut self, offset: usize, value: &str) -> Result<()> {
        let original = u2(&self.tail, offset)?;
        if original != 0 && self.parsed.constant_pool.utf8(original)? == value {
            return Ok(());
        }
        let index = self.pool.text(value)?;
        self.patches.insert(offset, index);
        Ok(())
    }

    fn mapped_pair(&mut self, original: u16, name: &str, descriptor: &str) -> Result<u16> {
        let Constant::NameAndType {
            name_index,
            descriptor_index,
        } = self.parsed.constant_pool.entries[original as usize]
        else {
            bail!("invalid original NameAndType")
        };
        let name_index = if self.parsed.constant_pool.utf8(name_index)? == name {
            name_index
        } else {
            self.pool.text(name)?
        };
        let descriptor_index = if self.parsed.constant_pool.utf8(descriptor_index)? == descriptor {
            descriptor_index
        } else {
            self.pool.text(descriptor)?
        };
        self.pool.pair_indices(name_index, descriptor_index)
    }

    fn descriptor_edge(&mut self, offset: usize) -> Result<()> {
        let original = self.parsed.constant_pool.utf8(u2(&self.tail, offset)?)?;
        self.text_edge(offset, &descriptor(original, self.names)?)
    }

    fn structure(&mut self) -> Result<()> {
        let mut position = 6;
        let interfaces = u2(&self.tail, position)? as usize;
        position += 2 + interfaces * 2;
        for methods in [false, true] {
            let count_offset = position;
            let count = u2(&self.tail, position)? as usize;
            position += 2;
            let expected = if methods {
                self.parsed.methods.len()
            } else {
                self.parsed.fields.len()
            };
            ensure!(count == expected, "member count mismatch");
            let mut signatures = HashSet::new();
            for index in 0..count {
                let member = MemberId {
                    class: self.owner,
                    index,
                };
                let name = if methods {
                    self.names.method_name(member)
                } else {
                    self.names.field_name(member)
                };
                let original_descriptor = self
                    .parsed
                    .constant_pool
                    .utf8(u2(&self.tail, position + 4)?)?;
                ensure!(
                    signatures.insert((
                        name.to_owned(),
                        descriptor(original_descriptor, self.names)?
                    )),
                    "duplicate mapped member {}.{name}",
                    self.names.class_name(self.owner)
                );
                self.text_edge(position + 2, name)?;
                self.descriptor_edge(position + 4)?;
                position += 6;
                position = self.attributes(
                    position,
                    if methods {
                        Scope::Method(index)
                    } else {
                        Scope::Field(index)
                    },
                )?;
            }
            if methods {
                self.method_end = position;
                let count = u16::try_from(count + self.bridges.len())
                    .context("synthetic ClassName bridges exceed JVM method count")?;
                self.patches.insert(count_offset, count);
            }
        }
        position = self.attributes(position, Scope::Class)?;
        ensure!(position == self.tail.len(), "trailing classfile bytes");
        Ok(())
    }

    fn attributes(&mut self, mut position: usize, scope: Scope) -> Result<usize> {
        let count = u2(&self.tail, position)?;
        position += 2;
        for _ in 0..count {
            let name = self
                .parsed
                .constant_pool
                .utf8(u2(&self.tail, position)?)?
                .to_owned();
            let size = u4(&self.tail, position + 2)? as usize;
            let start = position + 6;
            let end = start
                .checked_add(size)
                .context("attribute length overflow")?;
            ensure!(end <= self.tail.len(), "truncated {name} attribute");
            match name.as_str() {
                "Code" => {
                    let Scope::Method(method) = scope else {
                        bail!("Code outside method")
                    };
                    ensure!(size >= 12, "truncated Code attribute");
                    let length = u4(&self.tail, start + 4)? as usize;
                    let code_start = start + 8;
                    let code_end = code_start
                        .checked_add(length)
                        .context("Code length overflow")?;
                    ensure!(code_end + 2 <= end, "truncated Code bytes");
                    if let Some(changes) = self.literals.methods.get(&method) {
                        for (&pc, change) in changes {
                            let offset = code_start + pc as usize;
                            ensure!(offset < code_end, "ClassName literal offset outside Code");
                            match self.tail[offset] {
                                0x12 => {
                                    ensure!(
                                        *change <= 255,
                                        "ClassName literal at {}.{} bytecode {pc} requires ldc widening; refusing to change instruction PCs",
                                        self.parsed.name,
                                        self.parsed.methods[method].name
                                    );
                                    self.tail[offset + 1] = *change as u8;
                                }
                                0x13 => {
                                    self.patches.insert(offset + 1, *change);
                                }
                                _ => bail!(
                                    "ClassName literal at bytecode {pc} is not an ldc instruction"
                                ),
                            }
                        }
                    }
                    if let Some(calls) = self.literals.adapters.remove(&method) {
                        for (pc, arguments) in calls {
                            let offset = code_start + pc as usize;
                            ensure!(offset + 3 <= code_end, "ClassName call outside Code");
                            let opcode = self.tail[offset];
                            ensure!(
                                matches!(opcode, 0xb3 | 0xb5 | 0xb6..=0xb9),
                                "unsupported ClassName consumer opcode {opcode:#x}"
                            );
                            ensure!(
                                !self.literals.uninitialized_stores.contains(&(method, pc)),
                                "ClassName field adapter at {}.{} bytecode {pc} would pass uninitialized this before constructor delegation",
                                self.parsed.name,
                                self.parsed.methods[method].name
                            );
                            let target = u2(&self.tail, offset + 1)?;
                            let count = if opcode == 0xb9 {
                                self.tail[offset + 3]
                            } else {
                                0
                            };
                            let bridge = self.bridge(opcode, target, count, &arguments)?;
                            self.tail[offset] = 0xb8;
                            self.patches.insert(offset + 1, bridge);
                            if opcode == 0xb9 {
                                self.tail[offset + 3..offset + 5].fill(0);
                            }
                        }
                    }
                    let handlers = u2(&self.tail, code_end)? as usize;
                    let attributes = code_end + 2 + handlers * 8;
                    ensure!(attributes + 2 <= end, "truncated Code handlers");
                    ensure!(
                        self.attributes(attributes, Scope::Code(method))? == end,
                        "invalid Code attribute length"
                    );
                }
                "ConstantValue" => {
                    ensure!(size == 2, "invalid ConstantValue length");
                    if let Scope::Field(field) = scope
                        && let Some(change) = self.literals.fields.get(&field)
                    {
                        self.patches.insert(start, *change);
                    }
                }
                "SourceFile" => {
                    ensure!(size == 2, "invalid SourceFile length");
                    if self.names.class_name(self.owner) != self.parsed.name {
                        let simple = self
                            .names
                            .class_name(self.owner)
                            .rsplit('/')
                            .next()
                            .unwrap();
                        self.text_edge(start, &format!("{simple}.java"))?;
                    }
                }
                "LocalVariableTable" | "LocalVariableTypeTable" => {
                    let count = u2(&self.tail, start)? as usize;
                    ensure!(size == 2 + 10 * count, "invalid {name} length");
                    for local in 0..count {
                        let entry = start + 2 + 10 * local;
                        if name == "LocalVariableTable" {
                            self.descriptor_edge(entry + 6)?;
                        } else {
                            self.signature_edge(entry + 6)?;
                        }
                        if let Scope::Code(method) = scope
                            && u2(&self.tail, entry)? == 0
                            && let Some(name) =
                                self.parameter(method, u2(&self.tail, entry + 8)?)?
                        {
                            self.text_edge(entry + 4, &name)?;
                        }
                    }
                }
                "MethodParameters" => {
                    let count =
                        *self.tail.get(start).context("truncated MethodParameters")? as usize;
                    ensure!(size == 1 + count * 4, "invalid MethodParameters length");
                    if let Scope::Method(method) = scope {
                        let member = MemberId {
                            class: self.owner,
                            index: method,
                        };
                        for ordinal in 0..count {
                            if let Some(name) = self.names.parameter_name(member, ordinal) {
                                self.text_edge(start + 1 + ordinal * 4, name)?;
                            }
                        }
                    }
                }
                "Signature" => {
                    ensure!(size == 2, "invalid Signature length");
                    self.signature_edge(start)?;
                }
                "InnerClasses" => {
                    let count = u2(&self.tail, start)? as usize;
                    ensure!(size == 2 + 8 * count, "invalid InnerClasses length");
                    for i in 0..count {
                        let entry = start + 2 + 8 * i;
                        let inner = self
                            .parsed
                            .constant_pool
                            .class_name(u2(&self.tail, entry)?)?;
                        let mapped = self.names.class_name_for(inner);
                        if u2(&self.tail, entry + 4)? != 0 && mapped != inner {
                            let outer = u2(&self.tail, entry + 2)?;
                            let prefix = if outer != 0 {
                                format!(
                                    "{}$",
                                    self.names.class_name_for(
                                        self.parsed.constant_pool.class_name(outer)?
                                    )
                                )
                            } else {
                                String::new()
                            };
                            let simple = mapped
                                .strip_prefix(&prefix)
                                .filter(|_| !prefix.is_empty())
                                .unwrap_or_else(|| mapped.rsplit('/').next().unwrap());
                            self.text_edge(entry + 4, simple)?;
                        }
                    }
                }
                "EnclosingMethod" => {
                    ensure!(size == 4, "invalid EnclosingMethod length");
                    let method = u2(&self.tail, start + 2)?;
                    if method != 0 {
                        let owner = self
                            .parsed
                            .constant_pool
                            .class_name(u2(&self.tail, start)?)?;
                        let (name, desc) = self.parsed.constant_pool.name_and_type(method)?;
                        let mapped =
                            self.names
                                .resolved_method_name(self.program, owner, name, desc);
                        let edge =
                            self.mapped_pair(method, mapped, &descriptor(desc, self.names)?)?;
                        self.patches.insert(start + 2, edge);
                    }
                }
                // Annotations are not part of the CLDC/MIDP contract. Their
                // UTF8 descriptors and element names require a separate mapper.
                "RuntimeVisibleAnnotations"
                | "RuntimeInvisibleAnnotations"
                | "RuntimeVisibleParameterAnnotations"
                | "RuntimeInvisibleParameterAnnotations"
                | "RuntimeVisibleTypeAnnotations"
                | "RuntimeInvisibleTypeAnnotations"
                | "AnnotationDefault" => {
                    bail!("{name} is outside J2ME bytecode remapping");
                }
                _ => {} // Includes StackMap/StackMapTable: Class edges stay stable.
            }
            position = end;
        }
        Ok(position)
    }

    fn parameter(&self, method: usize, slot: u16) -> Result<Option<String>> {
        let member = MemberId {
            class: self.owner,
            index: method,
        };
        let declaration = &self.parsed.methods[method];
        let descriptor = MethodDescriptor::parse(&declaration.descriptor)?;
        let mut current = if declaration.access_flags & 8 == 0 {
            1
        } else {
            0
        };
        for (ordinal, ty) in descriptor.parameters.iter().enumerate() {
            if current == slot {
                return Ok(self
                    .names
                    .parameter_name(member, ordinal)
                    .map(str::to_owned));
            }
            current += if matches!(
                ty,
                crate::classfile::JvmType::Long | crate::classfile::JvmType::Double
            ) {
                2
            } else {
                1
            };
        }
        Ok(None)
    }

    fn signature_edge(&mut self, offset: usize) -> Result<()> {
        let original = self.parsed.constant_pool.utf8(u2(&self.tail, offset)?)?;
        self.text_edge(offset, &signature(original, self.names)?)
    }

    /// Append a static adapter and route the original invoke through it. The
    /// original instruction has the same stack effect and PCs. Finite original
    /// candidates are compared once, so rename chains never translate twice.
    fn bridge(
        &mut self,
        opcode: u8,
        target: u16,
        interface_count: u8,
        arguments: &[literals::AdapterInput],
    ) -> Result<u16> {
        ensure!(
            self.parsed.access_flags & 0x0200 == 0,
            "ClassName bridge in a legacy interface is unsupported"
        );
        let reference = self.parsed.constant_pool.member_ref(target)?;
        let field = matches!(opcode, 0xb3 | 0xb5);
        if field {
            let member = self
                .program
                .resolve_field(reference.owner, reference.name, reference.descriptor)
                .context("ClassName field adapter target is unresolved")?;
            ensure!(
                self.program.field(member).access_flags & 0x0010 == 0,
                "ClassName field adapter cannot move final field write {}.{} out of its initializer",
                reference.owner,
                reference.name
            );
        }
        ensure!(
            reference.name != "<init>",
            "shared ClassName constructor argument requires rewriting uninitialized references"
        );
        let target_descriptor = if field {
            format!("({})V", reference.descriptor)
        } else {
            reference.descriptor.to_owned()
        };
        let original_descriptor = MethodDescriptor::parse(&target_descriptor)?;
        let mut descriptor = descriptor(&target_descriptor, self.names)?;
        let receiver = !matches!(opcode, 0xb3 | 0xb8);
        if receiver {
            let resolved = if field {
                self.program
                    .resolve_field(reference.owner, reference.name, reference.descriptor)
            } else {
                self.program
                    .resolve_method(reference.owner, reference.name, reference.descriptor)
            };
            let protected_receiver = resolved.is_some_and(|member| {
                let flags = if field {
                    self.program.field(member).access_flags
                } else {
                    self.program.method(member).access_flags
                };
                let package = |name: &str| {
                    name.rsplit_once('/')
                        .map_or("", |(package, _)| package)
                        .to_owned()
                };
                flags & 4 != 0
                    && package(self.names.class_name(member.class))
                        != package(self.names.class_name(self.owner))
            });
            let owner = if opcode == 0xb7 || protected_receiver {
                self.names.class_name(self.owner)
            } else {
                self.names.class_name_for(reference.owner)
            };
            descriptor.insert_str(1, &format!("L{owner};"));
        }
        let mut suffix = self.bridges.len();
        let name = loop {
            let name = format!("$sf$className${suffix}");
            if !(0..self.parsed.methods.len()).any(|index| {
                self.names.method_name(MemberId {
                    class: self.owner,
                    index,
                }) == name
            }) && self.bridge_names.insert(name.clone())
            {
                break name;
            }
            suffix += 1;
        };
        let mut code = Vec::new();
        let mut frames = std::collections::BTreeSet::new();
        let string_class = self.pool.class("java/lang/String")?;
        let equals = self
            .pool
            .method(string_class, "equals", "(Ljava/lang/Object;)Z")?;
        let mut slots = Vec::new();
        let mut locals = u16::from(receiver);
        for ty in &original_descriptor.parameters {
            slots.push(locals);
            locals += if matches!(
                ty,
                crate::classfile::JvmType::Long | crate::classfile::JvmType::Double
            ) {
                2
            } else {
                1
            };
        }
        for argument in arguments {
            let (slot, names) = match argument {
                literals::AdapterInput::Argument { parameter, names } => {
                    ensure!(
                        !field
                            && original_descriptor.parameters.get(*parameter)
                                == Some(&crate::classfile::JvmType::Object(
                                    "java/lang/String".into()
                                )),
                        "ClassName bridge requires a scalar String parameter"
                    );
                    (slots[*parameter], names)
                }
                literals::AdapterInput::Field { names } => {
                    ensure!(
                        field && reference.descriptor == "Ljava/lang/String;",
                        "ClassName field bridge requires a scalar String value"
                    );
                    (u16::from(receiver), names)
                }
            };
            let mut ends = Vec::new();
            for original in names {
                let Some(mapped) = crate::semantics::class_names::relocated(original, self.names)
                    .filter(|mapped| mapped != original)
                else {
                    continue;
                };
                emit_constant(&mut code, self.pool.string(original)?);
                emit_local(&mut code, 0x19, slot);
                code.push(0xb6);
                emit_u2(&mut code, equals);
                let skip = code.len();
                code.extend_from_slice(&[0x99, 0, 0]);
                emit_constant(&mut code, self.pool.string(&mapped)?);
                emit_local(&mut code, 0x3a, slot);
                ends.push(code.len());
                code.extend_from_slice(&[0xa7, 0, 0]);
                let end = code.len();
                branch(&mut code, skip, end)?;
                frames.insert(end);
            }
            let end = code.len();
            for jump in ends {
                branch(&mut code, jump, end)?;
                frames.insert(end);
            }
        }
        if receiver {
            emit_local(&mut code, 0x19, 0);
        }
        for (ty, &slot) in original_descriptor.parameters.iter().zip(&slots) {
            let opcode = match ty {
                crate::classfile::JvmType::Long => 0x16,
                crate::classfile::JvmType::Float => 0x17,
                crate::classfile::JvmType::Double => 0x18,
                crate::classfile::JvmType::Object(_) | crate::classfile::JvmType::Array(_) => 0x19,
                _ => 0x15,
            };
            emit_local(&mut code, opcode, slot);
        }
        code.push(opcode);
        emit_u2(&mut code, target);
        if opcode == 0xb9 {
            code.extend_from_slice(&[interface_count, 0]);
        }
        code.push(match original_descriptor.return_type {
            crate::classfile::JvmType::Void => 0xb1,
            crate::classfile::JvmType::Long => 0xad,
            crate::classfile::JvmType::Float => 0xae,
            crate::classfile::JvmType::Double => 0xaf,
            crate::classfile::JvmType::Object(_) | crate::classfile::JvmType::Array(_) => 0xb0,
            _ => 0xac,
        });
        ensure!(
            code.len() <= 65535,
            "ClassName adapter exceeds JVM Code limit"
        );
        let mut payload = Vec::new();
        emit_u2(&mut payload, locals.max(2));
        emit_u2(&mut payload, locals);
        payload.extend_from_slice(&(code.len() as u32).to_be_bytes());
        payload.extend_from_slice(&code);
        emit_u2(&mut payload, 0); // exception table
        if self.parsed.major_version >= 50 && !frames.is_empty() {
            emit_u2(&mut payload, 1);
            let mut stack_map = Vec::new();
            emit_u2(&mut stack_map, frames.len() as u16);
            let mut previous = -1isize;
            for pc in frames {
                stack_map.push(251);
                emit_u2(&mut stack_map, (pc as isize - previous - 1) as u16);
                previous = pc as isize;
            }
            attribute(&mut payload, self.pool.text("StackMapTable")?, &stack_map);
        } else if !frames.is_empty() {
            // CLDC preverification uses the original full-frame StackMap
            // attribute, not the Java 6 StackMapTable format. The adapter has
            // unchanged parameter locals and an empty stack at every label.
            emit_u2(&mut payload, 1);
            let parsed = MethodDescriptor::parse(&descriptor)?;
            let mut locals_types = Vec::new();
            for ty in &parsed.parameters {
                match ty {
                    crate::classfile::JvmType::Float => locals_types.push(2),
                    crate::classfile::JvmType::Double => locals_types.push(3),
                    crate::classfile::JvmType::Long => locals_types.push(4),
                    crate::classfile::JvmType::Object(name) => {
                        locals_types.push(7);
                        emit_u2(&mut locals_types, self.pool.class(name)?);
                    }
                    crate::classfile::JvmType::Array(_) => {
                        locals_types.push(7);
                        emit_u2(&mut locals_types, self.pool.class(&ty.descriptor())?);
                    }
                    _ => locals_types.push(1),
                }
            }
            let mut stack_map = Vec::new();
            emit_u2(&mut stack_map, frames.len() as u16);
            for pc in frames {
                emit_u2(&mut stack_map, pc as u16);
                emit_u2(&mut stack_map, parsed.parameters.len() as u16);
                stack_map.extend_from_slice(&locals_types);
                emit_u2(&mut stack_map, 0);
            }
            attribute(&mut payload, self.pool.text("StackMap")?, &stack_map);
        } else {
            emit_u2(&mut payload, 0);
        }
        let mut method = Vec::new();
        let legacy_synthetic = self.parsed.major_version < 49;
        emit_u2(&mut method, if legacy_synthetic { 0x000a } else { 0x100a });
        emit_u2(&mut method, self.pool.text(&name)?);
        emit_u2(&mut method, self.pool.text(&descriptor)?);
        emit_u2(&mut method, if legacy_synthetic { 2 } else { 1 });
        attribute(&mut method, self.pool.text("Code")?, &payload);
        if legacy_synthetic {
            attribute(&mut method, self.pool.text("Synthetic")?, &[]);
        }
        self.bridges.push(method);
        self.pool.method(u2(&self.tail, 2)?, &name, &descriptor)
    }
}

fn emit_u2(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}
fn emit_constant(output: &mut Vec<u8>, index: u16) {
    if index <= 255 {
        output.extend_from_slice(&[0x12, index as u8]);
    } else {
        output.push(0x13);
        emit_u2(output, index);
    }
}
fn emit_local(output: &mut Vec<u8>, opcode: u8, slot: u16) {
    if slot <= 255 {
        output.extend_from_slice(&[opcode, slot as u8]);
    } else {
        output.extend_from_slice(&[0xc4, opcode]);
        emit_u2(output, slot);
    }
}
fn branch(code: &mut [u8], from: usize, to: usize) -> Result<()> {
    let delta = i16::try_from(to as isize - from as isize)
        .context("ClassName adapter branch exceeds JVM short branch limit")?;
    code[from + 1..from + 3].copy_from_slice(&delta.to_be_bytes());
    Ok(())
}
fn attribute(output: &mut Vec<u8>, name: u16, payload: &[u8]) {
    emit_u2(output, name);
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    output.extend_from_slice(payload);
}

#[derive(Clone, Copy)]
enum Scope {
    Class,
    Field(usize),
    Method(usize),
    Code(usize),
}

fn signature(original: &str, names: &NamePlan) -> Result<String> {
    fn sequence(
        input: &str,
        position: &mut usize,
        names: &NamePlan,
        end: Option<u8>,
    ) -> Result<String> {
        let bytes = input.as_bytes();
        let mut output = String::new();
        while *position < bytes.len() && Some(bytes[*position]) != end {
            if bytes[*position] == b'<' {
                // Formal parameter names are identifiers, not type signatures:
                // <LongName:Lold/Bound;> must not mistake LongName for a class.
                output.push('<');
                *position += 1;
                while bytes.get(*position) != Some(&b'>') {
                    let length = input[*position..]
                        .find(':')
                        .context("invalid formal type parameter")?;
                    output.push_str(&input[*position..=*position + length]);
                    *position += length + 1;
                    loop {
                        if bytes.get(*position) != Some(&b':') {
                            let start = *position;
                            while bytes.get(*position) == Some(&b'[') {
                                *position += 1;
                            }
                            match bytes.get(*position) {
                                Some(b'L' | b'T') => {
                                    let mut depth = 0usize;
                                    loop {
                                        match bytes
                                            .get(*position)
                                            .context("unterminated formal bound")?
                                        {
                                            b'<' => depth += 1,
                                            b'>' => {
                                                ensure!(depth > 0, "invalid formal bound");
                                                depth -= 1;
                                            }
                                            b';' if depth == 0 => {
                                                *position += 1;
                                                break;
                                            }
                                            _ => {}
                                        }
                                        *position += 1;
                                    }
                                }
                                Some(_) => *position += 1,
                                None => bail!("missing formal bound"),
                            }
                            output.push_str(&sequence(
                                &input[start..*position],
                                &mut 0,
                                names,
                                None,
                            )?);
                        }
                        if bytes.get(*position) != Some(&b':') {
                            break;
                        }
                        output.push(':');
                        *position += 1;
                    }
                }
                output.push('>');
                *position += 1;
                continue;
            }
            if bytes[*position] != b'L' {
                // Identifiers in formal/type variables can contain L; copy them
                // as a unit rather than misreading them as class descriptors.
                if bytes[*position] == b'T' {
                    let tail = input[*position..]
                        .find(';')
                        .context("unterminated type variable signature")?;
                    output.push_str(&input[*position..*position + tail + 1]);
                    *position += tail + 1;
                } else {
                    let ch = input[*position..].chars().next().unwrap();
                    output.push(ch);
                    *position += ch.len_utf8();
                }
                continue;
            }
            *position += 1;
            let mut segments = Vec::new();
            let mut owner = String::new();
            loop {
                let start = *position;
                while *position < bytes.len() && !matches!(bytes[*position], b';' | b'<' | b'.') {
                    *position += 1;
                }
                ensure!(
                    *position > start && *position < bytes.len(),
                    "invalid class signature {input}"
                );
                if !owner.is_empty() {
                    owner.push('$');
                }
                owner.push_str(&input[start..*position]);
                let args = if bytes[*position] == b'<' {
                    *position += 1;
                    let args = sequence(input, position, names, Some(b'>'))?;
                    ensure!(
                        bytes.get(*position) == Some(&b'>'),
                        "unterminated generic signature"
                    );
                    *position += 1;
                    format!("<{args}>")
                } else {
                    String::new()
                };
                segments.push((names.class_name_for(&owner).to_owned(), args));
                match bytes.get(*position) {
                    Some(b'.') => {
                        *position += 1;
                    }
                    Some(b';') => {
                        *position += 1;
                        break;
                    }
                    _ => bail!("invalid class signature {input}"),
                }
            }
            output.push('L');
            let mut previous = String::new();
            for (index, (mapped, args)) in segments.iter().enumerate() {
                if index == 0 {
                    output.push_str(mapped);
                } else {
                    let prefix = format!("{previous}$");
                    let suffix = mapped.strip_prefix(&prefix).context(
                        "generic inner class relocation does not preserve its enclosing type",
                    )?;
                    output.push('.');
                    output.push_str(suffix);
                }
                output.push_str(args);
                previous.clone_from(mapped);
            }
            output.push(';');
        }
        Ok(output)
    }
    sequence(original, &mut 0, names, None)
}

fn modified_utf8(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for unit in text.encode_utf16() {
        match unit {
            1..=0x7f => bytes.push(unit as u8),
            0..=0x7ff => {
                bytes.push(0xc0 | (unit >> 6) as u8);
                bytes.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                bytes.push(0xe0 | (unit >> 12) as u8);
                bytes.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    bytes
}

fn u2(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        bytes
            .get(offset..offset + 2)
            .context("truncated classfile u2")?
            .try_into()
            .unwrap(),
    ))
}
fn u4(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated classfile u4")?
            .try_into()
            .unwrap(),
    ))
}
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .context("classfile size overflow")?;
        let result = self
            .bytes
            .get(self.position..end)
            .context("truncated classfile")?;
        self.position = end;
        Ok(result)
    }
    fn u1(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u2(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
}
