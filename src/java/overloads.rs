//! Descriptor casts are needed only when Java would select another overload.
//! J2ME has widening conversions, but no boxing, varargs or generic inference.
use crate::classfile::{JvmType, MethodDescriptor};
use crate::hash::HashSet;
use crate::{
    naming::NamePlan,
    program::{ClassId, MemberId, Program},
};

pub(super) type Argument<'a> = super::conversions::SourceType<'a>;

pub(super) struct Site<'a> {
    pub owners: &'a [&'a str],
    pub name: &'a str,
    pub constructor: bool,
}

pub(super) fn casts(
    program: &Program<'_>,
    names: &NamePlan,
    caller: ClassId,
    site: &Site<'_>,
    parameters: &[JvmType],
    arguments: &[Argument<'_>],
) -> Vec<bool> {
    let exact = || {
        arguments
            .iter()
            .zip(parameters)
            .map(|(arg, expected)| !arg.exactly(expected))
            .collect()
    };
    if arguments
        .iter()
        .zip(parameters)
        .all(|(arg, expected)| arg.exactly(expected))
    {
        return vec![false; parameters.len()];
    }
    if arguments
        .iter()
        .any(|argument| !argument.null && !complete_type(program, argument.ty))
    {
        return exact();
    }
    if arguments.iter().zip(parameters).any(|(argument, target)| {
        if argument.null {
            !target.is_reference()
        } else {
            !argument.types().all(|ty| converts(program, ty, target))
        }
    }) {
        return exact();
    }
    let mut candidates = Vec::new();
    let mut visited = HashSet::default();
    for owner in site.owners {
        let Some(owner) = program.id(owner) else {
            return exact();
        };
        for class in std::iter::once(owner).chain(program.ancestors(owner).iter().copied()) {
            if site.constructor && class != owner {
                continue;
            }
            if !visited.insert(class) {
                continue;
            }
            let definition = program.class(class);
            if !site.constructor
                && definition
                    .super_name
                    .iter()
                    .chain(&definition.interfaces)
                    .any(|parent| program.id(parent).is_none())
            {
                // Missing hierarchy metadata cannot prove an inherited overload
                // absent. Retain descriptor types until that dependency exists.
                return exact();
            }
            for (index, method) in definition.methods.iter().enumerate() {
                if (method.name == "<init>") != site.constructor
                    || names.method_name(MemberId { class, index }) != site.name
                    || !program.visible_from(method.access_flags, class, caller)
                {
                    continue;
                }
                let Ok(descriptor) = MethodDescriptor::parse(&method.descriptor) else {
                    return exact();
                };
                if descriptor.parameters.len() == parameters.len()
                    && descriptor.parameters != parameters
                {
                    candidates.push(descriptor.parameters);
                }
            }
        }
    }
    let mut pinned = vec![false; parameters.len()];
    for candidate in candidates {
        let applicable = candidate.iter().enumerate().all(|(index, target)| {
            if pinned[index] {
                converts(program, &parameters[index], target)
            } else {
                accepts(program, &arguments[index], target)
            }
        });
        if !applicable {
            continue;
        }
        // The desired signature already wins against a less-specific overload.
        // Otherwise one descriptor cast suffices to reject this competitor.
        if let Some(index) = parameters
            .iter()
            .zip(&candidate)
            .position(|(desired, competing)| !converts(program, desired, competing))
        {
            pinned[index] = true;
        }
    }
    pinned
}

fn accepts(program: &Program<'_>, argument: &Argument<'_>, target: &JvmType) -> bool {
    argument.null && target.is_reference()
        || !argument.null && argument.types().any(|ty| converts(program, ty, target))
}

fn converts(program: &Program<'_>, from: &JvmType, to: &JvmType) -> bool {
    if from.is_reference() || to.is_reference() {
        program.assignable(from, to)
    } else {
        super::conversions::widening(from, to)
    }
}

fn complete_type(program: &Program<'_>, ty: &JvmType) -> bool {
    match ty {
        JvmType::Array(element) => complete_type(program, element),
        JvmType::Object(name) if name != "java/lang/Object" => {
            let Some(owner) = program.id(name) else {
                return false;
            };
            std::iter::once(owner)
                .chain(program.ancestors(owner).iter().copied())
                .all(|class| {
                    let class = program.class(class);
                    class
                        .super_name
                        .iter()
                        .chain(&class.interfaces)
                        .all(|name| program.id(name).is_some())
                })
        }
        _ => true,
    }
}
