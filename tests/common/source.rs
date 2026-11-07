//! Small inspections of emitted Java, shared by source-shape tests.

/// Mask literals and comments without moving byte offsets or line boundaries.
fn code(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let start = cursor;
        match bytes[cursor] {
            b'\'' | b'"' => {
                let quote = bytes[cursor];
                cursor += 1;
                while cursor < bytes.len() {
                    let character = bytes[cursor];
                    cursor += 1;
                    if character == b'\\' {
                        cursor = (cursor + 1).min(bytes.len());
                    } else if character == quote {
                        break;
                    }
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                cursor += 2;
                while cursor + 1 < bytes.len() && &bytes[cursor..cursor + 2] != b"*/" {
                    cursor += 1;
                }
                cursor = (cursor + 2).min(bytes.len());
            }
            _ => {
                cursor += 1;
                continue;
            }
        }
        for byte in &mut bytes[start..cursor] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}

fn method_bounds(source: &str, name: &str) -> (usize, usize, usize) {
    let code = code(source);
    for (start, _) in code.match_indices(name) {
        let before = &code[..start];
        if !before.chars().next_back().is_some_and(char::is_whitespace)
            || before.split_whitespace().next_back() == Some("new")
        {
            continue;
        }
        let mut tail = code[start + name.len()..].trim_start();
        if !tail.starts_with('(') {
            continue;
        }
        // Emitted parameter lists contain types and names, never expressions.
        let Some(close) = tail.find(')') else {
            continue;
        };
        tail = tail[close + 1..].trim_start();
        if tail.starts_with("throws ") {
            tail = &tail[tail.find(['{', ';', '}']).unwrap_or(tail.len())..];
        }
        if !tail.starts_with('{') {
            continue;
        }
        let open = code.len() - tail.len();
        let mut depth = 0;
        for (index, character) in code[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return (start, open, open + index);
            }
        }
        panic!("unclosed method {name}:\n{source}");
    }
    panic!("missing method {name}:\n{source}");
}

/// Extract the first ordinary emitted method body with this name.
pub fn method<'a>(source: &'a str, name: &str) -> &'a str {
    let (_, open, close) = method_bounds(source, name);
    &source[open + 1..close]
}

/// Include the method name and parameters for tests of parameter bindings.
pub fn method_declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let (start, _, close) = method_bounds(source, name);
    &source[start..=close]
}

/// Count complete Java words, excluding comments and quoted literals.
pub fn word_count(source: &str, word: &str) -> usize {
    code(source)
        .split(|character: char| !character.is_alphanumeric() && !matches!(character, '_' | '$'))
        .filter(|token| *token == word)
        .count()
}

pub fn assert_no_word(source: &str, word: &str) {
    assert_eq!(word_count(source, word), 0, "unexpected {word}:\n{source}");
}

#[test]
fn method_inspection_skips_calls_comments_literals_and_nested_braces() {
    let source = r#"
class Example {
    String misleading = " int target() { wrong(); }";
    // int target() { wrong(); }
    /* int target() { wrong(); } */
    void caller() { target(); if (target()) { wrong(); } }
    int target () throws Exception {
        if (ready) { mark("}\\\""); }
        // }
        /* } */
        return '{';
    }
    void later() { wrong(); }
}
"#;
    let body = method(source, "target");
    assert!(body.contains("if (ready) { mark("));
    assert!(body.contains("return '{';"));
    assert!(!body.contains("wrong"));
    assert!(method_declaration(source, "target").starts_with("target () throws Exception"));
}

#[test]
fn words_do_not_match_identifier_fragments_or_quoted_text() {
    let source = "int print = hint; String text = \"int\"; /* int */ // int\n";
    assert_eq!(word_count(source, "int"), 1);
    assert_no_word("print(hint); println(\"int\");", "int");
    assert_no_word("boolean_value = true;", "boolean");
}
