use super::{Expr, Operator, Prefix};

fn atom(name: &str) -> Expr {
    Expr::atom(name)
}

#[test]
fn binary_grouping_preserves_right_operand_evaluation() {
    for (operator, token) in [
        (Operator::Subtract, "-"),
        (Operator::Divide, "/"),
        (Operator::Remainder, "%"),
        (Operator::ShiftLeft, "<<"),
        (Operator::ShiftRight, ">>"),
        (Operator::UnsignedShiftRight, ">>>"),
        // Addition and multiplication are not associative for floating point.
        (Operator::Add, "+"),
        (Operator::Multiply, "*"),
    ] {
        let right = Expr::binary(operator, atom("b"), atom("c"));
        let expression = Expr::binary(operator, atom("a"), right);
        assert_eq!(expression.render(), format!("a {token} (b {token} c)"));
        let left = Expr::binary(operator, atom("a"), atom("b"));
        let expression = Expr::binary(operator, left, atom("c"));
        assert_eq!(expression.render(), format!("a {token} b {token} c"));
    }
    let sum = Expr::binary(Operator::Add, atom("a"), atom("b"));
    assert_eq!(
        Expr::binary(Operator::Multiply, sum, atom("c")).render(),
        "(a + b) * c"
    );
}

#[test]
fn bitwise_and_logical_grouping_exposes_mixed_operator_boundaries() {
    let low = Expr::binary(Operator::And, atom("a"), atom("MASK"));
    let high = Expr::binary(
        Operator::And,
        Expr::binary(Operator::ShiftLeft, atom("b"), atom("8")),
        atom("HIGH"),
    );
    assert_eq!(
        Expr::binary(Operator::Or, low, high).render(),
        "(a & MASK) | ((b << 8) & HIGH)"
    );
    let count = Expr::binary(Operator::Add, atom("n"), atom("4"));
    assert_eq!(
        Expr::binary(Operator::ShiftLeft, atom("1"), count).render(),
        "1 << (n + 4)"
    );
    let right = Expr::logical(true, vec![atom("b"), atom("c")]);
    assert_eq!(
        Expr::logical(false, vec![atom("a"), right]).render(),
        "a || (b && c)"
    );
    let first = "firstConditionWithALongDescriptiveName";
    let second = "secondConditionWithAnotherDescriptiveName";
    assert_eq!(
        Expr::logical(true, vec![atom(first), atom(second), atom("lastCondition")]).render(),
        format!("{first}\n        && {second}\n        && lastCondition")
    );
    // A uniform chain stays flat, rather than grouping every binary node.
    let left = Expr::binary(Operator::And, atom("a"), atom("b"));
    assert_eq!(
        Expr::binary(Operator::And, left, atom("c")).render(),
        "a & b & c"
    );
}

#[test]
fn bitwise_literals_keep_their_java_width_and_other_contexts_keep_decimal() {
    use crate::classfile::JvmType;
    for (value, ty, decimal, hexadecimal) in [
        (255, JvmType::Int, "255", "0xff"),
        (-16777216, JvmType::Int, "-16777216", "0xff000000"),
        (4294967295, JvmType::Long, "4294967295L", "0xffffffffL"),
        (
            -16777216,
            JvmType::Long,
            "-16777216L",
            "0xffffffffff000000L",
        ),
    ] {
        let literal = Expr::integer(value, ty, decimal);
        assert_eq!(
            Expr::binary(Operator::And, atom("value"), literal.clone()).render(),
            format!("value & {hexadecimal}")
        );
        assert_eq!(
            Expr::assignment(atom("value"), literal.clone(), Some(Operator::Or)).render(),
            format!("value |= {hexadecimal}")
        );
        assert_eq!(
            Expr::binary(Operator::Add, atom("value"), literal).render(),
            format!("value + {decimal}")
        );
    }
}

