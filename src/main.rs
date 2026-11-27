//! Kinet — ngôn ngữ lập trình nhỏ do bạn tự viết, dành cho agent SDK.
//!
//! Cách dùng:
//!   cargo run                    → mở REPL
//!   cargo run -- file.kinet      → chạy file source
//!   cargo run -- -e "code"       → chạy code từ tham số
//!   cargo run -- --tokens file   → xem danh sách token (Lexer)
//!   cargo run -- --ast file      → xem cây cú pháp (Parser)

mod ast;
mod interpreter;
mod lexer;
mod parser;
mod token;

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use ast::{Expr, Pattern, Stmt};
use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;
use token::TokenKind;

fn main() -> ExitCode {
    // Chạy mọi thứ trên thread stack lớn (64MB) để đệ quy sâu trong Kinet
    // không tràn stack trước khi chạm MAX_CALL_DEPTH.
    let spawned = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_all);
    match spawned {
        Ok(handle) => match handle.join() {
            Ok(code) => code,
            Err(_) => ExitCode::FAILURE,
        },
        Err(_) => ExitCode::FAILURE,
    }
}

fn run_all() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        repl();
        ExitCode::SUCCESS
    } else if args[0] == "-e" {
        if args.len() < 2 {
            eprintln!("Cần code sau '-e'");
            return ExitCode::FAILURE;
        }
        run_or_report(&args[1])
    } else if args[0] == "--tokens" {
        if args.len() < 2 {
            eprintln!("Cần file sau '--tokens'");
            ExitCode::FAILURE
        } else {
            cmd_tokens(&args[1])
        }
    } else if args[0] == "--ast" {
        if args.len() < 2 {
            eprintln!("Cần file sau '--ast'");
            ExitCode::FAILURE
        } else {
            cmd_ast(&args[1])
        }
    } else {
        match fs::read_to_string(&args[0]) {
            Ok(source) => run_or_report(&source),
            Err(e) => {
                eprintln!("Không đọc được file '{}': {}", args[0], e);
                ExitCode::FAILURE
            }
        }
    }
}

fn run_or_report(source: &str) -> ExitCode {
    match run_source(source) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}

fn run_source(source: &str) -> Result<(), String> {
    let tokens = Lexer::new(source).tokenize()?;
    let stmts = Parser::new(tokens).parse()?;
    let mut interpreter = Interpreter::new();
    interpreter.run(&stmts)
}

