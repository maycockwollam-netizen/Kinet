//! AST (Abstract Syntax Tree): biểu diễn cấu trúc ngữ pháp.

use crate::token::TokenKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    // Số, chuỗi, bool — so sánh bằng
    Literal(Expr),
    // Bắt giá trị vào biến (vd: case e => ...)
    Binding(String),
    // "_" hoặc "default" — khớp mọi thứ
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Nil,
    Variable(String),
    Assign {
        name: String,
        value: Box<Expr>,
    },
    Unary {
        op: TokenKind,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: TokenKind,
        right: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Lambda {
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    // Template chuỗi: danh sách phần (Str literal + biểu thức interpolated).
    Template(Vec<Expr>),
    List(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    // Mỗi câu lệnh được bọc với số dòng nguồn → báo lỗi có vị trí.
    Ann {
        line: usize,
        inner: Box<Stmt>,
    },
    Expr(Expr),
    Let {
        name: String,
        value: Expr,
    },
    Print(Vec<Expr>),
    Println(Vec<Expr>),
    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    For {
        var: String,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Import(String),
    Try {
        body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Vec<Stmt>,
    },
    Func {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Return(Expr),
}