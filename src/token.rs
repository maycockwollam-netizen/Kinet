//! Định nghĩa các loại token (đơn vị nhỏ nhất của code).

/// Sentinel cho `\$` (dollar được escape): lexer đổi `\$` thành ký tự này để
/// parser phân biệt với `${` (bắt đầu template được sửa ngay khi tạo Str).
pub const ESCAPED_DOLLAR: char = '\u{E000}';

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number(i64),
    Float(f64),
    Str(String),
    Ident(String),
    True,
    False,

    // Keywords
    Let,
    Func,
    Struct,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Throw,
    Break,
    Continue,
    Import,
    Try,
    Catch,
    Finally,
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
    NotIn,  // not in — chỉ dùng trong parser, desugar thành not (x in y)
    Shl,    // <<
    Shr,    // >>
    Band,   // &
    Bor,    // |
    Arrow,  // =>

    // Punctuation
    Comma,  // ,
    Semi,   // ;
    Dot,    // . (phương thức: x.len())
    LParen, // (
    RParen, // )
    LBrace, // {
    RBrace, // }
    LBracket, // [
    RBracket, // ]
    Colon,  // :

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