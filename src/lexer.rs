//! Lexer: quét chuỗi code thành danh sách token.

use crate::token::{Token, TokenKind};

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = token.kind == TokenKind::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_whitespace_and_comments();
        let line = self.line;

        let Some(&c) = self.chars.get(self.pos) else {
            return Ok(Token::new(TokenKind::Eof, line));
        };

        // Số
        if c.is_ascii_digit() {
            return self.number(line);
        }

        // Chuỗi
        if c == '"' {
            return self.string(line);
        }

        // Identifier / keyword
        if c.is_alphabetic() || c == '_' {
            return self.identifier(line);
        }

        self.pos += 1;
        let kind = match c {
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            ',' => TokenKind::Comma,
            ';' => TokenKind::Semi,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            '=' => {
                if self.match_char('=') {
                    TokenKind::Eq
                } else if self.match_char('>') {
                    TokenKind::Arrow
                } else {
                    TokenKind::Assign
                }
            }
            '!' => {
                if self.match_char('=') {
                    TokenKind::NotEq
                } else {
                    TokenKind::Not
                }
            }
            '<' => {
                if self.match_char('=') {
                    TokenKind::Le
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.match_char('=') {
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            _ => return Err(format!("Lỗi dòng {}: ký tự không hợp lệ '{}'", line, c)),
        };

        Ok(Token::new(kind, line))
    }

    fn number(&mut self, line: usize) -> Result<Token, String> {
        let start = self.pos;
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let value = text
            .parse::<i64>()
            .map_err(|_| format!("Lỗi dòng {}: số quá lớn '{}'", line, text))?;
        Ok(Token::new(TokenKind::Number(value), line))
    }

    fn string(&mut self, line: usize) -> Result<Token, String> {
        self.pos += 1; // bỏ dấu mở nháy "
        let mut out = String::new();
        loop {
            let Some(&c) = self.chars.get(self.pos) else {
                return Err(format!("Lỗi dòng {}: chuỗi chưa đóng ngoặc", line));
            };
            match c {
                '"' => {
                    self.pos += 1;
                    break;
                }
                '\n' => {
                    self.line += 1;
                    out.push('\n');
                    self.pos += 1;
                }
                '\\' => {
                    self.pos += 1;
                    let Some(&esc) = self.chars.get(self.pos) else {
                        return Err(format!("Lỗi dòng {}: chuỗi chưa đóng ngoặc", line));
                    };
                    match esc {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        other => {
                            return Err(format!(
                                "Lỗi dòng {}: escape không hợp lệ '\\{}'",
                                line, other
                            ))
                        }
                    }
                    self.pos += 1;
                }
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
        Ok(Token::new(TokenKind::Str(out), line))
    }

    fn identifier(&mut self, line: usize) -> Result<Token, String> {
        let start = self.pos;
        while self
            .chars
            .get(self.pos)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let kind = match text.as_str() {
            "let" => TokenKind::Let,
            "func" => TokenKind::Func,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "return" => TokenKind::Return,
            "match" => TokenKind::Match,
            "case" => TokenKind::Case,
            "default" => TokenKind::Default,
            "print" => TokenKind::Print,
            "println" => TokenKind::Println,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            _ => TokenKind::Ident(text),
        };
        Ok(Token::new(kind, line))
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.chars.get(self.pos) {
                Some(' ') | Some('\t') | Some('\r') => self.pos += 1,
                Some('\n') => {
                    self.line += 1;
                    self.pos += 1;
                }
                // comment // đến hết dòng
                Some('/') if self.chars.get(self.pos + 1) == Some(&'/') => {
                    while self.chars.get(self.pos).is_some_and(|c| *c != '\n') {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.chars.get(self.pos) == Some(&expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}