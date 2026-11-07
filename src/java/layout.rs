//! Relative layouts assembled from known expression boundaries. No Java text is
//! reparsed: calls supply their arguments and choices supply their three arms.
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

fn continuation(text: &str) -> String {
    text.replace('\n', "\n    ")
}
fn indent(text: &str) -> String {
    format!("    {}", continuation(text))
}
