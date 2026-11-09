//! Relative layouts assembled from known expression boundaries. No Java text is
//! reparsed: calls supply their arguments, choices supply their three arms, and
//! logical expressions supply their short-circuit operands in evaluation order.
//! CodeWriter adds the statement's indentation to every resulting line.
const WIDTH: usize = 88;

pub(super) fn arguments(arguments: Vec<String>) -> String {
    let flat = arguments.join(", ");
    if flat.len() <= WIDTH && !flat.contains('\n') {
        flat
    } else {
        format!(
            "\n{}\n",
            arguments
                .iter()
                .map(|argument| indent(argument))
                .collect::<Vec<_>>()
                .join(",\n")
        )
    }
}

pub(super) fn conditional(condition: String, yes: String, no: String) -> String {
    let flat = format!("{condition} ? {yes} : {no}");
    if flat.len() <= WIDTH && !flat.contains('\n') {
        flat
    } else {
        format!(
            "{condition}\n    ? {}\n    : {}",
            continuation(&yes),
            continuation(&no)
        )
    }
}

pub(super) fn logical(parts: Vec<String>, and: bool) -> String {
    let operator = if and { "&&" } else { "||" };
    let flat = parts.join(&format!(" {operator} "));
    if (flat.len() <= WIDTH && !flat.contains('\n')) || parts.len() < 2 {
        return flat;
    }
    let mut parts = parts.into_iter();
    let mut result = parts.next().unwrap_or_default();
    for part in parts {
        result.push_str("\n        ");
        result.push_str(operator);
        result.push(' ');
        result.push_str(&part.replace('\n', "\n        "));
    }
    result
}

fn continuation(text: &str) -> String {
    text.replace('\n', "\n    ")
}
fn indent(text: &str) -> String {
    format!("    {}", continuation(text))
}
