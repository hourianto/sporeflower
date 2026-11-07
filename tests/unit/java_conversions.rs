use super::*;
use crate::index::test_ids::*;
use crate::ir::{CompareOp, Condition, Local, Value};

#[test]
fn conditional_types_use_java_constant_representability_not_jvm_int_category() {
    for (ty, min, max) in [
        (JvmType::Byte, -128, 127),
        (JvmType::Short, -32768, 32767),
        (JvmType::Char, 0, 65535),
    ] {
        for constant in [min - 1, min, max, max + 1] {
            let expected = if min <= constant && constant <= max {
                &ty
            } else {
                &JvmType::Int
            };
            assert_eq!(
                conditional_numeric_type(&ty, None, &JvmType::Int, Some(constant)),
                Some(expected)
            );
            assert_eq!(
                conditional_numeric_type(&JvmType::Int, Some(constant), &ty, None),
                Some(expected)
            );
        }
        assert_eq!(
            conditional_numeric_type(&ty, None, &JvmType::Int, None),
            Some(&JvmType::Int)
        );
    }
    assert_eq!(
        conditional_numeric_type(&JvmType::Byte, None, &JvmType::Short, None),
        Some(&JvmType::Short)
    );
    assert_eq!(
        conditional_numeric_type(&JvmType::Byte, None, &JvmType::Char, None),
        Some(&JvmType::Int)
    );
    assert_eq!(
        conditional_numeric_type(&JvmType::Short, None, &JvmType::Char, None),
        Some(&JvmType::Int)
    );
}

#[test]
fn constant_choices_follow_inline_aliases_but_never_mutable_storage_or_assignments() {
    let kinds = [
        ValueKind::Literal(Literal::Int(128)),
        ValueKind::Cast {
            value: V0,
            ty: JvmType::Byte,
        },
        ValueKind::Alias(V1),
        ValueKind::Literal(Literal::Int(1)),
        ValueKind::Binary {
            op: BinaryOp::Add,
            left: V2,
            right: V3,
        },
        ValueKind::InlineAssign {
            local: L0,
            value: V4,
        },
        ValueKind::Select {
            condition: Condition {
                op: CompareOp::Gt,
                left: V0,
                right: V3,
            },
            then_value: V4,
            else_value: V3,
        },
    ];
    let mut ir = MethodIr {
        entry: B0,
        blocks: vec![].into(),
        values: kinds
            .into_iter()
            .map(|kind| Value {
                local: None,
                kind,
                ty: JvmType::Int,
                pc: 0,
                block: B0,
            })
            .collect(),
        locals: vec![Local {
            name_hint: None,
            name: "value".into(),
            ty: JvmType::Byte,
            parameter: None,
            is_this: false,
            needs_default: false,
        }]
        .into(),
        parameters: vec![],
        return_type: JvmType::Int,
        initialization_sites: vec![],
    };
    let program = Program::new(&[], &[]).unwrap();
    assert_eq!(int_constant(&program, &ir, V4), IntConstant::Known(-127));
    assert_eq!(int_constant(&program, &ir, V6), IntConstant::Known(-127));
    assert_eq!(int_constant(&program, &ir, V5), IntConstant::NotConstant);
    ir.values[V1].local = Some(L0);
    assert_eq!(int_constant(&program, &ir, V2), IntConstant::NotConstant);
    assert_eq!(int_constant(&program, &ir, V6), IntConstant::NotConstant);
}

#[test]
fn unresolved_constantness_preserves_possible_narrow_types_across_nested_choices() {
    let uncertain = conditional_source_type(
        SourceType::value(&JvmType::Byte),
        IntConstant::NotConstant,
        SourceType::value(&JvmType::Int),
        IntConstant::Unknown,
    )
    .unwrap();
    assert!(!uncertain.exactly(&JvmType::Int));
    assert_eq!(
        uncertain.types().collect::<Vec<_>>(),
        vec![&JvmType::Int, &JvmType::Byte]
    );
    let nested = conditional_source_type(
        uncertain,
        IntConstant::Unknown,
        SourceType::value(&JvmType::Char),
        IntConstant::NotConstant,
    )
    .unwrap();
    assert_eq!(
        nested.types().collect::<Vec<_>>(),
        vec![&JvmType::Int, &JvmType::Char]
    );
    let mutable = conditional_source_type(
        SourceType::value(&JvmType::Byte),
        IntConstant::NotConstant,
        SourceType::value(&JvmType::Int),
        IntConstant::NotConstant,
    )
    .unwrap();
    assert!(mutable.exactly(&JvmType::Int));
}

#[test]
fn numeric_constants_keep_java_integer_wrap_and_float_narrowing() {
    let int = |number: Option<Number>| match number.unwrap() {
        Number::Int(value) => value,
        _ => panic!("expected int"),
    };
    assert_eq!(
        int(Number::Int(i32::MAX).binary(BinaryOp::Add, Number::Int(1), &JvmType::Int)),
        i32::MIN
    );
    assert_eq!(
        int(Number::Int(i32::MIN).binary(BinaryOp::Div, Number::Int(-1), &JvmType::Int)),
        i32::MIN
    );
    assert!(
        Number::Int(1)
            .binary(BinaryOp::Div, Number::Int(0), &JvmType::Int)
            .is_none()
    );
    assert_eq!(int(Number::Double(128.0).cast(&JvmType::Byte)), -128);
    assert_eq!(
        int(Number::Float(f32::INFINITY).cast(&JvmType::Char)),
        65535
    );
    assert_eq!(int(Number::Double(f64::NAN).cast(&JvmType::Int)), 0);
}
