use super::{
    MappingDiagnostic, MappingDiagnostics, MappingOrigin, MappingSource,
    lexer::{self, Kind, Token},
};
use crate::Result;
use anyhow::ensure;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) enum Value {
    Integer(i64),
    String(String),
    Bool(bool),
    Class(String),
    Array(Vec<Value>),
}
#[derive(Clone, Debug)]
pub(crate) struct Annotation {
    pub origin: MappingOrigin,
    pub name: String,
    pub args: BTreeMap<String, Value>,
}
#[derive(Clone, Debug)]
pub(crate) struct Type {
    pub origin: MappingOrigin,
    pub name: String,
    pub dimensions: usize,
}
#[derive(Clone, Debug)]
pub(crate) struct Parameter {
    pub origin: MappingOrigin,
    pub name: String,
    pub ty: Type,
    pub annotations: Vec<Annotation>,
}
#[derive(Clone, Debug)]
pub(crate) struct Member {
    pub name: String,
    pub original: Option<String>,
    pub ty: Type,
    pub parameters: Option<Vec<Parameter>>,
    pub annotations: Vec<Annotation>,
    pub initializer: Option<Value>,
    pub origin: MappingOrigin,
}
#[derive(Clone, Debug)]
pub(crate) struct Declaration {
    pub name: String,
    pub original: Option<String>,
    pub interface: bool,
    pub annotations: Vec<Annotation>,
    pub members: Vec<Member>,
    pub origin: MappingOrigin,
}
#[derive(Clone, Debug)]
pub(crate) struct ParsedSource {
    pub builtin: bool,
    pub package: String,
    pub imports: Vec<String>,
    pub declarations: Vec<Declaration>,
}

impl ParsedSource {
    pub(super) fn explicit_import(&self, name: &str) -> Option<&str> {
        self.imports
            .iter()
            .find(|import| import.rsplit('.').next() == Some(name))
            .map(String::as_str)
    }
}

pub(crate) fn parse(source: &MappingSource) -> Result<ParsedSource> {
    let origin = MappingOrigin {
        path: source.path.as_str().into(),
        source: source.source.as_str().into(),
        line: 1,
        column: 1,
        width: 1,
    };
    let mut p = Parser {
        tokens: lexer::lex(&origin)?,
        origin,
        at: 0,
    };
    let package = if p.eat_word("package") {
        let name = p.qualified()?;
        p.expect(';')?;
        name
    } else {
        String::new()
    };
    let mut imports = Vec::new();
    let mut declarations = Vec::new();
    let mut errors = MappingDiagnostics::default();
    while p.token().kind != Kind::End {
        if p.eat(';') {
            continue;
        }
        // A map is a declaration DSL, not a compilation unit. Imports remain
        // file-scoped even when written between domain marker declarations.
        if p.eat_word("import") {
            imports.push(p.import()?);
            continue;
        }
        let origin = p.origin();
        let annotations = p.attributes()?;
        let interface = if p.eat_word("interface") {
            true
        } else {
            p.expect_word("class")?;
            false
        };
        let name = p.word()?;
        let mut original = p.was();
        // Header inheritance is documentary: all actual hierarchy facts come from bytecode.
        if p.eat_word("extends") || p.eat_word("implements") {
            p.qualified()?;
            while p.eat(',') || p.eat_word("implements") {
                p.qualified()?;
            }
        }
        if let Some(owner) = p.was() {
            ensure!(
                original.replace(owner).is_none(),
                p.error("duplicate was owner")
            );
        }
        p.expect('{')?;
        let mut members = Vec::new();
        while !p.eat('}') {
            if p.token().kind == Kind::End {
                errors.push(p.error("expected '}' to close class declaration"));
                break;
            }
            if !p.eat(';') {
                let start = p.at;
                match p.member(&name) {
                    Ok(member) => members.push(member),
                    Err(error) => {
                        errors.push(error);
                        p.recover_member(start);
                    }
                }
            }
        }
        declarations.push(Declaration {
            name,
            original,
            interface,
            annotations,
            members,
            origin,
        });
    }
    ensure!(
        !declarations.is_empty(),
        p.error("missing class declaration")
    );
    errors.finish()?;
    Ok(ParsedSource {
        builtin: source.builtin,
        package,
        imports,
        declarations,
    })
}

pub(crate) fn parse_all(sources: &[MappingSource]) -> Result<Vec<ParsedSource>> {
    let mut parsed = Vec::new();
    let mut errors = MappingDiagnostics::default();
    for source in sources {
        match parse(source) {
            Ok(source) => parsed.push(source),
            Err(error) => errors.push(error),
        }
    }
    errors.finish()?;
    Ok(parsed)
}

