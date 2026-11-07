//! Inspect checked-exception inference without rendering or executing app code.
use anyhow::{Context, Result};
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    bytecode::{self, Operand},
    input::Input,
    program::{MemberId, Program},
};
use std::path::Path;

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("usage: throws_scan JAR")?;
    let input = Input::read(Path::new(&path))?;
    let api = ApiCatalog::bundled()?.select(input.classes(), &ApiOptions::default())?;
    let program = Program::from_input(&input, &api.classes)?;
    let plan = program.throws_plan();
    let arguments: Vec<_> = std::env::args().collect();
    if let [_, _, owner, name, descriptor] = arguments.as_slice() {
        let member = program
            .resolve_method(owner, name, descriptor)
            .context("method not found")?;
        let ir =
            sporeflower::ir::decompile_method(program.class(member.class), program.method(member))?;
        let prepared = sporeflower::structure::PreparedIr::new(ir);
        println!(
            "IR\n{:#?}\nSTRUCTURED\n{:#?}",
            prepared.ir(),
            sporeflower::structure::structure(&prepared)?
        );
        return Ok(());
    }
    let mut errors = 0;
    for (class_id, class) in input.classes().iter().enumerate() {
        for (index, method) in class.methods.iter().enumerate() {
            let member = MemberId {
                class: class_id,
                index,
            };
            if let Some(diagnostic) = plan.diagnostic(member) {
                errors += 1;
                println!(
                    "DIAGNOSTIC {}.{}{}: {diagnostic}",
                    class.name, method.name, method.descriptor
                );
            }
            let Some(code) = &method.code else { continue };
            for instruction in bytecode::decode(&code.bytes)? {
                if instruction.opcode == 0xbf
                    && let Some(fact) = plan.throw_fact(member, instruction.pc)
                    && fact
                        .checked
                        .iter()
                        .any(|name| name == "java/lang/Throwable")
                {
                    println!(
                        "RETHROW {}.{}{} @{}",
                        class.name, method.name, method.descriptor, instruction.pc
                    );
                }
                if instruction.opcode != 0xb6 {
                    continue;
                }
                let Operand::Constant(index) = instruction.operand else {
                    continue;
                };
                let called = class.constant_pool.member_ref(index)?;
                if called.owner != "java/lang/Class" || called.name != "newInstance" {
                    continue;
                }
                if let Some(facts) = plan.reflection_at(member, instruction.pc) {
                    println!(
                        "REFLECTION {}.{}{} @{} checked={:?} unknown={}",
                        class.name,
                        method.name,
                        method.descriptor,
                        instruction.pc,
                        facts.checked,
                        facts.unknown
                    );
                }
            }
        }
    }
    println!("{errors} checked-exception diagnostics");
    Ok(())
}
