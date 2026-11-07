use super::{JvmType, MethodDescriptor};
use crate::Result;
use anyhow::{bail, ensure};

impl JvmType {
    /// Parse one field descriptor (or `V`, useful for method return types).
    pub fn parse(descriptor: &str) -> Result<Self> {
        let mut position = 0;
        let ty = parse_type(descriptor.as_bytes(), &mut position, true)?;
        ensure!(
            position == descriptor.len(),
            "trailing data in type descriptor {descriptor:?}"
        );
        Ok(ty)
    }

    pub fn descriptor(&self) -> String {
        match self {
            Self::Void => "V".into(),
            Self::Boolean => "Z".into(),
            Self::Byte => "B".into(),
            Self::Char => "C".into(),
            Self::Short => "S".into(),
            Self::Int => "I".into(),
            Self::Long => "J".into(),
            Self::Float => "F".into(),
            Self::Double => "D".into(),
            Self::Object(name) => format!("L{name};"),
            Self::Array(element) => format!("[{}", element.descriptor()),
        }
    }

    pub fn slots(&self) -> u16 {
        match self {
            Self::Void => 0,
            Self::Long | Self::Double => 2,
            _ => 1,
        }
    }
    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Object(_) | Self::Array(_))
    }
    pub fn is_int_like(&self) -> bool {
        matches!(
            self,
            Self::Boolean | Self::Byte | Self::Char | Self::Short | Self::Int
        )
    }
    pub fn array_dimensions(&self) -> usize {
        match self {
            Self::Array(element) => 1 + element.array_dimensions(),
            _ => 0,
        }
    }
}

impl MethodDescriptor {
    pub fn parse(descriptor: &str) -> Result<Self> {
        let bytes = descriptor.as_bytes();
        ensure!(
            bytes.first() == Some(&b'('),
            "invalid method descriptor {descriptor:?}"
        );
        let mut position = 1;
        let mut parameters = Vec::new();
        let mut parameter_slots = 0usize;
        while bytes.get(position) != Some(&b')') {
            let parameter = parse_type(bytes, &mut position, false)?;
            parameter_slots += usize::from(parameter.slots());
            ensure!(
                parameter_slots <= 255,
                "method descriptor exceeds 255 parameter slots"
            );
            parameters.push(parameter);
        }
        position += 1;
        let return_type = parse_type(bytes, &mut position, true)?;
        ensure!(
            position == bytes.len(),
            "trailing data in method descriptor {descriptor:?}"
        );
        Ok(Self {
            parameters,
            return_type,
        })
    }

    pub fn descriptor(&self) -> String {
        let mut result = String::from("(");
        for parameter in &self.parameters {
            result.push_str(&parameter.descriptor());
        }
        result.push(')');
        result.push_str(&self.return_type.descriptor());
        result
    }

    pub fn parameter_slots(&self) -> u16 {
        self.parameters.iter().map(JvmType::slots).sum()
    }
}

fn parse_type(bytes: &[u8], position: &mut usize, allow_void: bool) -> Result<JvmType> {
    let Some(&tag) = bytes.get(*position) else {
        bail!("truncated type descriptor at {}", *position)
    };
    *position += 1;
    Ok(match tag {
        b'V' if allow_void => JvmType::Void,
        b'Z' => JvmType::Boolean,
        b'B' => JvmType::Byte,
        b'C' => JvmType::Char,
        b'S' => JvmType::Short,
        b'I' => JvmType::Int,
        b'J' => JvmType::Long,
        b'F' => JvmType::Float,
        b'D' => JvmType::Double,
        b'L' => {
            let start = *position;
            let Some(length) = bytes[start..].iter().position(|&b| b == b';') else {
                bail!("unterminated object type in descriptor")
            };
            ensure!(length > 0, "empty object type in descriptor");
            let name = std::str::from_utf8(&bytes[start..start + length])?;
            ensure!(
                !name.bytes().any(|b| matches!(b, b'.' | b'[')),
                "invalid object type {name:?}"
            );
            *position += length + 1;
            JvmType::Object(name.to_owned())
        }
        b'[' => {
            // Count the prefix iteratively; hostile input cannot exhaust the native stack.
            let mut dimensions = 1;
            while bytes.get(*position) == Some(&b'[') {
                dimensions += 1;
                *position += 1;
            }
            ensure!(
                dimensions <= 255,
                "array descriptor has more than 255 dimensions"
            );
            let mut ty = parse_type(bytes, position, false)?;
            for _ in 0..dimensions {
                ty = JvmType::Array(Box::new(ty));
            }
            ty
        }
        _ => bail!("invalid descriptor tag 0x{tag:02x} at {}", *position - 1),
    })
}
