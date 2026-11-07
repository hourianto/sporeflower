//! Inspect only allocation boundaries requiring program-wide source planning.
use sporeflower::{
    api::{ApiCatalog, ApiOptions},
    input::Input,
    naming::{NamePlan, RenameOptions},
    program::Program,
};
use std::collections::BTreeSet;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let catalog = ApiCatalog::bundled()?;
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let check = arguments.iter().any(|argument| argument == "--check");
    for path in arguments
        .into_iter()
        .filter(|argument| argument != "--check")
    {
        let input = Input::read(Path::new(&path))?;
        let selected = catalog.select(input.classes(), &ApiOptions::default())?;
        let program = Program::from_input(&input, &selected.classes)?;
        let plan = program.initialization_plan();
        println!("{path}: {:?}", plan.statistics);
        let mut external: Vec<_> = plan.external_sites().collect();
        external.sort_by_key(|(member, site)| (member.class, member.index, site.pc));
        let owners: BTreeSet<_> = external.iter().map(|(member, _)| member.class).collect();
        for (member, site) in external {
            let method = program.method(member);
            println!(
                "  external {}.{}{} pc {} -> {}",
                program.class(member.class).name,
                method.name,
                method.descriptor,
                site.pc,
                site.class
            );
        }
        if check && !owners.is_empty() {
            let names = NamePlan::build(&program, &RenameOptions::default())?;
            for owner in owners {
                let rendered = sporeflower::java::decompile_class(
                    &program,
                    &names,
                    owner,
                    &Default::default(),
                )?;
                for diagnostic in rendered.diagnostics {
                    if diagnostic.contains("allocation") || diagnostic.contains("external NEW") {
                        println!("  unresolved {diagnostic}");
                    }
                }
            }
        }
    }
    Ok(())
}