/// `--tokens`: in ra từng token sau giai đoạn Lexer.
fn cmd_tokens(path: &str) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Không đọc được file '{}': {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    match Lexer::new(&source).tokenize() {
        Ok(tokens) => {
            println!("        {:?}", tokens.iter().map(|t| t.kind.clone()).collect::<Vec<_>>());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}

/// `--ast`: in cây cú pháp sau giai đoạn Parser (đã format dễ đọc).
fn cmd_ast(path: &str) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Không đọc được file '{}': {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    match Lexer::new(&source).tokenize().and_then(|t| Parser::new(t).parse()) {
        Ok(stmts) => {
            let mut out = String::new();
            for stmt in &stmts {
                p_stmt(stmt, 0, &mut out);
            }
            print!("{}", out);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}

// ---------- In cây cú pháp (AST printer) ----------

fn p_indent<'a>(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push_str("  ");
    }
}

fn p_stmt(stmt: &Stmt, indent: usize, out: &mut String) {
    match stmt {
        // Bỏ qua lớp Ann khi in AST — nội dung đã đủ.
        Stmt::Ann { inner, .. } => p_stmt(inner, indent, out),
        Stmt::Let { name, value } => {
            p_indent(out, indent);
            out.push_str(&format!("let {} = ", name));
            p_expr(value, 0, out);
            out.push('\n');
        }
        Stmt::Print(exprs) => {
            p_indent(out, indent);
            out.push_str("print(");
            p_exprs(exprs, 0, out);
            out.push_str(")\n");
        }
        Stmt::Println(exprs) => {
            p_indent(out, indent);
            out.push_str("println(");
            p_exprs(exprs, 0, out);
            out.push_str(")\n");
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => {
            p_indent(out, indent);
            out.push_str("if ");
            p_expr(cond, 0, out);
            out.push('\n');
            p_stmts(then_branch, indent + 1, out);
            if !else_branch.is_empty() {
                p_indent(out, indent);
                out.push_str("else\n");
                p_stmts(else_branch, indent + 1, out);
            }
        }
        Stmt::While { cond, body } => {
            p_indent(out, indent);
            out.push_str("while ");
            p_expr(cond, 0, out);
            out.push('\n');
            p_stmts(body, indent + 1, out);
        }
        Stmt::For { var, iterable, body } => {
            p_indent(out, indent);
            out.push_str(&format!("for {} in ", var));
            p_expr(iterable, 0, out);
            out.push('\n');
            p_stmts(body, indent + 1, out);
        }
        Stmt::Break => {
            p_indent(out, indent);
            out.push_str("break\n");
        }
        Stmt::Import(path) => {
            p_indent(out, indent);
            out.push_str(&format!("import \"{}\"\n", path));
        }
        Stmt::Try {
            body,
            catch_var,
            catch_body,
        } => {
            p_indent(out, indent);
            out.push_str("try\n");
            p_stmts(body, indent + 1, out);
            p_indent(out, indent);
            out.push_str("catch");
            if let Some(v) = catch_var {
                out.push(' ');
                out.push_str(v);
            }
            out.push('\n');
            p_stmts(catch_body, indent + 1, out);
        }
        Stmt::Continue => {
            p_indent(out, indent);
            out.push_str("continue\n");
        }
        Stmt::Func {
            name,
            params,
            body,
        } => {
            p_indent(out, indent);
            out.push_str(&format!("func {}({})\n", name, params.join(", ")));
            p_stmts(body, indent + 1, out);
        }
        Stmt::Return(e) => {
            p_indent(out, indent);
            out.push_str("return ");
            p_expr(e, 0, out);
            out.push('\n');
        }
        Stmt::Expr(e) => {
            p_indent(out, indent);
            p_expr(e, 0, out);
            out.push('\n');
        }
    }
}

fn p_stmts(stmts: &[Stmt], indent: usize, out: &mut String) {
    for s in stmts {
        p_stmt(s, indent, out);
    }
}

fn p_exprs(exprs: &[Expr], indent: usize, out: &mut String) {
    for (i, e) in exprs.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        p_expr(e, indent, out);
    }
}

fn p_expr(expr: &Expr, indent: usize, out: &mut String) {
    match expr {
        Expr::Number(n) => out.push_str(&n.to_string()),
        Expr::Float(f) => out.push_str(&format!("{}", f)),
        Expr::Str(s) => out.push_str(&format!("\"{}\"", s)),
        Expr::Bool(b) => out.push_str(&b.to_string()),
        Expr::Nil => out.push_str("nil"),
        Expr::Variable(name) => out.push_str(name),
        Expr::Assign { name, value } => {
            out.push_str(name);
            out.push_str(" = ");
            p_expr(value, indent, out);
        }
        Expr::Unary { op, expr } => {
            out.push_str(op_symbol(op));
            p_expr(expr, indent + 1, out);
        }
        Expr::Binary { left, op, right } => {
            out.push('(');
            p_expr(left, indent + 1, out);
            out.push(' ');
            out.push_str(op_symbol(op));
            out.push(' ');
            p_expr(right, indent + 1, out);
            out.push(')');
        }
        Expr::Call { callee, args } => {
            p_expr(callee, indent, out);
            out.push('(');
            p_exprs(args, indent + 1, out);
            out.push(')');
        }
        Expr::List(items) => {
            out.push('[');
            p_exprs(items, indent + 1, out);
            out.push(']');
        }
        Expr::Dict(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                p_expr(k, indent + 1, out);
                out.push_str(": ");
                p_expr(v, indent + 1, out);
            }
            out.push('}');
        }
        Expr::Lambda { params, body } => {
            out.push_str("func (");
            out.push_str(&params.join(", "));
            out.push_str(") {\n");
            p_stmts(body, indent + 1, out);
            p_indent(out, indent);
            out.push('}');
        }
        Expr::Template(parts) => {
            out.push('"');
            for part in parts {
                match part {
                    Expr::Str(s) => out.push_str(s),
                    e => {
                        out.push_str("${");
                        p_expr(e, indent + 1, out);
                        out.push('}');
                    }
                }
            }
            out.push('"');
        }
        Expr::Match { value, arms } => {
            out.push_str("match ");
            p_expr(value, indent, out);
            out.push_str(" {\n");
            for arm in arms {
                p_indent(out, indent + 1);
                out.push_str("case ");
                p_pattern(&arm.pattern, indent + 1, out);
                out.push_str(" => ");
                p_expr(&arm.body, indent + 1, out);
                out.push('\n');
            }
            p_indent(out, indent);
            out.push('}');
        }
    }
}

fn p_pattern(pattern: &Pattern, indent: usize, out: &mut String) {
    match pattern {
        Pattern::Literal(e) => p_expr(e, indent, out),
        Pattern::Binding(name) => out.push_str(name),
        Pattern::Wildcard => out.push('_'),
    }
}

fn op_symbol(op: &TokenKind) -> &'static str {
    match op {
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",
        TokenKind::Eq => "==",
        TokenKind::NotEq => "!=",
        TokenKind::Lt => "<",
        TokenKind::Gt => ">",
        TokenKind::Le => "<=",
        TokenKind::Ge => ">=",
        TokenKind::And => "and",
        TokenKind::Or => "or",
        TokenKind::Not => "not",
        _ => "?",
    }
}

fn repl() {
    let mut interpreter = Interpreter::new();
    let stdin = io::stdin();
    println!("Kinet REPL — gõ code, 'exit' để thoát");
    loop {
        print!("> ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() || line.trim() == "exit" {
            break;
        }
        match eval_line(&mut interpreter, &line) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}

fn eval_line(interpreter: &mut Interpreter, line: &str) -> Result<(), String> {
    let tokens = Lexer::new(line).tokenize()?;
    let stmts = Parser::new(tokens).parse()?;
    interpreter.run(&stmts)
}
