use super::{Instruction, Operand};
use crate::Result;
use anyhow::{bail, ensure};

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    pc: u32,
}
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        ensure!(
            self.bytes.len() - self.position >= N,
            "truncated instruction at byte {}",
            self.pc
        );
        let result = self.bytes[self.position..self.position + N]
            .try_into()
            .unwrap();
        self.position += N;
        Ok(result)
    }
    fn u1(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }
    fn u2(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take()?))
    }
    fn i2(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take()?))
    }
    fn i4(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take()?))
    }
    fn target(&self, displacement: i32) -> Result<u32> {
        let target = i64::from(self.pc) + i64::from(displacement);
        ensure!(
            target >= 0 && target < self.bytes.len() as i64,
            "branch at {} targets outside code: {target}",
            self.pc
        );
        Ok(target as u32)
    }
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Instruction>> {
    ensure!(!bytes.is_empty(), "empty bytecode");
    ensure!(u32::try_from(bytes.len()).is_ok(), "bytecode too large");
    let mut input = Reader {
        bytes,
        position: 0,
        pc: 0,
    };
    let mut instructions = Vec::with_capacity(bytes.len() / 2 + 1);
    while input.position < bytes.len() {
        input.pc = input.position as u32;
        let mut opcode = input.u1()?;
        let operand = match opcode {
            0x00..=0x0f => Operand::None,
            0x10 => Operand::Int(i32::from(input.u1()? as i8)),
            0x11 => Operand::Int(i32::from(input.i2()?)),
            0x12 => Operand::Constant(u16::from(input.u1()?)),
            0x13 | 0x14 => Operand::Constant(input.u2()?),
            0x15..=0x19 | 0x36..=0x3a | 0xa9 => Operand::Local(u16::from(input.u1()?)),
            0x1a..=0x2d => {
                let index = opcode - 0x1a;
                opcode = 0x15 + index / 4;
                Operand::Local(u16::from(index % 4))
            }
            0x2e..=0x35 => Operand::None,
            0x3b..=0x4e => {
                let index = opcode - 0x3b;
                opcode = 0x36 + index / 4;
                Operand::Local(u16::from(index % 4))
            }
            0x4f..=0x83 => Operand::None,
            0x84 => Operand::Increment {
                local: u16::from(input.u1()?),
                amount: i16::from(input.u1()? as i8),
            },
            0x85..=0x98 => Operand::None,
            0x99..=0xa8 | 0xc6 | 0xc7 => {
                let displacement = input.i2()?;
                Operand::Branch(input.target(i32::from(displacement))?)
            }
            0xaa => {
                while !input.position.is_multiple_of(4) {
                    input.u1()?;
                }
                let displacement = input.i4()?;
                let default = input.target(displacement)?;
                let low = input.i4()?;
                let high = input.i4()?;
                let count = i64::from(high) - i64::from(low) + 1;
                ensure!(
                    count > 0 && count <= ((bytes.len() - input.position) / 4) as i64,
                    "invalid tableswitch range {low}..{high} at {}",
                    input.pc
                );
                let mut cases = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let displacement = input.i4()?;
                    cases.push(((i64::from(low) + index) as i32, input.target(displacement)?));
                }
                Operand::Switch { default, cases }
            }
            0xab => {
                while !input.position.is_multiple_of(4) {
                    input.u1()?;
                }
                let displacement = input.i4()?;
                let default = input.target(displacement)?;
                let count = input.i4()?;
                ensure!(
                    count >= 0 && count as usize <= (bytes.len() - input.position) / 8,
                    "invalid lookupswitch pair count {count} at {}",
                    input.pc
                );
                let mut cases = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let key = input.i4()?;
                    let displacement = input.i4()?;
                    cases.push((key, input.target(displacement)?));
                }
                // Preserve source order, but ambiguous duplicate keys are not valid JVM dispatch.
                ensure!(
                    cases.windows(2).all(|pair| pair[0].0 < pair[1].0),
                    "unsorted or duplicate lookupswitch keys at {}",
                    input.pc
                );
                Operand::Switch { default, cases }
            }
            0xac..=0xb1 => Operand::None,
            0xb2..=0xb8 | 0xbb | 0xbd | 0xc0 | 0xc1 => Operand::Constant(input.u2()?),
            0xb9 => {
                let index = input.u2()?;
                let count = input.u1()?;
                input.u1()?;
                Operand::InterfaceCall { index, count }
            }
            0xba => {
                let index = input.u2()?;
                input.u2()?;
                Operand::Constant(index)
            }
            0xbc => {
                let atype = input.u1()?;
                ensure!(
                    (4..=11).contains(&atype),
                    "invalid newarray type {atype} at {}",
                    input.pc
                );
                Operand::Int(i32::from(atype))
            }
            0xbe | 0xbf | 0xc2 | 0xc3 => Operand::None,
            0xc4 => {
                opcode = input.u1()?;
                match opcode {
                    0x15..=0x19 | 0x36..=0x3a | 0xa9 => Operand::Local(input.u2()?),
                    0x84 => Operand::Increment {
                        local: input.u2()?,
                        amount: input.i2()?,
                    },
                    _ => bail!(
                        "wide prefix before invalid opcode {opcode:02x} at {}",
                        input.pc
                    ),
                }
            }
            0xc5 => {
                let index = input.u2()?;
                let dimensions = input.u1()?;
                ensure!(
                    dimensions > 0,
                    "zero-dimensional multianewarray at {}",
                    input.pc
                );
                Operand::MultiArray { index, dimensions }
            }
            0xc8 | 0xc9 => {
                opcode = if opcode == 0xc8 { 0xa7 } else { 0xa8 };
                let displacement = input.i4()?;
                Operand::Branch(input.target(displacement)?)
            }
            _ => bail!("invalid JVM opcode 0x{opcode:02x} at byte {}", input.pc),
        };
        instructions.push(Instruction {
            pc: input.pc,
            opcode,
            operand,
        });
    }
    // Validate targets once, against a dense byte-address bitmap. All downstream
    // code can index instructions without repairing branches into operands.
    let mut boundaries = vec![false; bytes.len()];
    for instruction in &instructions {
        boundaries[instruction.pc as usize] = true;
    }
    for instruction in &instructions {
        let valid = |pc: u32| -> Result<()> {
            ensure!(
                boundaries[pc as usize],
                "branch at {} targets the middle of an instruction: {pc}",
                instruction.pc
            );
            Ok(())
        };
        match &instruction.operand {
            Operand::Branch(target) => valid(*target)?,
            Operand::Switch { default, cases } => {
                valid(*default)?;
                for (_, target) in cases {
                    valid(*target)?;
                }
            }
            _ => (),
        }
    }
    Ok(instructions)
}
