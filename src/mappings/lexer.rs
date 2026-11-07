use super::{MappingDiagnostic, MappingOrigin};
use crate::Result;
use anyhow::{bail, ensure};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Word(String),
    Number(String),
    String(String),
    Char(char),
    Was(String),
    Symbol(char),
    End,
}
impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Word(value) | Self::Number(value) => write!(f, "'{value}'"),
            Self::String(_) => f.write_str("string literal"),
            Self::Char(_) => f.write_str("character literal"),
            Self::Was(_) => f.write_str("was comment"),
            Self::Symbol(ch) => write!(f, "'{ch}'"),
            Self::End => f.write_str("end of file"),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub kind: Kind,
    pub line: usize,
    pub column: usize,
    pub width: usize,
}

pub(crate) fn lex(source: &MappingOrigin) -> Result<Vec<Token>> {
    let chars: Vec<char> = source.source.chars().collect();
    let (mut i, mut line, mut column) = (0, 1, 1);
    let mut tokens = Vec::new();
    while i < chars.len() {
        let start = (line, column);
        let begin_token = i;
        let error =
            |message: String| MappingDiagnostic::new(message, source.at(start.0, start.1, 1));
        let ch = chars[i];
        if ch.is_whitespace() {
            advance(ch, &mut i, &mut line, &mut column);
            continue;
        }
        let kind = if ch == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                advance(chars[i], &mut i, &mut line, &mut column);
            }
            continue;
        } else if ch == '/' && chars.get(i + 1) == Some(&'*') {
            advance('/', &mut i, &mut line, &mut column);
            advance('*', &mut i, &mut line, &mut column);
            let begin = i;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                advance(chars[i], &mut i, &mut line, &mut column);
            }
            ensure!(
                i + 1 < chars.len(),
                error("unterminated comment".to_string())
            );
            let comment: String = chars[begin..i].iter().collect();
            advance('*', &mut i, &mut line, &mut column);
            advance('/', &mut i, &mut line, &mut column);
            let comment = comment.trim();
            if let Some(original) = comment
                .strip_prefix("was")
                .filter(|rest| rest.starts_with(char::is_whitespace))
            {
                let original = original.trim();
                ensure!(
                    !original.is_empty() && !original.chars().any(char::is_whitespace),
                    error("invalid was identity".to_string())
                );
                Kind::Was(original.into())
            } else {
                continue;
            }
        } else if ch == '"' || ch == '\'' {
            advance(ch, &mut i, &mut line, &mut column);
            let mut value = String::new();
            while i < chars.len() && chars[i] != ch {
                ensure!(
                    chars[i] != '\n' && chars[i] != '\r',
                    error("newline in literal".to_string())
                );
                let value_ch = if chars[i] == '\\' {
                    advance('\\', &mut i, &mut line, &mut column);
                    ensure!(i < chars.len(), error("unterminated escape".to_string()));
                    let esc = chars[i];
                    advance(esc, &mut i, &mut line, &mut column);
                    match esc {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        '\\' => '\\',
                        '\'' => '\'',
                        '"' => '"',
                        'u' => {
                            while chars.get(i) == Some(&'u') {
                                advance('u', &mut i, &mut line, &mut column);
                            }
                            let mut code = 0;
                            for _ in 0..4 {
                                let digit = chars.get(i).and_then(|c| c.to_digit(16));
                                ensure!(
                                    digit.is_some(),
                                    error("invalid Unicode escape".to_string())
                                );
                                code = code * 16 + digit.unwrap();
                                advance(chars[i], &mut i, &mut line, &mut column);
                            }
                            char::from_u32(code).ok_or_else(|| {
                                anyhow::anyhow!(error(
                                    "isolated surrogate in mapping literal".to_string()
                                ))
                            })?
                        }
                        '0'..='7' => {
                            let mut code = esc.to_digit(8).unwrap();
                            for _ in 0..if esc <= '3' { 2 } else { 1 } {
                                let Some(digit) = chars.get(i).and_then(|c| c.to_digit(8)) else {
                                    break;
                                };
                                code = code * 8 + digit;
                                advance(chars[i], &mut i, &mut line, &mut column);
                            }
                            char::from_u32(code).unwrap()
                        }
                        _ => bail!(error(format!("unsupported escape \\{esc}"))),
                    }
                } else {
                    let value_ch = chars[i];
                    advance(value_ch, &mut i, &mut line, &mut column);
                    value_ch
                };
                value.push(value_ch);
            }
            ensure!(
                chars.get(i) == Some(&ch),
                error("unterminated literal".to_string())
            );
            advance(ch, &mut i, &mut line, &mut column);
            if ch == '\'' {
                ensure!(
                    value.chars().count() == 1 && value.chars().next().unwrap() as u32 <= 0xffff,
                    error("char literal must contain one UTF-16 character".to_string())
                );
                Kind::Char(value.chars().next().unwrap())
            } else {
                Kind::String(value)
            }
        } else if ch.is_ascii_digit() {
            let begin = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                advance(chars[i], &mut i, &mut line, &mut column);
            }
            Kind::Number(chars[begin..i].iter().collect())
        } else if ch == '_' || ch == '$' || ch.is_alphabetic() || !ch.is_ascii() {
            let begin = i;
            while i < chars.len()
                && (chars[i] == '_'
                    || chars[i] == '$'
                    || chars[i].is_alphanumeric()
                    || !chars[i].is_ascii())
            {
                advance(chars[i], &mut i, &mut line, &mut column);
            }
            Kind::Word(chars[begin..i].iter().collect())
        } else {
            ensure!(
                "@.{}[]();,=*+-".contains(ch),
                error(format!("unexpected character {ch:?}"))
            );
            advance(ch, &mut i, &mut line, &mut column);
            Kind::Symbol(ch)
        };
        tokens.push(Token {
            kind,
            line: start.0,
            column: start.1,
            width: if line == start.0 { i - begin_token } else { 1 },
        });
    }
    tokens.push(Token {
        kind: Kind::End,
        line,
        column,
        width: 1,
    });
    Ok(tokens)
}

fn advance(ch: char, i: &mut usize, line: &mut usize, column: &mut usize) {
    *i += 1;
    if ch == '\n' {
        *line += 1;
        *column = 1;
    } else {
        *column += 1;
    }
}
