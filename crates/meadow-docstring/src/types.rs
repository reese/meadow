//! Parsing of YARD type expressions.

use std::fmt;

use meadow_model::{HashStyle, SourceText, TypeExpr};

/// An error produced while parsing a type expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParseError {
    /// Human-readable message.
    pub message: String,
    /// Byte offset where the error was detected.
    pub offset: usize,
}

impl fmt::Display for TypeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for TypeParseError {}

/// Parses a comma-separated type list into a structured expression.
///
/// # Errors
///
/// Returns an error when the input is malformed.
pub fn parse_type_list(text: &SourceText) -> Result<TypeExpr, TypeParseError> {
    Parser {
        input: text.as_bytes(),
        position: 0,
    }
    .parse()
}

/// Parses raw UTF-8 or non-UTF-8 type bytes into a structured expression.
///
/// # Errors
///
/// Returns an error when the input is malformed.
pub fn parse_type_list_bytes(input: &[u8]) -> Result<TypeExpr, TypeParseError> {
    Parser { input, position: 0 }.parse()
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn parse(mut self) -> Result<TypeExpr, TypeParseError> {
        let expression = self.parse_list()?;
        self.skip_whitespace();
        if self.position != self.input.len() {
            return Err(self.error("unexpected trailing input"));
        }
        Ok(expression)
    }

    fn parse_list(&mut self) -> Result<TypeExpr, TypeParseError> {
        let mut types = vec![self.parse_type()?];
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b',') {
                break;
            }
            self.position += 1;
            types.push(self.parse_type()?);
        }

        if types.len() == 1 {
            types.pop().ok_or_else(|| self.error("empty type list"))
        } else {
            Ok(TypeExpr::Union(types))
        }
    }

    fn parse_type(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.skip_whitespace();
        match self.peek() {
            None => Err(self.error("expected a type")),
            Some(b'(') => self.parse_tuple(),
            Some(b'#') => self.parse_duck(),
            Some(b':') => self.parse_symbol(),
            Some(byte) if is_name_start(byte) => self.parse_name(),
            Some(byte) if byte.is_ascii_digit() => Ok(self.parse_numeric_literal()),
            Some(byte) => Err(self.error(&format!("unexpected character '{}'", byte as char))),
        }
    }

    fn parse_tuple(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.position += 1;
        let mut types = Vec::new();

        loop {
            self.skip_whitespace();
            if self.peek() == Some(b')') {
                self.position += 1;
                if types.is_empty() {
                    return Err(self.error("empty tuple type"));
                }
                return Ok(TypeExpr::Tuple(types));
            }

            types.push(self.parse_type()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.position += 1;
                    self.skip_whitespace();
                    if self.peek() == Some(b')') {
                        return Err(self.error("trailing comma in tuple type"));
                    }
                }
                Some(b')') => {
                    self.position += 1;
                    return Ok(TypeExpr::Tuple(types));
                }
                _ => return Err(self.error("expected ',' or ')' in tuple type")),
            }
        }
    }

    fn parse_duck(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.position += 1;
        let start = self.position;
        while self.peek().is_some_and(is_name_char) {
            self.position += 1;
        }
        if self.position == start {
            return Err(self.error("expected a duck type name after '#'"));
        }
        Ok(TypeExpr::Duck(SourceText::from_bytes(
            &self.input[start..self.position],
        )))
    }

    fn parse_symbol(&mut self) -> Result<TypeExpr, TypeParseError> {
        let start = self.position;
        self.position += 1;
        while let Some(byte) = self.peek() {
            if is_delimiter(byte) {
                break;
            }
            self.position += 1;
        }
        if self.position == start + 1 {
            return Err(self.error("expected a symbol name after ':'"));
        }
        Ok(TypeExpr::Literal(SourceText::from_bytes(
            &self.input[start..self.position],
        )))
    }

    fn parse_numeric_literal(&mut self) -> TypeExpr {
        let start = self.position;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'_'))
        {
            self.position += 1;
        }
        TypeExpr::Literal(SourceText::from_bytes(&self.input[start..self.position]))
    }

    fn parse_name(&mut self) -> Result<TypeExpr, TypeParseError> {
        let start = self.position;
        while self.peek().is_some_and(is_name_char) {
            self.position += 1;
        }
        let name = SourceText::from_bytes(&self.input[start..self.position]);

        self.skip_whitespace();
        match self.peek() {
            Some(b'<') => self.parse_angle_generic(name),
            Some(b'{') => self.parse_brace_hash(),
            _ => {
                if is_literal_keyword(name.as_bytes()) {
                    Ok(TypeExpr::Literal(name))
                } else {
                    Ok(TypeExpr::Name(name))
                }
            }
        }
    }

    fn parse_angle_generic(&mut self, name: SourceText) -> Result<TypeExpr, TypeParseError> {
        self.position += 1;
        let mut params = Vec::new();

        loop {
            self.skip_whitespace();
            if self.peek() == Some(b'>') {
                self.position += 1;
                break;
            }
            params.push(self.parse_type()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'>') => {
                    self.position += 1;
                    break;
                }
                _ => return Err(self.error("expected ',' or '>' in generic type")),
            }
        }

        if params.is_empty() {
            return Err(self.error("empty generic type"));
        }

        if name.as_bytes() == b"Hash" && params.len() == 2 {
            let value = params
                .pop()
                .ok_or_else(|| self.error("missing hash value type"))?;
            let key = params
                .pop()
                .ok_or_else(|| self.error("missing hash key type"))?;
            Ok(TypeExpr::Hash {
                key: Box::new(key),
                value: Box::new(value),
                style: HashStyle::Angles,
            })
        } else {
            Ok(TypeExpr::Generic {
                base: Box::new(TypeExpr::Name(name)),
                params,
            })
        }
    }

    fn parse_brace_hash(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.position += 1;
        self.skip_whitespace();
        let key = self.parse_type()?;
        self.skip_whitespace();

        if self.input[self.position..].starts_with(b"=>") {
            self.position += 2;
        } else if matches!(self.peek(), Some(b',' | b':')) {
            self.position += 1;
        } else {
            return Err(self.error("expected '=>' or ',' in hash type"));
        }

        self.skip_whitespace();
        let value = self.parse_type()?;
        self.skip_whitespace();
        if self.peek() != Some(b'}') {
            return Err(self.error("expected '}' in hash type"));
        }
        self.position += 1;

        Ok(TypeExpr::Hash {
            key: Box::new(key),
            value: Box::new(value),
            style: HashStyle::Braces,
        })
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn error(&self, message: &str) -> TypeParseError {
        TypeParseError {
            message: message.to_string(),
            offset: self.position,
        }
    }
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'?' | b'!') || byte >= 0x80
}

fn is_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b',' | b'>' | b'}' | b')' | b']' | b'<' | b'{' | b'=' | b'#'
        )
}

fn is_literal_keyword(name: &[u8]) -> bool {
    matches!(name, b"true" | b"false" | b"nil" | b"void" | b"self")
}
