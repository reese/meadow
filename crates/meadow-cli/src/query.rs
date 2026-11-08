//! Restricted query expressions for `meadow list`.
//!
//! Supported forms mirror the common YARD queries:
//! `object.visibility == :public`, `object.type == :method`,
//! `@api.text == 'public'`, and boolean combinations.

use meadow_model::{CodeObject, KindData, ObjectKind, Scope, Visibility};

/// A parsed query expression.
#[derive(Debug, Clone)]
pub struct Query {
    expression: Expr,
}

#[derive(Debug, Clone)]
enum Expr {
    Or(Box<Self>, Box<Self>),
    And(Box<Self>, Box<Self>),
    Not(Box<Self>),
    Compare(Operand, Operator, Operand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Equal,
    NotEqual,
}

#[derive(Debug, Clone)]
enum Operand {
    Path(Vec<String>),
    String(String),
    Number(i64),
    Bool(bool),
    Nil,
}

#[derive(Debug, Clone)]
enum Value {
    String(String),
    Number(i64),
    Bool(bool),
    Nil,
}

impl Query {
    /// Parses a query expression.
    ///
    /// # Errors
    ///
    /// Returns a message for invalid syntax.
    pub fn parse(source: &str) -> Result<Self, String> {
        let tokens = tokenize(source)?;
        let mut parser = Parser {
            tokens,
            position: 0,
        };
        let expression = parser.parse_or()?;
        if parser.peek() != &Token::End {
            return Err(format!("unexpected token {:?}", parser.peek()));
        }
        Ok(Self { expression })
    }

