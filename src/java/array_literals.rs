//! Relative layout for array initializer expressions. The enclosing CodeWriter
//! supplies its indentation to every line; nested initializers indent themselves.
const LINE_WIDTH: usize = 120;

pub(super) fn layout(head: &str, elements: &[String], nested: bool) -> String {
    let contents =
        elements.iter().map(String::len).sum::<usize>() + elements.len().saturating_sub(1) * 2;
    if elements.iter().all(|element| !element.contains('\n'))
        && head.len() + contents + 3 <= LINE_WIDTH
    {
        return format!("{head} {} }}", elements.join(", "));
    }
    let mut result = String::with_capacity(head.len() + contents + elements.len() * 5 + 3);
    result.push_str(head);
    result.push('\n');
    if nested || elements.iter().any(|element| element.contains('\n')) {
        for (index, element) in elements.iter().enumerate() {
            let mut lines = element.split('\n').peekable();
            while let Some(line) = lines.next() {
                result.push_str("    ");
                result.push_str(line);
                if lines.peek().is_none() && index + 1 < elements.len() {
                    result.push(',');
                }
                result.push('\n');
            }
        }
    } else {
        let mut line = String::from("    ");
        for (index, element) in elements.iter().enumerate() {
            let comma = index + 1 < elements.len();
            if line.len() > 4 && line.len() + 1 + element.len() + usize::from(comma) > LINE_WIDTH {
                result.push_str(&line);
                result.push('\n');
                line.clear();
                line.push_str("    ");
            }
            if line.len() > 4 {
                line.push(' ');
            }
            line.push_str(element);
            if comma {
                line.push(',');
            }
        }
        result.push_str(&line);
        result.push('\n');
    }
    result.push('}');
    result
}
