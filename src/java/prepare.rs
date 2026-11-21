//! Required Java target lowering, before any structured or publication-site plan.
use super::{
    allocations, constructors, context::EmissionContext, field_joins, linkage::LinkagePlan,
    local_names,
};
use crate::{Result, program::MemberId, structure::PreparedIr};

pub(super) struct PreparedBody {
    pub ir: PreparedIr,
    pub helpers: allocations::AllocationHelpers,
}

impl PreparedBody {
    pub fn new(
        cx: &EmissionContext<'_, '_>,
        linkage: &LinkagePlan,
        member: MemberId,
        parameters: &[String],
    ) -> Result<Self> {
        let program = cx.program;
        let method = program.method(member);
        if let Some(diagnostic) = program.throws_plan().diagnostic(member) {
            anyhow::bail!("{diagnostic}");
        }
        let (mut ir, counts) = cx.method_ir(member)?;
        cx.record_simplifications(counts);
        if method.name == "<init>" {
            constructors::normalize_finals(cx, member, &mut ir)?;
        }
        linkage.lower(cx, member, &mut ir)?;
        let helpers = allocations::lower(cx, member, &mut ir)?;
        if method.name == "<init>" {
            field_joins::converge(program, member.class, &mut ir);
        }
        local_names::apply(&mut ir, parameters, cx.types);
        Ok(Self {
            ir: PreparedIr::new(ir),
            helpers,
        })
    }
}