    /// Evaluates the query against an object.
    #[must_use]
    pub fn matches(&self, object: &CodeObject) -> bool {
        self.expression.evaluate(object)
    }
}

impl Expr {
    fn evaluate(&self, object: &CodeObject) -> bool {
        match self {
            Self::Or(left, right) => left.evaluate(object) || right.evaluate(object),
            Self::And(left, right) => left.evaluate(object) && right.evaluate(object),
            Self::Not(inner) => !inner.evaluate(object),
            Self::Compare(left, operator, right) => {
                let left = left.value(object);
                let right = right.value(object);
                let equal = match (left, right) {
                    (Value::String(left), Value::String(right)) => left == right,
                    (Value::Number(left), Value::Number(right)) => left == right,
                    (Value::Bool(left), Value::Bool(right)) => left == right,
                    (Value::Nil, Value::Nil) => true,
                    _ => false,
                };
                match operator {
                    Operator::Equal => equal,
                    Operator::NotEqual => !equal,
                }
            }
        }
    }
}

impl Operand {
    fn value(&self, object: &CodeObject) -> Value {
        match self {
            Self::String(value) => Value::String(value.clone()),
            Self::Number(value) => Value::Number(*value),
            Self::Bool(value) => Value::Bool(*value),
            Self::Nil => Value::Nil,
            Self::Path(path) => object_value(object, path),
        }
    }
}

fn object_value(object: &CodeObject, path: &[String]) -> Value {
    if let Some(first) = path.first()
        && let Some(tag) = first.strip_prefix('@')
    {
        let tag = object
            .docstring
            .tags
            .iter()
            .find(|candidate| candidate.tag == tag);
        return match tag {
            None => Value::Nil,
            Some(tag) if path.get(1).is_some_and(|field| field == "text") => {
                Value::String(tag.text.to_string_lossy().into_owned())
            }
            Some(tag) => Value::String(tag.text.to_string_lossy().into_owned()),
        };
    }

    match path {
        [object_name, field] if object_name == "object" => match field.as_str() {
            "visibility" => Value::String(visibility(object.visibility)),
            "type" | "kind" => Value::String(kind(object)),
            "name" => Value::String(object.name.to_string_lossy().into_owned()),
            "path" => Value::String(object.path.to_string_lossy().into_owned()),
            "scope" => Value::String(scope(object)),
            "docstring" | "doc" => {
                Value::String(object.docstring.text.to_string_lossy().into_owned())
            }
            "dynamic" => Value::Bool(object.dynamic),
            _ => Value::Nil,
        },
        _ => Value::Nil,
    }
}

fn visibility(visibility: Visibility) -> String {
    match visibility {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
    }
    .to_string()
}

fn kind(object: &CodeObject) -> String {
    match object.kind {
        ObjectKind::Root => "root",
        ObjectKind::Module => "module",
        ObjectKind::Class => "class",
        ObjectKind::Method => "method",
        ObjectKind::Constant => "constant",
        ObjectKind::ClassVariable => "class_variable",
        ObjectKind::InstanceVariable => "instance_variable",
        ObjectKind::Macro => "macro",
    }
    .to_string()
}

fn scope(object: &CodeObject) -> String {
    match &object.extra {
        KindData::Method { scope, .. } => match scope {
            Scope::Instance => "instance",
            Scope::Class => "class",
        },
        _ => "namespace",
    }
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Tag(String),
    String(String),
    Symbol(String),
    Number(i64),
    True,
    False,
    Nil,
    Dot,
    And,
    Or,
    Not,
    Equal,
    NotEqual,
    LeftParen,
    RightParen,
    End,
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let characters: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        match character {
            '(' => {
                tokens.push(Token::LeftParen);
                index += 1;
            }
            ')' => {
                tokens.push(Token::RightParen);
                index += 1;
            }
            '.' => {
                tokens.push(Token::Dot);
                index += 1;
            }
            '&' if characters.get(index + 1) == Some(&'&') => {
                tokens.push(Token::And);
                index += 2;
            }
            '|' if characters.get(index + 1) == Some(&'|') => {
                tokens.push(Token::Or);
                index += 2;
            }
            '=' if characters.get(index + 1) == Some(&'=') => {
                tokens.push(Token::Equal);
                index += 2;
            }
            '!' if characters.get(index + 1) == Some(&'=') => {
                tokens.push(Token::NotEqual);
                index += 2;
            }
            '!' => {
                tokens.push(Token::Not);
                index += 1;
            }
            '@' => {
                index += 1;
                let start = index;
                while index < characters.len() && is_ident_char(characters[index]) {
                    index += 1;
                }
                tokens.push(Token::Tag(characters[start..index].iter().collect()));
            }
            ':' => {
                index += 1;
                let start = index;
                while index < characters.len() && is_ident_char(characters[index]) {
                    index += 1;
                }
                tokens.push(Token::Symbol(characters[start..index].iter().collect()));
            }
            '\'' | '"' => {
                let quote = character;
                index += 1;
                let mut value = String::new();
                while index < characters.len() && characters[index] != quote {
                    value.push(characters[index]);
                    index += 1;
                }
                if index >= characters.len() {
                    return Err("unterminated string in query".to_string());
                }
                index += 1;
                tokens.push(Token::String(value));
            }
            character if character.is_ascii_digit() => {
                let start = index;
                while index < characters.len() && characters[index].is_ascii_digit() {
                    index += 1;
                }
                let value = characters[start..index].iter().collect::<String>();
                tokens.push(Token::Number(value.parse().unwrap_or(0)));
            }
            character if is_ident_char(character) => {
                let start = index;
                while index < characters.len() && is_ident_char(characters[index]) {
                    index += 1;
                }
                let value = characters[start..index].iter().collect::<String>();
                tokens.push(match value.as_str() {
                    "true" => Token::True,
                    "false" => Token::False,
                    "nil" => Token::Nil,
                    _ => Token::Ident(value),
                });
            }
            other => return Err(format!("unexpected character '{other}' in query")),
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}

fn is_ident_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.position].clone();
        self.position += 1;
        token
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.peek() == &Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while self.peek() == &Token::And {
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Token::Not => {
                self.advance();
                Ok(Expr::Not(Box::new(self.parse_unary()?)))
            }
            Token::LeftParen => {
                self.advance();
                let expression = self.parse_or()?;
                if self.advance() != Token::RightParen {
                    return Err("expected ')' in query".to_string());
                }
                Ok(expression)
            }
            _ => self.parse_comparison(),
        }
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let left = self.parse_operand()?;
        let operator = match self.advance() {
            Token::Equal => Operator::Equal,
            Token::NotEqual => Operator::NotEqual,
            other => return Err(format!("expected '==' or '!=' in query, found {other:?}")),
        };
        let right = self.parse_operand()?;
        Ok(Expr::Compare(left, operator, right))
    }

    fn parse_operand(&mut self) -> Result<Operand, String> {
        match self.advance() {
            Token::String(value) | Token::Symbol(value) => Ok(Operand::String(value)),
            Token::Number(value) => Ok(Operand::Number(value)),
            Token::True => Ok(Operand::Bool(true)),
            Token::False => Ok(Operand::Bool(false)),
            Token::Nil => Ok(Operand::Nil),
            Token::Ident(first) => {
                let mut path = vec![first];
                while self.peek() == &Token::Dot {
                    self.advance();
                    match self.advance() {
                        Token::Ident(field) => path.push(field),
                        other => return Err(format!("expected field after '.', found {other:?}")),
                    }
                }
                Ok(Operand::Path(path))
            }
            Token::Tag(tag) => {
                let mut path = vec![format!("@{tag}")];
                while self.peek() == &Token::Dot {
                    self.advance();
                    match self.advance() {
                        Token::Ident(field) => path.push(field),
                        other => return Err(format!("expected field after '.', found {other:?}")),
                    }
                }
                Ok(Operand::Path(path))
            }
            other => Err(format!("unexpected token {other:?} in query")),
        }
    }
}