#[test]
fn unary_minus_does_not_form_a_decrement_token() {
    for literal in ["-1", "-2147483648", "-9223372036854775808L", "-0.0F"] {
        assert_eq!(
            Expr::prefix(Prefix::Negate, atom(literal)).render(),
            format!("-({literal})")
        );
    }
    let negated = Expr::prefix(Prefix::Negate, atom("value"));
    assert_eq!(Expr::prefix(Prefix::Negate, negated).render(), "-(-value)");
    let decremented = Expr::update(atom("value"), false, false);
    assert_eq!(
        Expr::prefix(Prefix::Negate, decremented).render(),
        "-(--value)"
    );
    let decremented = Expr::update(atom("value"), false, true);
    assert_eq!(
        Expr::prefix(Prefix::Negate, decremented).render(),
        "-value--"
    );
}

#[test]
fn cast_grammar_distinguishes_primitive_and_reference_types() {
    for (value, primitive, reference) in [
        ("-1", "(int) -1", "(Owner) (-1)"),
        ("+1", "(int) +1", "(Owner) (+1)"),
    ] {
        assert_eq!(Expr::cast("int", atom(value)).render(), primitive);
        assert_eq!(Expr::cast("Owner", atom(value)).render(), reference);
    }
    assert_eq!(
        Expr::cast("int", Expr::prefix(Prefix::Negate, atom("n"))).render(),
        "(int) -n"
    );
    assert_eq!(
        Expr::cast("Owner", Expr::prefix(Prefix::Negate, atom("n"))).render(),
        "(Owner) (-n)"
    );
    for (increment, token) in [(true, "++"), (false, "--")] {
        assert_eq!(
            Expr::cast("int", Expr::update(atom("n"), increment, false)).render(),
            format!("(int) {token}n")
        );
        assert_eq!(
            Expr::cast("Owner", Expr::update(atom("n"), increment, false)).render(),
            format!("(Owner) ({token}n)")
        );
    }
    let choice = Expr::conditional(atom("flag"), atom("a"), atom("b"));
    assert_eq!(Expr::cast("int", choice).render(), "(int) (flag ? a : b)");
    let assignment = Expr::assignment(atom("a"), atom("b"), None);
    assert_eq!(Expr::cast("int", assignment).render(), "(int) (a = b)");
    let cast = Expr::cast("java.lang.Object", atom("value"));
    assert_eq!(
        Expr::cast("Owner", cast).render(),
        "(Owner) (java.lang.Object) value"
    );
    let cast_null = Expr::cast("java.lang.Object", Expr::null());
    assert_eq!(Expr::cast("Owner", cast_null).render(), "(Owner) null");
}

#[test]
fn choices_and_assignments_follow_java_operand_grammar() {
    let condition = Expr::conditional(atom("a"), atom("b"), atom("c"));
    let yes = Expr::assignment(atom("x"), atom("one"), None);
    let no = Expr::assignment(atom("x"), atom("two"), None);
    assert_eq!(
        Expr::conditional(condition, yes, no).render(),
        "(a ? b : c) ? x = one : (x = two)"
    );
    let no = Expr::conditional(atom("b"), atom("x"), atom("y"));
    assert_eq!(
        Expr::conditional(atom("a"), atom("z"), no).render(),
        "a ? z : b ? x : y"
    );
    let right = Expr::assignment(atom("b"), atom("c"), None);
    assert_eq!(
        Expr::assignment(atom("a"), right, None).render(),
        "a = b = c"
    );
}

#[test]
fn choice_polarity_keeps_numeric_truth_tests_and_flat_alternatives() {
    assert_eq!(
        Expr::conditional(
            Expr::prefix(Prefix::Not, atom("second")),
            atom("1"),
            atom("0"),
        )
        .render(),
        "second ? 0 : 1"
    );
    assert_eq!(
        Expr::conditional(
            Expr::binary(Operator::NotEqual, atom("value"), atom("0")),
            atom("1"),
            atom("0"),
        )
        .render(),
        "value != 0 ? 1 : 0"
    );
    assert_eq!(
        Expr::conditional(
            Expr::prefix(Prefix::Not, atom("first")),
            atom("1"),
            Expr::conditional(atom("second"), atom("2"), atom("3")),
        )
        .render(),
        "!first ? 1 : second ? 2 : 3"
    );
}

