//! Inspect a lifted method without executing input bytecode.
use sporeflower::{Result, input::Input, ir, structure};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: inspect_flow JAR CLASS METHOD [DESCRIPTOR]"
    );
    let input = Input::read(std::path::Path::new(&args[0]))?;
    let class = input
        .classes()
        .iter()
        .find(|class| class.name == args[1])
        .ok_or_else(|| anyhow::anyhow!("class not found"))?;
    for method in class.methods.iter().filter(|method| {
        method.name == args[2]
            && args
                .get(3)
                .is_none_or(|descriptor| method.descriptor == *descriptor)
    }) {
        let raw = ir::decompile_method(class, method)?;
        let prepared = structure::PreparedIr::new(raw.clone());
        let ir = if std::env::var_os("SPOREFLOWER_INSPECT_PREPARE").is_some() {
            prepared.ir()
        } else {
            &raw
        };
        println!(
            "{} {}{} entry {}",
            class.name, method.name, method.descriptor, ir.entry
        );
        if std::env::var_os("SPOREFLOWER_INSPECT_VALUES").is_some() {
            for (id, value) in ir.values.iter_enumerated() {
                println!(
                    "V{id} pc={} block={} local={:?} {:?}",
                    value.pc, value.block, value.local, value.kind
                );
            }
        }
        if std::env::var_os("SPOREFLOWER_INSPECT_GRAPH").is_some() {
            println!("SOURCE FLOW {:#?}", structure::FlowGraph::from(ir));
        }
        for (id, block) in ir.blocks.iter_enumerated() {
            println!(
                "B{id} pc={} {:?} handlers={:?}",
                block.pc, block.terminator, block.handlers
            );
            for (index, statement) in block.statements.iter().enumerate() {
                println!("  {index}: {statement:?}");
            }
        }
        let structured = structure::structure(&prepared)?;
        if std::env::var_os("SPOREFLOWER_INSPECT_RECOVERY").is_some() {
            println!("RECOVERY (final region graph)");
            if structured.recovery_attempts.is_empty() {
                println!("  recovered without a rejected strategy");
            }
            for attempt in &structured.recovery_attempts {
                println!("  {attempt}");
            }
        }
        println!("{structured:#?}");
    }
    Ok(())
}
