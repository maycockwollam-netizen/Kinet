//! Kinet — ngôn ngữ lập trình nhỏ do bạn tự viết, dành cho agent SDK.
//!
//! Cách dùng:
//!   cargo run             → mở REPL
//!   cargo run -- file.kinet → chạy file source
//!   cargo run -- -e "println(2 + 3)" → chạy code từ tham số

mod ast;
mod interpreter;
mod lexer;
mod parser;
mod token;

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        repl();
        ExitCode::SUCCESS
    } else if args[0] == "-e" {
        if args.len() < 2 {
            eprintln!("Cần code sau '-e'");
            return ExitCode::FAILURE;
        }
        match run_source(&args[1]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        let path = &args[0];
        match fs::read_to_string(path) {
            Ok(source) => match run_source(&source) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("Không đọc được file '{}': {}", path, e);
                ExitCode::FAILURE
            }
        }
    }
}

fn run_source(source: &str) -> Result<(), String> {
    let tokens = Lexer::new(source).tokenize()?;
    let stmts = Parser::new(tokens).parse()?;
    let mut interpreter = Interpreter::new();
    interpreter.run(&stmts)
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
