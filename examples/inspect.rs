//! Dump one method's representations while diagnosing a concrete reconstruction failure.
use anyhow::{Context, Result};
use sporeflower::{bytecode, cfg, input::Input, ir, structure};
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() >= 4,
        "usage: inspect JAR_OR_CLASS CLASS METHOD [DESCRIPTOR]"
    );
    let input = Input::read(Path::new(&args[1]))?;
    let class = input
        .classes()
        .iter()
        .find(|c| c.name == args[2])
        .context("class not found")?;
    let method = class
        .methods
        .iter()
        .find(|m| m.name == args[3] && args.get(4).is_none_or(|desc| *desc == m.descriptor))
        .context("method not found")?;
    let code = method.code.as_ref().context("no bytecode")?;
    let instructions = bytecode::decode(&code.bytes)?;
    let graph = cfg::ControlFlowGraph::build(&instructions, &code.exception_table)?;
    println!("BYTECODE\n{instructions:#?}\nCFG\n{graph:#?}");
    let values = ir::lift(class, method, &instructions, &graph)?;
    let prepared = structure::PreparedIr::new(values);
    println!(
        "IR\n{:#?}\nSTRUCTURED\n{:#?}",
        prepared.ir(),
        structure::structure(&prepared)?
    );
    Ok(())
}
