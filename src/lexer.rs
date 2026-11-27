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
            ':' => TokenKind::Colon,
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
                } else if self.match_char('<') {
                    TokenKind::Shl
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.match_char('=') {
                    TokenKind::Ge
                } else if self.match_char('>') {
                    TokenKind::Shr
                } else {
                    TokenKind::Gt
                }
            }
            '&' => TokenKind::Band,
            '|' => TokenKind::Bor,
            _ => return Err(format!("Lỗi dòng {}: ký tự không hợp lệ '{}'", line, c)),
        };

        Ok(Token::new(kind, line))
    }

    fn number(&mut self, line: usize) -> Result<Token, String> {
        let start = self.pos;
        let mut is_float = false;
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        // Phần thập phân: '.' phải có chữ số đi sau (tránh "1." bị hiểu lầm).
        if self
            .chars
            .get(self.pos)
            .is_some_and(|c| *c == '.' && self.chars.get(self.pos + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            is_float = true;
            self.pos += 1; // dấu '.'
            while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if is_float {
            let value = text
                .parse::<f64>()
                .map_err(|_| format!("Lỗi dòng {}: số thập phân không hợp lệ '{}'", line, text))?;
            Ok(Token::new(TokenKind::Float(value), line))
        } else {
            let value = text
                .parse::<i64>()
                .map_err(|_| format!("Lỗi dòng {}: số quá lớn '{}'", line, text))?;
            Ok(Token::new(TokenKind::Number(value), line))
        }
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
                        'r' => out.push('\r'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '$' => out.push(crate::token::ESCAPED_DOLLAR), // '\$' → sentinel→'$' khi tạo Str
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
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "import" => TokenKind::Import,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "finally" => TokenKind::Finally,
            "throw" => TokenKind::Throw,
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
                // comment /* ... */ (nhiều dòng được)
                Some('/') if self.chars.get(self.pos + 1) == Some(&'*') => {
                    self.pos += 2;
                    loop {
                        match self.chars.get(self.pos) {
                            Some('*') if self.chars.get(self.pos + 1) == Some(&'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some('\n') => {
                                self.line += 1;
                                self.pos += 1;
                            }
                            Some(_) => self.pos += 1,
                            None => break, // /* chưa đóng — vẫn chấp nhận tới hết file
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    fn kinds(src: &str) -> Vec<TokenKind> {
        Lexer::new(src)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn tokenize_let_and_expression() {
        assert_eq!(
            kinds("let x = 2 + 3;"),
            vec![
                TokenKind::Let,
                TokenKind::Ident("x".into()),
                TokenKind::Assign,
                TokenKind::Number(2),
                TokenKind::Plus,
                TokenKind::Number(3),
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn tokenize_string_with_escape() {
        assert_eq!(kinds(r#""a\nb""#)[0], TokenKind::Str("a\nb".into()));
    }

    #[test]
    fn tokenize_all_keywords() {
        let src = "let func if else while for in return break continue match case default print println and or not true false";
        let got = kinds(src);
        for kw in [
            TokenKind::Let,
            TokenKind::Func,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::While,
            TokenKind::For,
            TokenKind::In,
            TokenKind::Return,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::Match,
            TokenKind::Case,
            TokenKind::Default,
            TokenKind::Print,
            TokenKind::Println,
            TokenKind::And,
            TokenKind::Or,
            TokenKind::Not,
            TokenKind::True,
            TokenKind::False,
        ] {
            assert!(got.contains(&kw), "thiếu keyword: {:?}", kw);
        }
    }

    #[test]
    fn tokenize_arrow_and_brackets() {
        let got = kinds("x => [1, 2];");
        assert!(got.contains(&TokenKind::Arrow));
        assert!(got.contains(&TokenKind::LBracket));
        assert!(got.contains(&TokenKind::RBracket));
    }

    #[test]
    fn skip_comments() {
        let got = kinds("// chú thích\nlet a = 1;");
        assert!(got.contains(&TokenKind::Let));
        assert!(!got.iter().any(|k| matches!(k, TokenKind::Slash)));
    }

    #[test]
    fn unterminated_string_errors() {
        let err = Lexer::new(r#""abc"#).tokenize().unwrap_err();
        assert!(err.contains("chuỗi chưa đóng"));
    }

    #[test]
    fn invalid_char_errors() {
        let err = Lexer::new("let @ = 1;").tokenize().unwrap_err();
        assert!(err.contains("ký tự không hợp lệ"));
    }
}