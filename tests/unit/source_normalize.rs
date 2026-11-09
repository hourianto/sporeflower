use super::*;

#[test]
fn null_substitution_removes_inner_cast_but_keeps_the_consumer_type() {
    let ty = JvmType::Array(Box::new(JvmType::Int));
    let mut expression =
        Expr::cast("int[]", Expr::cast("Object", Expr::atom("saved"))).with_type(Some(&ty));
    // Local substitution changes an already constructed expression. It does
    // not pass the new operand back through Expr::cast.
    let Expression::Cast { value: outer, .. } = &mut expression.kind else {
        panic!("outer cast")
    };
    let Expression::Cast { value: inner, .. } = &mut outer.kind else {
        panic!("inner cast")
    };
    **inner = Expr::null();
    normalize_conversions(&mut expression);
    assert_eq!(expression.render(), "(int[]) null");
    assert_eq!(expression.source_type(), Some(&ty));
    // An argument keeps its exact cast, while an assignment/return already
    // supplies the type and can omit the remaining null cast.
    assigned_conversion(&mut expression, &ty);
    assert_eq!(expression.render(), "null");

    let mut nonnull = Expr::cast("int[]", Expr::cast("Object", Expr::atom("saved")));
    normalize_conversions(&mut nonnull);
    assert_eq!(nonnull.render(), "(int[]) (Object) saved");
}