#[test]
fn access_and_invocation_group_their_receivers() {
    let call = Expr::cast("Owner", atom("value"))
        .member("method")
        .invoke(vec![atom("a"), atom("b")]);
    assert_eq!(call.render(), "((Owner) value).method(a, b)");
    let array = Expr::new_array("int".into(), vec![atom("size")], 0);
    assert_eq!(
        array.index(atom("index")).render(),
        "(new int[size])[index]"
    );
    let receiver = Expr::conditional(atom("flag"), atom("a"), atom("b"));
    assert_eq!(receiver.member("field").render(), "(flag ? a : b).field");
    let target = Expr::cast("int[]", atom("value")).index(atom("index"));
    assert_eq!(
        Expr::update(target, true, true).render(),
        "((int[]) value)[index]++"
    );
}

#[test]
fn logical_and_concatenation_contexts_preserve_operand_boundaries() {
    let right = Expr::logical(false, vec![atom("b"), atom("c")]);
    assert_eq!(
        Expr::logical(true, vec![atom("a"), right]).render(),
        "a && (b || c)"
    );
    let right = Expr::binary(Operator::Add, atom("a"), atom("b"));
    assert_eq!(
        Expr::concatenation(vec![atom("\"\""), right]).render(),
        "\"\" + (a + b)"
    );
    let comparison = Expr::binary(Operator::Less, atom("a"), atom("b"));
    assert_eq!(Expr::prefix(Prefix::Not, comparison).render(), "!(a < b)");

    let comparison = Expr::binary(
        Operator::Equal,
        atom("read").invoke(vec![atom("Float.NaN")]),
        atom("read").invoke(vec![atom("0.0F")]),
    );
    assert_eq!(
        Expr::prefix(Prefix::Not, comparison).render(),
        "read(Float.NaN) != read(0.0F)"
    );
}

#[test]
fn canonical_negation_keeps_raw_boolean_value_boundaries() {
    let raw = atom("raw").with_type(Some(&crate::classfile::JvmType::Boolean));
    let normalized = Expr::prefix(Prefix::Not, Expr::prefix(Prefix::Not, raw.clone()));
    assert_eq!(normalized.render(), "!!raw");
    assert!(normalized.canonical_boolean());
    assert_eq!(
        Expr::prefix(Prefix::Not, raw.clone())
            .negated_test()
            .render(),
        "raw"
    );
    let exact = Expr::binary(Operator::Equal, raw, Expr::boolean(true));
    assert_eq!(exact.negated_test().render(), "raw != true");
}

#[test]
fn canonical_negation_distributes_tests_without_inverting_float_ordering() {
    use crate::classfile::JvmType;

    for ty in [JvmType::Float, JvmType::Double] {
        let comparison = Expr::binary(
            Operator::Less,
            atom("value").with_type(Some(&ty)),
            atom("limit").with_type(Some(&ty)),
        );
        let condition = Expr::logical(
            false,
            vec![comparison, Expr::prefix(Prefix::Not, atom("ready"))],
        );
        assert_eq!(
            condition.negated_test().render(),
            "!(value < limit) && ready"
        );
    }
    let comparison = Expr::binary(
        Operator::Less,
        atom("value").with_type(Some(&JvmType::Int)),
        atom("limit").with_type(Some(&JvmType::Int)),
    );
    let condition = Expr::logical(
        false,
        vec![comparison, Expr::prefix(Prefix::Not, atom("ready"))],
    );
    assert_eq!(condition.negated_test().render(), "value >= limit && ready");
}
