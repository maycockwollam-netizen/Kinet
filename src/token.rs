//! Định nghĩa các loại token (đơn vị nhỏ nhất của code).

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number(i64),
    Str(String),
    Ident(String),
    True,
    False,

    // Keywords
    Let,
    Func,
    If,
    Else,
    While,
    Return,
    Print,
    Println,
    Match,
    Case,
    Default,
    And,
    Or,
    Not,

    // Operators
    Plus,   // +
    Minus,  // -
    Star,   // *
    Slash,  // /
    Percent, // %
    Assign, // =
    Eq,     // ==
    NotEq,  // !=
    Lt,     // <
    Gt,     // >
    Le,     // <=
    Ge,     // >=
    Arrow,  // =>

    // Punctuation
    Comma,  // ,
    Semi,   // ;
    LParen, // (
    RParen, // )
    LBrace, // {
    RBrace, // }
    LBracket, // [
    RBracket, // ]

    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
}

impl Token {
    pub fn new(kind: TokenKind, line: usize) -> Self {
        Self { kind, line }
    }
}