//! Extract the prefix into a static helper and move its live results through
//! a private constructor. The original graph becomes the post-delegation body.
use super::*;

pub(super) fn render(
    cx: &EmissionContext<'_, '_>,
    mut post: MethodIr,
    member: MemberId,
    call: Delegation,
    default_fields: &[Expr],
) -> Result<super::super::RenderedMethod> {
    let program = cx.program;
    let types = cx.types;
    let mut pre = post.clone();
    pre.blocks[call.block].statements.truncate(call.statement);
    pre.blocks[call.block].terminator = Terminator::Return(None);
    pre.blocks[call.block].handlers.clear();
    let pre_reachable = prune(&mut pre);
    for &block in &pre_reachable {
        for statement in &pre.blocks[block].statements {
            for local in statement_reads(&pre, statement) {
                ensure!(
                    !pre.locals[local].is_this,
                    "preinitialization accesses uninitialized this; cannot prove delegation leaves that state unobserved"
                );
            }
        }
        for value in pre.blocks[block].terminator.operands() {
            for local in value_reads(&pre, ValueUse::Reference(value)) {
                ensure!(
                    !pre.locals[local].is_this,
                    "preinitialization branch accesses uninitialized this"
                );
            }
        }
        if block != call.block {
            ensure!(
                !matches!(pre.blocks[block].terminator, Terminator::Return(_)),
                "constructor returns before initializing its receiver"
            );
        }
    }

    post.entry = call.block;
    post.blocks[call.block].statements.drain(..=call.statement);
    let post_reachable = prune(&mut post);
    ensure!(
        pre_reachable
            .intersection(&post_reachable)
            .all(|&block| block == call.block),
        "constructor control flow crosses its initialization boundary more than once"
    );
    let captures: Vec<LocalId> = live_at_entry(&post)?
        .into_iter()
        .filter(|&local| !post.locals[local].is_this)
        .collect();
    for &argument in &call.args {
        ensure!(
            !value_reads(&pre, ValueUse::Reference(argument))
                .iter()
                .any(|&local| pre.locals[local].is_this),
            "constructor argument accesses uninitialized this"
        );
    }
    let signature = MethodDescriptor::parse(&program.method(member).descriptor)?;
    let target_signature = MethodDescriptor::parse(&call.target.descriptor)?;
    ensure!(
        target_signature.parameters.len() == call.args.len(),
        "constructor argument count mismatch"
    );
    let carrier = Carrier::new(
        helper_name(program, types, member.index, "$Preinit"),
        target_signature.parameters,
        &post,
        captures.iter().copied(),
    );
    let carrier_name = &carrier.name;
    let prepare = helper_name(program, types, member.index, "$preinit");
    let carrier_type = carrier.ty();

    // Carrier reads are plain field accesses on a fresh non-null object. The
    // static helper computes the original argument expressions in JVM order.
    let state_local = pre.locals.next_id();
    pre.locals.push(Local {
        name_hint: None,
        name: "$state".into(),
        ty: carrier_type.clone(),
        parameter: None,
        is_this: false,
        needs_default: false,
    });
    let pc = pre.blocks[call.block].pc;
    let state_value = carrier.allocate(&mut pre, call.block, pc, Some(state_local));
    pre.blocks[call.block].statements.push(Statement::Assign {
        local: state_local,
        value: crate::ir::ValueUse::Definition(state_value),
    });
    for (index, &value) in call.args.iter().enumerate() {
        pre.blocks[call.block].statements.push(carrier.store(
            state_value,
            Slot::Argument(index),
            value,
        ));
    }
    for &local in &captures {
        let value = pre
            .local_reference(local)
            .context("live constructor local has no value identity")?;
        pre.blocks[call.block].statements.push(carrier.store(
            state_value,
            Slot::Local(local),
            value,
        ));
    }
    pre.blocks[call.block].terminator = Terminator::Return(Some(state_value));
    pre.return_type = carrier_type;

    // Original parameters become ordinary locals in the private constructor;
    // only parameters used after delegation are transported and declared.
    let names = parameter_names(&post, &signature);
    let parameters = signature
        .parameters
        .iter()
        .zip(&names)
        .map(|(ty, name)| format!("{} {name}", types.ty(ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = names.join(", ");
    for local in &mut post.locals {
        if !local.is_this {
            local.parameter = None;
        }
    }
    post.parameters.truncate(1); // only the original receiver remains a parameter
    let post_state = carrier.parameter(&mut post, 0, 1, call.block, pc);
    let mut restore = Vec::new();
    for &local in &captures {
        let pc = post.blocks[call.block].pc;
        let value = carrier.load(&mut post, post_state, Slot::Local(local), call.block, pc);
        restore.push(Statement::Assign {
            local,
            value: crate::ir::ValueUse::Reference(value),
        });
    }
    restore.append(&mut post.blocks[call.block].statements);
    post.blocks[call.block].statements = restore;

    let mut declarations = CodeWriter::new(1);
    let throws = crate::java::generated::throws_clause(program.method_throws(member), types);

    declarations.open(format!(
        "private static {carrier_name} {prepare}({parameters}){throws} {{"
    ));
    let pre_body = cx.render(
        &PreparedIr::new(pre),
        Some(member),
        BodyOptions::original(BodyKind::Helper),
    )?;
    declarations.text.push_str(&pre_body.text);
    declarations.close("}");
    declarations.line("");

    let (markers, marker_values) = bridge_markers(program, types)?;
    let simple = types.simple_name();
    declarations.open(format!(
        "private {simple}({carrier_name} $state{markers}){throws} {{"
    ));
    let delegates = call.target.owner == program.class(types.owner).name;
    let pc = call.pc;
    let args = (0..call.args.len())
        .map(|index| carrier.load(&mut post, post_state, Slot::Argument(index), call.block, pc))
        .collect();
    let invocation = Statement::ConstructorCall {
        method: call.target,
        receiver: call.receiver,
        args,
        pc,
    };
    // The carrier's parameter indexes have no incoming facts from the original descriptor.
    let post_options = BodyOptions::captured(BodyKind::Constructor(ConstructorEntry {
        invocation: Some(&invocation),
        crossed: Vec::new(),
        delegates,
        defaults: default_fields,
    }));
    let post_body = cx.render(&PreparedIr::new(post), Some(member), post_options)?;
    declarations.text.push_str(&post_body.text);
    declarations.close("}");
    declarations.line("");
    carrier.emit(types, &mut declarations);

    let mut body = CodeWriter::new(2);
    body.line(format!("this({prepare}({arguments}){marker_values});"));
    Ok(super::super::RenderedMethod {
        body: body.text,
        declarations: declarations.text,
        fallback: pre_body.fallback || post_body.fallback,
        initializers: Vec::new(),
    })
}

fn parameter_names(ir: &MethodIr, signature: &MethodDescriptor) -> Vec<String> {
    let mut slot = 1;
    signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let name = ir
                .locals
                .iter()
                .find(|local| local.parameter == Some(index))
                .map_or_else(|| format!("p{slot}"), |local| local.name.clone());
            slot += ty.slots();
            name
        })
        .collect()
}