struct Parser {
    origin: MappingOrigin,
    tokens: Vec<Token>,
    at: usize,
}
impl Parser {
    // Restart at a declaration terminator, respecting annotation arrays and
    // empty method bodies. The outer class's closing brace remains unconsumed.
    fn recover_member(&mut self, start: usize) {
        self.at = start;
        let mut braces = 0;
        loop {
            match self.token().kind {
                Kind::End => return,
                Kind::Symbol('}') if braces == 0 => return,
                Kind::Symbol('{') => braces += 1,
                Kind::Symbol('}') => braces -= 1,
                Kind::Symbol(';') if braces == 0 => {
                    self.at += 1;
                    return;
                }
                _ => {}
            }
            self.at += 1;
        }
    }
    fn origin(&self) -> MappingOrigin {
        let token = self.token();
        self.origin.at(token.line, token.column, token.width)
    }
    fn token(&self) -> &Token {
        &self.tokens[self.at]
    }
    fn eat(&mut self, ch: char) -> bool {
        if self.token().kind == Kind::Symbol(ch) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, word: &str) -> bool {
        if matches!(&self.token().kind, Kind::Word(w) if w == word) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn error(&self, message: &str) -> anyhow::Error {
        MappingDiagnostic::new(
            format!("{message}; found {}", self.token().kind),
            self.origin(),
        )
        .into()
    }
    fn expect(&mut self, ch: char) -> Result<()> {
        if self.eat(ch) {
            Ok(())
        } else {
            Err(self.error(&format!("expected '{ch}'")))
        }
    }
    fn expect_word(&mut self, word: &str) -> Result<()> {
        if self.eat_word(word) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {word}")))
        }
    }
    fn word(&mut self) -> Result<String> {
        if let Kind::Word(word) = &self.token().kind {
            let word = word.clone();
            self.at += 1;
            Ok(word)
        } else {
            Err(self.error("expected identifier"))
        }
    }
    fn qualified(&mut self) -> Result<String> {
        let mut name = self.word()?;
        while self.token().kind == Kind::Symbol('.')
            && self
                .tokens
                .get(self.at + 1)
                .is_some_and(|token| matches!(token.kind, Kind::Word(_)))
        {
            self.at += 1;
            name.push('.');
            name.push_str(&self.word()?);
        }
        Ok(name)
    }
    fn was(&mut self) -> Option<String> {
        if let Kind::Was(name) = &self.token().kind {
            let name = name.clone();
            self.at += 1;
            Some(name)
        } else {
            None
        }
    }
    fn modifiers(&mut self) {
        while matches!(&self.token().kind, Kind::Word(w) if ["public","protected","private","static","final","abstract","native","synchronized","transient","volatile","strictfp","default"].contains(&w.as_str()))
        {
            self.at += 1;
        }
    }
    fn import(&mut self) -> Result<String> {
        ensure!(
            !self.eat_word("static"),
            self.error("static imports are not supported in mapping declarations")
        );
        let mut name = self.word()?;
        while self.eat('.') {
            if self.eat('*') {
                name.push_str(".*");
                break;
            }
            name.push('.');
            name.push_str(&self.word()?);
        }
        self.expect(';')?;
        Ok(name)
    }
    fn attributes(&mut self) -> Result<Vec<Annotation>> {
        let mut annotations = Vec::new();
        loop {
            let start = self.at;
            annotations.extend(self.annotations()?);
            self.modifiers();
            if self.at == start {
                return Ok(annotations);
            }
        }
    }
    fn dimensions(&mut self) -> Result<usize> {
        let mut n = 0;
        while self.eat('[') {
            self.expect(']')?;
            n += 1;
        }
        Ok(n)
    }
    fn ty(&mut self) -> Result<Type> {
        Ok(Type {
            origin: self.origin(),
            name: self.qualified()?,
            dimensions: self.dimensions()?,
        })
    }
    fn annotations(&mut self) -> Result<Vec<Annotation>> {
        let mut annotations = Vec::new();
        while self.token().kind == Kind::Symbol('@') {
            let mut origin = self.origin();
            self.at += 1;
            let name = self.qualified()?.rsplit('.').next().unwrap().to_owned();
            origin.width = name.chars().count() + 1;
            let mut args = BTreeMap::new();
            if self.eat('(') && !self.eat(')') {
                loop {
                    let key = if matches!(&self.token().kind, Kind::Word(_))
                        && self
                            .tokens
                            .get(self.at + 1)
                            .is_some_and(|t| t.kind == Kind::Symbol('='))
                    {
                        let key = self.word()?;
                        self.expect('=')?;
                        key
                    } else {
                        "value".into()
                    };
                    let value = self.value()?;
                    ensure!(
                        args.insert(key.clone(), value).is_none(),
                        self.error(&format!("duplicate annotation argument {key}"))
                    );
                    if self.eat(')') {
                        break;
                    }
                    self.expect(',')?;
                }
            }
            annotations.push(Annotation { name, args, origin });
        }
        Ok(annotations)
    }
    fn value(&mut self) -> Result<Value> {
        if self.eat('{') {
            let mut values = Vec::new();
            if !self.eat('}') {
                loop {
                    values.push(self.value()?);
                    if self.eat('}') {
                        break;
                    }
                    self.expect(',')?;
                    if self.eat('}') {
                        break;
                    }
                }
            }
            return Ok(Value::Array(values));
        }
        let origin = self.origin();
        let literal_error = |message| MappingDiagnostic::new(message, origin.clone());
        let negative = self.eat('-');
        let positive = if negative { false } else { self.eat('+') };
        let value = match self.token().kind.clone() {
            Kind::Number(raw) => {
                self.at += 1;
                let raw = raw.replace('_', "");
                let wide = raw.ends_with(['l', 'L']);
                let raw = raw.trim_end_matches(['l', 'L']);
                let (digits, radix) =
                    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
                        (hex, 16)
                    } else if let Some(binary) =
                        raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B"))
                    {
                        (binary, 2)
                    } else if raw.len() > 1 && raw.starts_with('0') {
                        (&raw[1..], 8)
                    } else {
                        (raw, 10)
                    };
                let unsigned = u64::from_str_radix(digits, radix)
                    .map_err(|_| literal_error("invalid integral literal"))?;
                let number = if radix != 10 {
                    if wide {
                        unsigned as i64
                    } else {
                        let bits = u32::try_from(unsigned)
                            .map_err(|_| literal_error("integer literal requires L suffix"))?;
                        i64::from(bits as i32)
                    }
                } else {
                    ensure!(
                        unsigned <= i64::MAX as u64 + u64::from(negative),
                        literal_error("integer literal out of range")
                    );
                    unsigned as i64
                };
                Value::Integer(if negative {
                    number.wrapping_neg()
                } else {
                    number
                })
            }
            Kind::Char(ch) if !negative && !positive => {
                self.at += 1;
                Value::Integer(ch as i64)
            }
            Kind::String(value) if !negative && !positive => {
                self.at += 1;
                Value::String(value)
            }
            Kind::Word(word)
                if !negative && !positive && ["true", "false"].contains(&word.as_str()) =>
            {
                self.at += 1;
                Value::Bool(word == "true")
            }
            Kind::Word(_) if !negative && !positive => {
                let name = self.qualified()?;
                let Some(name) = name.strip_suffix(".class") else {
                    return Err(literal_error("expected class literal ending in .class").into());
                };
                Value::Class(name.into())
            }
            _ => return Err(self.error("expected a literal value")),
        };
        Ok(value)
    }
    fn member(&mut self, class: &str) -> Result<Member> {
        let origin = self.origin();
        let annotations = self.attributes()?;
        let mut ty = self.ty()?;
        let constructor =
            ty.name == class && ty.dimensions == 0 && self.token().kind == Kind::Symbol('(');
        let name = if constructor {
            "<init>".into()
        } else {
            self.word()?
        };
        if constructor {
            ty = Type {
                origin: ty.origin,
                name: "void".into(),
                dimensions: 0,
            };
        }
        let parameters = if self.eat('(') {
            let mut parameters = Vec::new();
            if !self.eat(')') {
                loop {
                    let annotations = self.attributes()?;
                    let mut ty = self.ty()?;
                    if self.eat('.') {
                        self.expect('.')?;
                        self.expect('.')?;
                        ty.dimensions += 1;
                    }
                    let origin = self.origin();
                    let name = self.word()?;
                    ty.dimensions += self.dimensions()?;
                    parameters.push(Parameter {
                        origin,
                        name,
                        ty,
                        annotations,
                    });
                    if self.eat(')') {
                        break;
                    }
                    self.expect(',')?;
                }
            }
            Some(parameters)
        } else {
            ty.dimensions += self.dimensions()?;
            None
        };
        let mut original = self.was();
        if self.eat_word("throws") {
            self.qualified()?;
            while self.eat(',') {
                self.qualified()?;
            }
        }
        let initializer = if self.eat('=') {
            Some(self.value()?)
        } else {
            None
        };
        if let Some(name) = self.was() {
            ensure!(
                original.replace(name).is_none(),
                self.error("duplicate member was comment")
            );
        }
        if self.eat('{') {
            ensure!(parameters.is_some(), self.error("field cannot have a body"));
            self.expect('}')?;
        } else {
            self.expect(';')?;
            ensure!(
                !constructor,
                self.error("constructors require an empty body")
            );
        }
        if constructor {
            ensure!(
                original.is_none(),
                self.error("constructors do not use a was comment")
            );
        }
        if parameters.is_some() {
            ensure!(
                initializer.is_none(),
                self.error("method cannot have an initializer")
            );
        }
        Ok(Member {
            name,
            original,
            ty,
            parameters,
            annotations,
            initializer,
            origin,
        })
    }
}
