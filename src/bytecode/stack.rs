//! Physical operand-stack operations shared by abstract interpreters. Values are
//! opaque: a duplicate clones an identity or fact, never evaluates its producer.
use crate::{Result, classfile::JvmType};
use anyhow::{Context, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackOperation {
    Pop,
    Pop2,
    Dup,
    DupX1,
    DupX2,
    Dup2,
    Dup2X1,
    Dup2X2,
    Swap,
}

impl StackOperation {
    pub fn decode(opcode: u8) -> Option<Self> {
        Some(match opcode {
            0x57 => Self::Pop,
            0x58 => Self::Pop2,
            0x59 => Self::Dup,
            0x5a => Self::DupX1,
            0x5b => Self::DupX2,
            0x5c => Self::Dup2,
            0x5d => Self::Dup2X1,
            0x5e => Self::Dup2X2,
            0x5f => Self::Swap,
            _ => return None,
        })
    }

    pub fn apply<V: Clone>(self, stack: &mut Vec<V>, category: impl Fn(&V) -> u16) -> Result<()> {
        let (top_words, lower_words) = match self {
            Self::Pop | Self::Dup => (1, 0),
            Self::Pop2 | Self::Dup2 => (2, 0),
            Self::DupX1 | Self::Swap => (1, 1),
            Self::DupX2 => (1, 2),
            Self::Dup2X1 => (2, 1),
            Self::Dup2X2 => (2, 2),
        };
        let end = stack.len();
        let top = group_start(stack, end, top_words, &category)?;
        let lower = group_start(stack, top, lower_words, &category)?;
        match self {
            Self::Pop | Self::Pop2 => stack.truncate(top),
            Self::Swap => stack.swap(end - 1, end - 2),
            _ => {
                // [lower, top] -> [top, lower, top]. The original top values
                // remain aliases; only the stack positions are copied.
                stack.extend_from_within(top..end);
                stack[lower..end].rotate_right(end - top);
            }
        }
        Ok(())
    }
}

fn group_start<V>(
    stack: &[V],
    mut end: usize,
    words: u16,
    category: &impl Fn(&V) -> u16,
) -> Result<usize> {
    let mut width = 0;
    while width < words {
        end = end.checked_sub(1).context("operand stack underflow")?;
        let slots = category(&stack[end]);
        ensure!(matches!(slots, 1 | 2), "invalid operand stack category");
        width += slots;
    }
    ensure!(
        width == words,
        "stack permutation splits a category-two value"
    );
    Ok(end)
}

pub(crate) fn pop<V>(stack: &mut Vec<V>) -> Result<V> {
    stack.pop().context("operand stack underflow")
}

pub(crate) fn pop_one<V>(stack: &mut Vec<V>, category: impl Fn(&V) -> u16) -> Result<V> {
    let value = pop(stack)?;
    ensure!(category(&value) == 1, "expected category-one operand");
    Ok(value)
}

/// Pop JVM arguments right-to-left, returning declaration/source order. A
/// category-two argument is one value even though it occupies two stack words.
pub(crate) fn arguments<V>(
    stack: &mut Vec<V>,
    parameters: &[JvmType],
    category: impl Fn(&V) -> u16,
) -> Result<Vec<V>> {
    let mut arguments = Vec::with_capacity(parameters.len());
    for parameter in parameters.iter().rev() {
        let value = pop(stack)?;
        ensure!(
            category(&value) == parameter.slots(),
            "argument category mismatch"
        );
        arguments.push(value);
    }
    arguments.reverse();
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values 1..=4 are category one; 5..=8 are category two. Distinct numbers
    // make value identity and source order observable independently of width.
    fn category(value: &u8) -> u16 {
        if *value < 5 { 1 } else { 2 }
    }

    #[test]
    fn every_legal_stack_permutation_preserves_identity_and_order() {
        for (opcode, before, after) in [
            (0x57, vec![1], vec![]),
            (0x58, vec![1, 2], vec![]),
            (0x58, vec![5], vec![]),
            (0x59, vec![1], vec![1, 1]),
            (0x5a, vec![1, 2], vec![2, 1, 2]),
            (0x5b, vec![1, 2, 3], vec![3, 1, 2, 3]),
            (0x5b, vec![5, 1], vec![1, 5, 1]),
            (0x5c, vec![1, 2], vec![1, 2, 1, 2]),
            (0x5c, vec![5], vec![5, 5]),
            (0x5d, vec![1, 2, 3], vec![2, 3, 1, 2, 3]),
            (0x5d, vec![1, 5], vec![5, 1, 5]),
            (0x5e, vec![1, 2, 3, 4], vec![3, 4, 1, 2, 3, 4]),
            (0x5e, vec![1, 2, 5], vec![5, 1, 2, 5]),
            (0x5e, vec![5, 1, 2], vec![1, 2, 5, 1, 2]),
            (0x5e, vec![5, 6], vec![6, 5, 6]),
            (0x5f, vec![1, 2], vec![2, 1]),
        ] {
            let mut stack = vec![8]; // Every operation must preserve its prefix.
            stack.extend(before);
            StackOperation::decode(opcode)
                .unwrap()
                .apply(&mut stack, category)
                .unwrap();
            assert_eq!(stack, [vec![8], after].concat(), "opcode {opcode:02x}");
        }
    }

    #[test]
    fn stack_permutations_reject_underflow_and_split_categories() {
        for opcode in 0x57..=0x5f {
            let operation = StackOperation::decode(opcode).unwrap();
            assert!(operation.apply(&mut Vec::<u8>::new(), category).is_err());
        }
        for (opcode, mut stack) in [
            (0x57, vec![5]),
            (0x58, vec![5, 1]),
            (0x59, vec![5]),
            (0x5a, vec![5, 1]),
            (0x5b, vec![5, 1, 2]),
            (0x5c, vec![5, 1]),
            (0x5d, vec![5, 6]),
            (0x5e, vec![5, 1, 6]),
            (0x5f, vec![5, 1]),
        ] {
            assert!(
                StackOperation::decode(opcode)
                    .unwrap()
                    .apply(&mut stack, category)
                    .is_err(),
                "opcode {opcode:02x}"
            );
        }
    }

    #[test]
    fn invocation_arguments_keep_mixed_width_declaration_order() {
        let mut stack = vec![4, 1, 5, 2, 6];
        let parameters = [JvmType::Int, JvmType::Long, JvmType::Int, JvmType::Double];
        assert_eq!(
            arguments(&mut stack, &parameters, category).unwrap(),
            [1, 5, 2, 6]
        );
        assert_eq!(stack, [4]);
        assert!(arguments(&mut vec![1], &[JvmType::Long], category).is_err());
    }
}
