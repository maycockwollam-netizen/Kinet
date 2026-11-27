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

/// Một mệnh đề trong comprehension: `for x in iter` hoặc bộ lọc `if cond`.
#[derive(Debug, Clone, PartialEq)]
pub enum CompClause {
    For {
        var: String,
        iterable: Expr,
    },
    If(Expr),
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
    If {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
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
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    // Gán mục: `name[i][j] = value` — cập nhật trong container có tên rõ ràng.
    IndexAssign {
        name: String,
        indices: Vec<Expr>,
        value: Box<Expr>,
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
    // Cắt mục: `a[1:4]`, `a[:4]`, `a[::2]`, `a[::-1]` (start/end/step tùy chọn).
    Slice {
        target: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
    // List comprehension: `[expr for x in iter if cond] ...`
    Comp {
        element: Box<Expr>,
        clauses: Vec<CompClause>,
    },
    // Dict comprehension: `{ key: value for x in iter if cond }`
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        clauses: Vec<CompClause>,
    },
    // Phương thức: `receiver.method(args)` — desugar thành native có sẵn.
    MethodCall {
        target: Box<Expr>,
        name: String,
        args: Vec<Expr>,
    },
    // Lấy field của struct: `p.x`
    Member {
        target: Box<Expr>,
        name: String,
    },
    // Gán field: `p.x = v` (hoặc lồng `a.b.c = v`) — root biến + chuỗi field.
    MemberAssign {
        name: String,
        path: Vec<String>,
        value: Box<Expr>,
    },
}

/// Một phương thức khai báo trong struct (có `self` khi chạy).
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
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
    Throw(Expr),
    Try {
        body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Option<Vec<Stmt>>,
        finally_body: Vec<Stmt>,
    },
    Func {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Struct {
        name: String,
        fields: Vec<(String, Expr)>,
        methods: Vec<MethodDef>,
    },
    Return(Expr),
}