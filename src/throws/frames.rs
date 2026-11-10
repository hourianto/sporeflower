//! Reference-type frames for ATHROW summaries. Numeric values retain only their
//! category; locals are sparse, so inflated max_locals does not multiply memory
//! by the number of basic blocks. No expressions or source variables are built.
use super::scan::active_handlers;
use crate::hash::HashMap;
use crate::{
    Result,
    bytecode::{
        Instruction, Operand,
        numeric::NumericOperation,
        stack::{self, StackOperation},
    },
    cfg::{BlockId, ControlFlowGraph},
    classfile::{ClassFile, Constant, JvmType, Method, MethodDescriptor},
    index::IdVec,
    program::Program,
};
use anyhow::{Context, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Origin {
    Direct(Arc<str>),
    Caught {
        block: BlockId,
        limit: Option<Arc<str>>,
    },
}

pub(super) struct ThrowValue {
    pub source_type: String,
    pub origins: BTreeSet<Origin>,
}

#[derive(Default)]
pub(super) struct Summary {
    pub throws: BTreeMap<u32, ThrowValue>,
    /// None is an unknown Class value; an empty set denotes a null receiver.
    pub reflection: BTreeMap<u32, Option<BTreeSet<Arc<str>>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Known {
    Strings(BTreeSet<Arc<str>>),
    Classes(BTreeSet<Arc<str>>),
    Instances(BTreeSet<Arc<str>>),
}

impl Known {
    fn join(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Strings(a), Self::Strings(b)) => {
                Some(Self::Strings(a.union(b).cloned().collect()))
            }
            (Self::Classes(a), Self::Classes(b)) => {
                Some(Self::Classes(a.union(b).cloned().collect()))
            }
            (Self::Instances(a), Self::Instances(b)) => {
                Some(Self::Instances(a.union(b).cloned().collect()))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    One,
    Two,
    Null,
    Reference {
        ty: Arc<str>,
        origins: BTreeSet<Origin>,
        known: Option<Known>,
    },
    UnknownReference,
}

impl Value {
    fn width(&self) -> u16 {
        if matches!(self, Self::Two) { 2 } else { 1 }
    }
    fn reference(name: &str) -> Self {
        let ty = Arc::from(name);
        Self::Reference {
            ty: Arc::clone(&ty),
            origins: BTreeSet::from([Origin::Direct(ty)]),
            known: None,
        }
    }
    fn caught(name: &str, block: BlockId) -> Self {
        Self::Reference {
            ty: Arc::from(name),
            origins: BTreeSet::from([Origin::Caught { block, limit: None }]),
            known: None,
        }
    }
    fn ty(ty: &JvmType) -> Self {
        match ty {
            JvmType::Long | JvmType::Double => Self::Two,
            JvmType::Object(name) => Self::reference(name),
            JvmType::Array(_) => Self::reference(&ty.descriptor()),
            _ => Self::One,
        }
    }
    fn known(name: &str, known: Known) -> Self {
        let mut value = Self::reference(name);
        if let Self::Reference {
            known: destination, ..
        } = &mut value
        {
            *destination = Some(known);
        }
        value
    }
}

#[derive(Clone, Debug, Default)]
struct Frame {
    locals: HashMap<u16, Value>,
    stack: Vec<Value>,
}

struct Analysis<'a, 'b> {
    program: &'a Program<'b>,
    joins: HashMap<(Arc<str>, Arc<str>), Arc<str>>,
}

pub(super) fn analyze(
    program: &Program<'_>,
    class: &ClassFile,
    method: &Method,
    instructions: &[Instruction],
    cfg: &ControlFlowGraph,
) -> Result<Summary> {
    let mut analysis = Analysis {
        program,
        joins: HashMap::default(),
    };
    let mut initial = Frame::default();
    let mut slot = if method.access_flags & 8 == 0 {
        initial.locals.insert(0, Value::reference(&class.name));
        1
    } else {
        0
    };
    for parameter in MethodDescriptor::parse(&method.descriptor)?.parameters {
        if parameter.is_reference() {
            initial.locals.insert(slot, Value::ty(&parameter));
        }
        slot += parameter.slots();
    }
    let mut entries: IdVec<BlockId, _> = vec![None; cfg.blocks.len()].into();
    entries[cfg.entry] = Some(initial);
    let mut pending = VecDeque::from([cfg.entry]);
    let mut queued: IdVec<BlockId, _> = vec![false; cfg.blocks.len()].into();
    queued[cfg.entry] = true;
    let mut throws = BTreeMap::<u32, Value>::new();
    let mut reflection = BTreeMap::<u32, Value>::new();
    while let Some(block) = pending.pop_front() {
        queued[block] = false;
        let mut frame = entries[block].as_ref().unwrap().clone();
        let basic = &cfg.blocks[block];
        for instruction in &instructions[basic.start..basic.end] {
            if instruction.can_throw() {
                for handler in active_handlers(program, &basic.handlers) {
                    let incoming = Frame {
                        locals: frame.locals.clone(),
                        stack: vec![Value::caught(
                            handler
                                .catch_type
                                .as_deref()
                                .unwrap_or("java/lang/Throwable"),
                            handler.target,
                        )],
                    };
                    if analysis.merge(&mut entries[handler.target], &incoming)?
                        && !queued[handler.target]
                    {
                        queued[handler.target] = true;
                        pending.push_back(handler.target);
                    }
                }
            }
            if instruction.opcode == 0xbf {
                let value = frame
                    .stack
                    .last()
                    .context("ATHROW has empty operand stack")?
                    .clone();
                if let Some(previous) = throws.get_mut(&instruction.pc) {
                    *previous = analysis.join(previous, &value);
                } else {
                    throws.insert(instruction.pc, value);
                }
            }
            if is_reflection(class, instruction) {
                let value = frame
                    .stack
                    .last()
                    .context("Class.newInstance has no receiver")?
                    .clone();
                if let Some(previous) = reflection.get_mut(&instruction.pc) {
                    *previous = analysis.join(previous, &value);
                } else {
                    reflection.insert(instruction.pc, value);
                }
            }
            transfer(program, class, instruction, &mut frame)
                .with_context(|| format!("reference frame at byte {}", instruction.pc))?;
        }
        for &successor in &basic.successors {
            if analysis.merge(&mut entries[successor], &frame)? && !queued[successor] {
                queued[successor] = true;
                pending.push_back(successor);
            }
        }
    }
    let throws = throws
        .into_iter()
        .filter_map(|(pc, value)| match value {
            Value::Null => None, // ATHROW null creates an unchecked NullPointerException.
            Value::Reference { ty, origins, .. } => Some((
                pc,
                ThrowValue {
                    source_type: if ty.as_ref() == "java/lang/Object" || ty.starts_with('[') {
                        "java/lang/Throwable".into()
                    } else {
                        ty.to_string()
                    },
                    origins,
                },
            )),
            _ => Some((
                pc,
                ThrowValue {
                    source_type: "java/lang/Throwable".into(),
                    origins: BTreeSet::from([Origin::Direct(Arc::from("java/lang/Throwable"))]),
                },
            )),
        })
        .collect();
    let reflection = reflection
        .into_iter()
        .map(|(pc, value)| {
            (
                pc,
                match value {
                    Value::Null => Some(BTreeSet::new()),
                    Value::Reference {
                        known: Some(Known::Classes(classes)),
                        ..
                    } => Some(classes),
                    _ => None,
                },
            )
        })
        .collect();
    Ok(Summary { throws, reflection })
}

pub(super) fn is_reflection(class: &ClassFile, instruction: &Instruction) -> bool {
    instruction.opcode == 0xb6
        && instruction
            .constant_index()
            .ok()
            .and_then(|index| class.constant_pool.member_ref(index).ok())
            .is_some_and(|member| {
                member.owner == "java/lang/Class"
                    && member.name == "newInstance"
                    && member.descriptor == "()Ljava/lang/Object;"
            })
}

impl Analysis<'_, '_> {
    fn merge(&mut self, destination: &mut Option<Frame>, source: &Frame) -> Result<bool> {
        let Some(destination) = destination else {
            *destination = Some(source.clone());
            return Ok(true);
        };
        ensure!(
            destination.stack.len() == source.stack.len(),
            "incompatible stack heights at reference merge"
        );
        let mut changed = false;
        for (left, right) in destination.stack.iter_mut().zip(&source.stack) {
            ensure!(
                left.width() == right.width(),
                "incompatible categories at reference merge"
            );
            let joined = self.join(left, right);
            if *left != joined {
                *left = joined;
                changed = true;
            }
        }
        destination.locals.retain(|slot, left| {
            let Some(right) = source.locals.get(slot) else {
                changed = true;
                return false;
            };
            let joined = self.join(left, right);
            if *left != joined {
                *left = joined;
                changed = true;
            }
            true
        });
        Ok(changed)
    }

    fn join(&mut self, left: &Value, right: &Value) -> Value {
        if left == right {
            return left.clone();
        }
        match (left, right) {
            (Value::Null, Value::Reference { .. }) => right.clone(),
            (Value::Reference { .. }, Value::Null) => left.clone(),
            (
                Value::Reference {
                    ty: left,
                    origins: left_origins,
                    known: left_known,
                },
                Value::Reference {
                    ty: right,
                    origins: right_origins,
                    known: right_known,
                },
            ) => {
                let key = if left < right {
                    (left.clone(), right.clone())
                } else {
                    (right.clone(), left.clone())
                };
                let joined = if let Some(joined) = self.joins.get(&key) {
                    joined.clone()
                } else {
                    let joined: Arc<str> = Arc::from(self.common_reference(left, right));
                    self.joins.insert(key, joined.clone());
                    joined
                };
                Value::Reference {
                    ty: joined,
                    origins: left_origins.union(right_origins).cloned().collect(),
                    known: left_known
                        .as_ref()
                        .zip(right_known.as_ref())
                        .and_then(|(a, b)| a.join(b)),
                }
            }
            _ => Value::UnknownReference,
        }
    }

    fn common_reference<'a>(&'a self, left: &'a str, right: &'a str) -> &'a str {
        if self.program.is_subtype(left, right) {
            return right;
        }
        if self.program.is_subtype(right, left) {
            return left;
        }
        let mut current = self.program.id(left);
        while let Some(class) = current {
            let Some(parent) = self.program.class(class).super_name.as_deref() else {
                break;
            };
            if self.program.is_subtype(right, parent) {
                return parent;
            }
            current = self.program.id(parent);
        }
        "java/lang/Object"
    }
}

fn pop(frame: &mut Frame) -> Result<Value> {
    stack::pop(&mut frame.stack)
}
fn pop_one(frame: &mut Frame) -> Result<Value> {
    stack::pop_one(&mut frame.stack, Value::width)
}

fn transfer(
    program: &Program<'_>,
    class: &ClassFile,
    instruction: &Instruction,
    frame: &mut Frame,
) -> Result<()> {
    let opcode = instruction.opcode;
    match opcode {
        0x00 => {}
        0x01 => frame.stack.push(Value::Null),
        0x02..=0x08 | 0x0b..=0x0d | 0x10..=0x11 => frame.stack.push(Value::One),
        0x09..=0x0a | 0x0e..=0x0f => frame.stack.push(Value::Two),
        0x12..=0x14 => {
            let value = match class.constant_pool.get(instruction.constant_index()?)? {
                Constant::Integer(_) | Constant::Float(_) => Value::One,
                Constant::Long(_) | Constant::Double(_) => Value::Two,
                Constant::String(index) => Value::known(
                    "java/lang/String",
                    Known::Strings(BTreeSet::from([Arc::from(
                        class.constant_pool.utf8(*index)?,
                    )])),
                ),
                Constant::Class(index) => Value::known(
                    "java/lang/Class",
                    Known::Classes(BTreeSet::from([Arc::from(
                        class.constant_pool.utf8(*index)?,
                    )])),
                ),
                _ => bail!("unsupported legacy LDC constant"),
            };
            frame.stack.push(value);
        }
        0x15 | 0x17 => frame.stack.push(Value::One),
        0x16 | 0x18 => frame.stack.push(Value::Two),
        0x19 => frame.stack.push(
            frame
                .locals
                .get(&instruction.local_index()?)
                .cloned()
                .unwrap_or(Value::UnknownReference),
        ),
        0x2e..=0x35 => {
            pop_one(frame)?;
            let array = pop_one(frame)?;
            let element = if opcode == 0x32 {
                match array {
                    Value::Reference { ty: name, .. } => name
                        .strip_prefix('[')
                        .and_then(|element| JvmType::parse(element).ok())
                        .map_or(Value::UnknownReference, |ty| Value::ty(&ty)),
                    _ => Value::UnknownReference,
                }
            } else if matches!(opcode, 0x2f | 0x31) {
                Value::Two
            } else {
                Value::One
            };
            frame.stack.push(element);
        }
        0x36..=0x3a => {
            let slot = instruction.local_index()?;
            let value = pop(frame)?;
            if opcode == 0x3a {
                frame.locals.insert(slot, value);
            } else {
                frame.locals.remove(&slot);
            }
            if matches!(opcode, 0x37 | 0x39)
                && let Some(next) = slot.checked_add(1)
            {
                frame.locals.remove(&next);
            }
        }
        0x4f..=0x56 => {
            pop(frame)?;
            pop_one(frame)?;
            pop_one(frame)?;
        }
        0x57..=0x5f => StackOperation::decode(opcode)
            .unwrap()
            .apply(&mut frame.stack, Value::width)?,
        0x60..=0x83 | 0x85..=0x98 => {
            let operation = NumericOperation::decode(opcode).unwrap();
            operation.operands(&mut frame.stack)?;
            frame.stack.push(Value::ty(&operation.result_type()));
        }
        0x84 => {
            if let Operand::Increment { local, .. } = instruction.operand {
                frame.locals.remove(&local);
            }
        }
        0x99..=0x9e | 0xc6..=0xc7 => {
            pop_one(frame)?;
        }
        0x9f..=0xa6 => {
            pop_one(frame)?;
            pop_one(frame)?;
        }
        0xa7 => {}
        0xaa..=0xab => {
            pop_one(frame)?;
        }
        0xac..=0xb0 => {
            pop(frame)?;
        }
        0xb1 => {}
        0xb2..=0xb5 => {
            let member = class
                .constant_pool
                .member_ref(instruction.constant_index()?)?;
            let ty = JvmType::parse(member.descriptor)?;
            if matches!(opcode, 0xb3 | 0xb5) {
                pop(frame)?;
            }
            if matches!(opcode, 0xb4 | 0xb5) {
                pop_one(frame)?;
            }
            if matches!(opcode, 0xb2 | 0xb4) {
                frame.stack.push(Value::ty(&ty));
            }
        }
        0xb6..=0xb9 => {
            let member = class
                .constant_pool
                .member_ref(instruction.constant_index()?)?;
            let signature = MethodDescriptor::parse(member.descriptor)?;
            let arguments =
                stack::arguments(&mut frame.stack, &signature.parameters, Value::width)?;
            let receiver = (opcode != 0xb8).then(|| pop_one(frame)).transpose()?;
            if signature.return_type != JvmType::Void {
                let known = if member.owner == "java/lang/Class"
                    && member.name == "forName"
                    && member.descriptor == "(Ljava/lang/String;)Ljava/lang/Class;"
                {
                    match arguments.first() {
                        Some(Value::Reference {
                            known: Some(Known::Strings(names)),
                            ..
                        }) => Some(Known::Classes(
                            names
                                .iter()
                                .map(|name| Arc::from(name.replace('.', "/")))
                                .collect(),
                        )),
                        _ => None,
                    }
                } else if member.name == "getClass" && member.descriptor == "()Ljava/lang/Class;" {
                    match receiver.as_ref() {
                        Some(Value::Reference {
                            known: Some(Known::Instances(classes)),
                            ..
                        }) => Some(Known::Classes(classes.clone())),
                        Some(Value::Reference { ty, .. }) => program.id(ty).and_then(|id| {
                            if program.class(id).access_flags & 0x0010 != 0 {
                                Some(Known::Classes(BTreeSet::from([ty.clone()])))
                            } else if program.is_own(id) {
                                // The same closed project hierarchy used for virtual
                                // call targets also bounds getClass on an owned base.
                                Some(Known::Classes(
                                    program.classes()[..program.own_count()]
                                        .iter()
                                        .filter(|class| program.is_subtype(&class.name, ty))
                                        .map(|class| Arc::from(class.name.as_str()))
                                        .collect(),
                                ))
                            } else {
                                None
                            }
                        }),
                        _ => None,
                    }
                } else {
                    None
                };
                let mut value = Value::ty(&signature.return_type);
                if let Value::Reference {
                    known: destination, ..
                } = &mut value
                {
                    *destination = known;
                }
                frame.stack.push(value);
            }
        }
        0xbb => {
            let name = class
                .constant_pool
                .class_name(instruction.constant_index()?)?;
            frame.stack.push(Value::known(
                name,
                Known::Instances(BTreeSet::from([Arc::from(name)])),
            ));
        }
        0xbc => {
            pop_one(frame)?;
            let ty = JvmType::Array(Box::new(instruction.newarray_type()?));
            frame.stack.push(Value::ty(&ty));
        }
        0xbd => {
            pop_one(frame)?;
            let name = class
                .constant_pool
                .class_name(instruction.constant_index()?)?;
            let descriptor = if name.starts_with('[') {
                format!("[{name}")
            } else {
                format!("[L{name};")
            };
            frame.stack.push(Value::reference(&descriptor));
        }
        0xbe => {
            pop_one(frame)?;
            frame.stack.push(Value::One);
        }
        0xbf | 0xc2..=0xc3 => {
            pop_one(frame)?;
        }
        0xc0 => {
            let value = pop_one(frame)?;
            let target = class
                .constant_pool
                .class_name(instruction.constant_index()?)?;
            let narrowed = match value {
                Value::Null => Value::Null,
                Value::Reference { origins, known, .. } => {
                    let origins = origins
                        .into_iter()
                        .filter_map(|origin| match origin {
                            Origin::Direct(name) => {
                                Some(Origin::Direct(if program.is_subtype(&name, target) {
                                    name
                                } else {
                                    Arc::from(target)
                                }))
                            }
                            Origin::Caught { block, limit: None } => Some(Origin::Caught {
                                block,
                                limit: Some(Arc::from(target)),
                            }),
                            Origin::Caught {
                                block,
                                limit: Some(limit),
                            } => {
                                if program.is_subtype(&limit, target) {
                                    Some(Origin::Caught {
                                        block,
                                        limit: Some(limit),
                                    })
                                } else if program.is_subtype(target, &limit) {
                                    Some(Origin::Caught {
                                        block,
                                        limit: Some(Arc::from(target)),
                                    })
                                } else {
                                    None
                                }
                            }
                        })
                        .collect();
                    Value::Reference {
                        ty: Arc::from(target),
                        origins,
                        known,
                    }
                }
                _ => Value::reference(target),
            };
            frame.stack.push(narrowed);
        }
        0xc1 => {
            pop_one(frame)?;
            frame.stack.push(Value::One);
        }
        0xc5 => {
            let Operand::MultiArray { index, dimensions } = instruction.operand else {
                bail!("missing multianewarray dimensions")
            };
            for _ in 0..dimensions {
                pop_one(frame)?;
            }
            frame
                .stack
                .push(Value::reference(class.constant_pool.class_name(index)?));
        }
        _ => bail!("unsupported legacy opcode {opcode:#x}"),
    }
    Ok(())
}
