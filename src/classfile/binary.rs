//! Lossless class-file primitives shared by parsing, native edits and stub output.
//! Pool entries retain original bytes; only explicitly redirected edges change.
use crate::Result;
use anyhow::{bail, ensure};
use std::{cell::Cell, collections::HashMap};

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            base: 0,
        }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
    pub fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        ensure!(
            length <= self.remaining(),
            "truncated class file at byte {}: need {length}, have {}",
            self.base + self.position,
            self.remaining()
        );
        let result = &self.bytes[self.position..self.position + length];
        self.position += length;
        Ok(result)
    }
    pub fn u1(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u2(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u4(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u8(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn sub(&mut self, length: usize) -> Result<Reader<'a>> {
        let base = self.base + self.position;
        Ok(Reader {
            bytes: self.take(length)?,
            position: 0,
            base,
        })
    }
    /// Decode constant payloads without copying their original representation.
    /// Both semantic parsing and lossless rewriting use this framing authority.
    pub fn pool<T: Default>(
        &mut self,
        mut decode: impl FnMut(u16, u8, &[u8]) -> Result<T>,
    ) -> Result<Vec<T>> {
        let count = self.u2()? as usize;
        ensure!(count > 0, "constant_pool_count must be positive");
        let mut entries = Vec::with_capacity(count);
        entries.push(T::default());
        while entries.len() < count {
            let index = entries.len() as u16;
            let tag = self.u1()?;
            let length = match tag {
                1 => self.u2()? as usize,
                3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
                5 | 6 => 8,
                7 | 8 | 16 | 19 | 20 => 2,
                15 => 3,
                _ => bail!("unknown constant-pool tag {tag} at index {index}"),
            };
            entries.push(decode(index, tag, self.take(length)?)?);
            if matches!(tag, 5 | 6) {
                ensure!(
                    entries.len() < count,
                    "two-slot constant at final constant-pool index"
                );
                entries.push(T::default());
            }
        }
        Ok(entries)
    }

    pub fn end(&self) -> Result<()> {
        ensure!(
            self.remaining() == 0,
            "{} trailing bytes at class-file offset {}",
            self.remaining(),
            self.base + self.position
        );
        Ok(())
    }
}

pub(crate) struct Entry {
    pub tag: u8,
    pub data: Vec<u8>,
}

pub(crate) struct Pool {
    pub entries: Vec<Option<Entry>>,
    // Zero invalidates a representative after an edit; resolve duplicates lazily.
    indices: HashMap<u8, HashMap<Vec<u8>, Cell<u16>>>,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            entries: vec![None],
            indices: HashMap::new(),
        }
    }
}

impl Pool {
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn read(input: &mut Reader<'_>) -> Result<Self> {
        let mut pool = Self::default();
        pool.entries = input.pool(|index, tag, data| {
            pool.indices
                .entry(tag)
                .or_default()
                .entry(data.to_vec())
                .or_insert(Cell::new(index));
            Ok(Some(Entry {
                tag,
                data: data.to_vec(),
            }))
        })?;
        Ok(pool)
    }

    pub fn find(&self, tag: u8, data: &[u8]) -> Option<u16> {
        let cached = self.indices.get(&tag)?.get(data)?;
        if cached.get() == 0 {
            let index = self.entries.iter().position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.tag == tag && entry.data == data)
            })? as u16;
            cached.set(index);
        }
        Some(cached.get())
    }

    pub fn append(&mut self, tag: u8, data: Vec<u8>) -> Result<u16> {
        let wide = matches!(tag, 5 | 6);
        ensure!(
            self.entries.len() + usize::from(wide) < u16::MAX as usize,
            "constant pool exceeds JVM limit"
        );
        let index = self.entries.len() as u16;
        self.indices
            .entry(tag)
            .or_default()
            .entry(data.clone())
            .and_modify(|cached| {
                if cached.get() == 0 {
                    cached.set(index);
                }
            })
            .or_insert(Cell::new(index));
        self.entries.push(Some(Entry { tag, data }));
        if wide {
            self.entries.push(None);
        }
        Ok(index)
    }

    pub fn insert(&mut self, tag: u8, data: Vec<u8>) -> Result<u16> {
        match self.find(tag, &data) {
            Some(index) => Ok(index),
            None => self.append(tag, data),
        }
    }

    /// Update the interning index together with an explicitly relocated edge.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn replace(&mut self, index: u16, data: Vec<u8>) {
        let entry = self.entries[index as usize].as_mut().unwrap();
        let indices = self.indices.get_mut(&entry.tag).unwrap();
        if let Some(cached) = indices.get(&entry.data)
            && cached.get() == index
        {
            cached.set(0);
        }
        let cached = indices.entry(data.clone()).or_insert(Cell::new(index));
        if cached.get() == 0 {
            cached.set(index);
        }
        entry.data = data;
    }

    pub fn text(&mut self, text: &str) -> Result<u16> {
        let bytes = modified_utf8(text);
        ensure!(
            u16::try_from(bytes.len()).is_ok(),
            "UTF8 constant exceeds JVM length limit"
        );
        self.insert(1, bytes)
    }

    pub fn pair(&mut self, name: &str, descriptor: &str) -> Result<u16> {
        let name = self.text(name)?;
        let descriptor = self.text(descriptor)?;
        self.pair_indices(name, descriptor)
    }

    pub fn pair_indices(&mut self, name: u16, descriptor: u16) -> Result<u16> {
        self.insert(
            12,
            [&name.to_be_bytes()[..], &descriptor.to_be_bytes()].concat(),
        )
    }

    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn string(&mut self, text: &str) -> Result<u16> {
        let text = self.text(text)?;
        self.insert(8, text.to_be_bytes().to_vec())
    }

    pub fn class(&mut self, name: &str) -> Result<u16> {
        let name = self.text(name)?;
        self.insert(7, name.to_be_bytes().to_vec())
    }

    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn method(&mut self, owner: u16, name: &str, descriptor: &str) -> Result<u16> {
        let pair = self.pair(name, descriptor)?;
        self.insert(10, [&owner.to_be_bytes()[..], &pair.to_be_bytes()].concat())
    }

    pub fn write(&self, output: &mut Vec<u8>) {
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

pub(crate) fn modified_utf8(text: &str) -> Vec<u8> {
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

#[cfg(test)]
#[path = "../../tests/unit/classfile_pool.rs"]
mod tests;
