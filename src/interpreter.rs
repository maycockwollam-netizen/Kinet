//! Interpreter: chạy AST (tree-walking).
//!
//! Mô hình:
//!   Value  = giá trị runtime (số, chuỗi, bool, hàm, nil)
//!   Env    = phạm vi biến, có nối tới cha (cho closure/block)
//!   Flow   = tín hiệu điều khiển (return) trong khi chạy

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;

use indexmap::IndexMap;

use crate::ast::{Expr, Pattern, Stmt};
use crate::token::TokenKind;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    List(Vec<Value>),
    // Cặp (khóa, giá trị) — IndexMap: giữ thứ tự chèn NHƯNG tra cứu O(1).
    Map(IndexMap<Value, Value>),
    Func(FuncDef),
    Native(NativeFunc),
    Nil,
}

// Hàm builtin do Rust cài, nhận sẵn danh sách giá trị đối số.
#[derive(Debug, Clone, Copy)]
pub struct NativeFunc {
    pub name: &'static str,
    pub func: fn(&[Value]) -> Result<Value, String>,
}

// So sánh hàm chỉ dựa trên tên (closure chứa Env không so sánh được).
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => *a as f64 == *b,
            (Value::Float(a), Value::Int(b)) => *a == *b as f64,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => {
                // Thứ tự không quan trọng — so theo tập (khóa, giá trị).
                a.len() == b.len() && a.iter().all(|(k, v)| b.get(k) == Some(v))
            }
            (Value::Func(a), Value::Func(b)) => a.name == b.name,
            (Value::Native(a), Value::Native(b)) => a.name == b.name,
            (Value::Nil, Value::Nil) => true,
            _ => false,
        }
    }
}

impl Eq for Value {}

// Hash nhất quán với PartialEq (map băm theo tập để thứ tự không làm lệch).
impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        use std::collections::hash_map::DefaultHasher;
        match self {
            Value::Int(n) => {
                0u8.hash(state);
                n.hash(state);
            }
            Value::Float(n) => {
                // Chuẩn hóa -0.0 → 0.0 để nhất quán với ==
                let v = if *n == 0.0 { 0.0f64 } else { *n };
                8u8.hash(state);
                v.to_bits().hash(state);
            }
            Value::Str(s) => {
                1u8.hash(state);
                s.hash(state);
            }
            Value::Bool(b) => {
                2u8.hash(state);
                b.hash(state);
            }
            Value::Nil => 3u8.hash(state),
            Value::Func(f) => {
                4u8.hash(state);
                f.name.hash(state);
            }
            Value::Native(n) => {
                5u8.hash(state);
                n.name.hash(state);
            }
            Value::List(items) => {
                6u8.hash(state);
                items.len().hash(state);
                for it in items {
                    it.hash(state);
                }
            }
            Value::Map(entries) => {
                7u8.hash(state);
                // Băm từng cặp (khóa, giá trị), sắp xếp rồi gộp — độc lập thứ tự.
                let mut pair_hashes: Vec<u64> = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let mut hs = DefaultHasher::new();
                    k.hash(&mut hs);
                    v.hash(&mut hs);
                    pair_hashes.push(hs.finish());
                }
                pair_hashes.sort_unstable();
                pair_hashes.hash(state);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub closure: EnvRef,
}

type EnvRef = std::rc::Rc<std::cell::RefCell<Env>>;

#[derive(Debug, Clone, Default)]
pub struct Env {
    values: HashMap<String, Value>,
    parent: Option<EnvRef>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    fn child(parent: EnvRef) -> Self {
        Self {
            values: HashMap::new(),
            parent: Some(parent),
        }
    }

    fn define(&mut self, name: &str, value: Value) {
        self.values.insert(name.to_string(), value);
    }

    fn assign(&mut self, name: &str, value: Value) -> Result<(), String> {
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), value);
            Ok(())
        } else if let Some(parent) = &self.parent {
            parent.borrow_mut().assign(name, value)
        } else {
            Err(format!("Chưa khai báo biến '{}'", name))
        }
    }

    fn get(&self, name: &str) -> Result<Value, String> {
        if let Some(v) = self.values.get(name) {
            Ok(v.clone())
        } else if let Some(parent) = &self.parent {
            parent.borrow().get(name)
        } else {
            Err(format!("Chưa khai báo biến '{}'", name))
        }
    }
}

#[derive(Debug)]
enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

pub struct Interpreter {
    globals: EnvRef,
    call_depth: usize,
    // Ngân sách thực thi: đếm mỗi câu lệnh đã chạy, chống vòng lặp vô hạn.
    steps: u64,
    max_steps: u64,
    // Dòng nguồn của câu lệnh đang thực thi (cho báo lỗi + stack trace).
    current_line: Option<usize>,
    // Các file đã import thành công (đường dẫn đã chuẩn hóa).
    imported: HashSet<std::path::PathBuf>,
    // Các file đang trong quá trình import — phát hiện import vòng.
    loading: HashSet<std::path::PathBuf>,
}

//: Giới hạn độ sâu gọi hàm tránh stack overflow. Main chạy trên thread 64MB
//: (xem main.rs) nên con số này thoải mái cho đệ quy hợp lệ.
const MAX_CALL_DEPTH: usize = 2000;

// Marker đánh dấu lỗi đã có stack trace (tránh bọc trùng khi lỗi dâng lên).
const STACK_MARKER: &str = "  ↳ tại hàm ";
// Giới hạn số frame hiển thị để tránh phình khi đệ quy sâu.
const MAX_STACK_FRAMES: usize = 30;
// Ngân sách mặc định: số câu lệnh tối đa mỗi lần run() (chống vòng lặp vô hạn).
const DEFAULT_MAX_STEPS: u64 = 20_000_000;
// Giới hạn số phần tử tối đa mà range() được phép tạo (chống ăn hết RAM).
const MAX_RANGE_LEN: i64 = 2_000_000;
// Trần bộ nhớ cho chuỗi (ký tự) và list/map (phần tử) — chống phình RAM.
const MAX_STR_CHARS: usize = 64_000_000;
const MAX_LIST_LEN: usize = 2_000_000;

fn check_len(what: &str, len: usize, max: usize) -> Result<(), String> {
    if len > max {
        return Err(format!(
            "{} quá lớn ({} > giới hạn {}) — nghi ngờ phình bộ nhớ",
            what, len, max
        ));
    }
    Ok(())
}

/// Thêm tiền tố "Lỗi dòng N:" — nhà máy ngoài cùng (đã có dòng thì giữ).
fn annotate_line(line: usize, err: String) -> String {
    if err.starts_with("Lỗi dòng ") {
        err
    } else {
        format!("Lỗi dòng {}: {}", line, err)
    }
}

/// Thêm một frame (tên hàm + dòng gọi) vào cuối thông báo lỗi, nếu chưa đủ frame.
fn annotate_stack(mut err: String, fn_name: &str, line: Option<usize>) -> String {
    if err.matches(STACK_MARKER).count() >= MAX_STACK_FRAMES {
        return err;
    }
    err.push('\n');
    err.push_str(STACK_MARKER);
    err.push_str(fn_name);
    if let Some(l) = line {
        err.push_str(&format!(" (dòng {})", l));
    }
    err
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = std::rc::Rc::new(std::cell::RefCell::new(Env::new()));
        // Hàm dựng sẵn (native) có sẵn cho mọi chương trình.
        {
            let mut g = globals.borrow_mut();
            for (name, func) in [
                ("len", native_len as fn(&[Value]) -> Result<Value, String>),
                ("get", native_get as fn(&[Value]) -> Result<Value, String>),
                ("contains", native_contains as fn(&[Value]) -> Result<Value, String>),
                ("append", native_append as fn(&[Value]) -> Result<Value, String>),
                ("range", native_range as fn(&[Value]) -> Result<Value, String>),
                ("split", native_split as fn(&[Value]) -> Result<Value, String>),
                ("starts_with", native_starts_with as fn(&[Value]) -> Result<Value, String>),
                ("ends_with", native_ends_with as fn(&[Value]) -> Result<Value, String>),
                ("trim", native_trim as fn(&[Value]) -> Result<Value, String>),
                ("replace", native_replace as fn(&[Value]) -> Result<Value, String>),
                ("map_set", native_map_set as fn(&[Value]) -> Result<Value, String>),
                ("map_keys", native_map_keys as fn(&[Value]) -> Result<Value, String>),
                ("map_remove", native_map_remove as fn(&[Value]) -> Result<Value, String>),
                ("read_file", native_read_file as fn(&[Value]) -> Result<Value, String>),
                ("write_file", native_write_file as fn(&[Value]) -> Result<Value, String>),
                ("chars", native_chars as fn(&[Value]) -> Result<Value, String>),
                ("to_int", native_to_int as fn(&[Value]) -> Result<Value, String>),
                ("ord", native_ord as fn(&[Value]) -> Result<Value, String>),
                ("substring", native_substring as fn(&[Value]) -> Result<Value, String>),
                ("to_upper", native_to_upper as fn(&[Value]) -> Result<Value, String>),
                ("to_lower", native_to_lower as fn(&[Value]) -> Result<Value, String>),
                ("list_join", native_list_join as fn(&[Value]) -> Result<Value, String>),
                // map/filter là higher-order: intercept trong Interpreter::call.
                ("map", native_higher_order_stub as fn(&[Value]) -> Result<Value, String>),
                ("filter", native_higher_order_stub as fn(&[Value]) -> Result<Value, String>),
            ] {
                g.define(name, Value::Native(NativeFunc { name, func }));
            }
        }
        Self {
            globals,
            call_depth: 0,
            steps: 0,
            max_steps: DEFAULT_MAX_STEPS,
            current_line: None,
            imported: HashSet::new(),
            loading: HashSet::new(),
        }
    }

    pub fn run(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            match self.exec(stmt, self.globals.clone())? {
                Flow::Normal => {}
                Flow::Return(v) => {
                    return Err(format!(
                        "'return' bên ngoài hàm (giá trị {:?}) — không hợp lệ",
                        v
                    ))
                }
                Flow::Break => return Err("'break' ngoài vòng lặp".to_string()),
                Flow::Continue => return Err("'continue' ngoài vòng lặp".to_string()),
            }
        }
        Ok(())
    }

    // Chạy một block, trả về flow khi gặp return/break/continue.
    fn exec_block(&mut self, stmts: &[Stmt], env: EnvRef) -> Result<Flow, String> {
        for s in stmts {
            let flow = self.exec(s, env.clone())?;
            if !matches!(flow, Flow::Normal) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    /// Đếm một bước thực thi và kiểm tra ngân sách (chống vòng lặp vô hạn).
    fn tick(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > self.max_steps {
            return Err(format!(
                "Quá ngân sách thực thi ({} bước) — có thể là vòng lặp vô hạn",
                self.max_steps
            ));
        }
        Ok(())
    }

    fn exec(&mut self, stmt: &Stmt, env: EnvRef) -> Result<Flow, String> {
        // Ngân sách thực thi: chống vòng lặp vô hạn (while true v.v.).
        self.tick()?;
        match stmt {
            Stmt::Ann { line, inner } => {
                self.current_line = Some(*line);
                match self.exec(inner, env) {
                    Ok(flow) => Ok(flow),
                    Err(e) => Err(annotate_line(*line, e)),
                }
            }
            Stmt::Expr(e) => {
                self.eval(e, env)?;
                Ok(Flow::Normal)
            }
            Stmt::Let { name, value } => {
                let v = self.eval(value, env.clone())?;
                env.borrow_mut().define(name, v);
                Ok(Flow::Normal)
            }
            Stmt::Print(exprs) => {
                let text = self.join_printed(exprs, env)?;
                print!("{}", text);
                // print không xuống dòng — cần flush để kết quả hiện kịp khi chạy file.
                let _ = std::io::stdout().flush();
                Ok(Flow::Normal)
            }
            Stmt::Println(exprs) => {
                let text = self.join_printed(exprs, env)?;
                println!("{}", text);
                Ok(Flow::Normal)
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.eval(cond, env.clone())?;
                let branch = if self.truthy(&cond_value) {
                    then_branch
                } else {
                    else_branch
                };
                // Nhất quán với for/match: mỗi block if có scope riêng,
                // biến khai báo trong if không rò ra ngoài.
                let scope = std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                self.exec_block(branch, scope)
            }
            Stmt::While { cond, body } => {
                loop {
                    // Mỗi vòng lặp là một bước (kể cả body rỗng) — chống treo vô hạn.
                    self.tick()?;
                    let cond_value = self.eval(cond, env.clone())?;
                    if !self.truthy(&cond_value) {
                        break;
                    }
                    // Nhất quán với for: mỗi lần lặp có scope riêng.
                    let scope =
                        std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                    match self.exec_block(body, scope)? {
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Break => break,
                        Flow::Continue | Flow::Normal => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::For { var, iterable, body } => {
                let iter_value = self.eval(iterable, env.clone())?;
                let Value::List(items) = iter_value else {
                    return Err(format!(
                        "'for in' cần list (vd: range(0,5)), gặp {}",
                        self.to_string(&iter_value)
                    ));
                };
                for item in items {
                    // Biến lặp chỉ tồn tại trong body.
                    self.tick()?;
                    let scope =
                        std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                    scope.borrow_mut().define(var, item);
                    match self.exec_block(body, scope)? {
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Break => break,
                        Flow::Continue | Flow::Normal => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Import(path) => {
                // Chạy code file import ngay trong cùng global env (function/let
                // dùng được sau `import`). Tránh import lặp qua set đã chuẩn hóa.
                let resolved = resolve_sandbox(path)?;
                let canon = resolved.canonicalize().unwrap_or(resolved.clone());
                if self.imported.contains(&canon) {
                    return Ok(Flow::Normal);
                }
                if self.loading.contains(&canon) {
                    return Err(format!(
                        "import('{}'): phát hiện import vòng (A import B import A...)",
                        path
                    ));
                }
                self.loading.insert(canon.clone());
                let outcome = (|| -> Result<Flow, String> {
                    let src = std::fs::read_to_string(&canon)
                        .map_err(|e| format!("import('{}') thất bại: {}", path, e))?;
                    let tokens = crate::lexer::Lexer::new(&src).tokenize()?;
                    let stmts = crate::parser::Parser::new(tokens).parse()?;
                    match self.exec_block(&stmts, self.globals.clone())? {
                        Flow::Normal => Ok(Flow::Normal),
                        _ => Err(format!(
                            "import('{}'): file được import không được có return/break ở top-level",
                            path
                        )),
                    }
                })();
                // Luôn tháo khỏi 'loading' dù thành công hay lỗi → import lại được.
                self.loading.remove(&canon);
                match outcome {
                    // Chỉ coi là "đã nạp" khi thật sự thành công.
                    Ok(flow) => {
                        self.imported.insert(canon);
                        Ok(flow)
                    }
                    Err(e) => Err(format!("{} (từ file import '{}')", e, path)),
                }
            }
            Stmt::Try {
                body,
                catch_var,
                catch_body,
            } => {
                // Chỉ bắt lỗi runtime (Err); return/break/continue vẫn đi qua.
                match self.exec_block(body, env.clone()) {
                    Ok(flow) => Ok(flow),
                    Err(error_msg) => {
                        let scope =
                            std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                        if let Some(var) = catch_var {
                            scope.borrow_mut().define(var, Value::Str(error_msg));
                        }
                        self.exec_block(catch_body, scope)
                    }
                }
            }
            Stmt::Func {
                name,
                params,
                body,
            } => {
                let func = Value::Func(FuncDef {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: env.clone(),
                });
                env.borrow_mut().define(name, func);
                Ok(Flow::Normal)
            }
            Stmt::Return(e) => {
                let v = self.eval(e, env)?;
                Ok(Flow::Return(v))
            }
        }
    }

    fn eval(&mut self, expr: &Expr, env: EnvRef) -> Result<Value, String> {
        match expr {
            Expr::Number(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Lambda { params, body } => Ok(Value::Func(FuncDef {
                name: "<lambda>".to_string(),
                params: params.clone(),
                body: body.clone(),
                closure: env.clone(),
            })),
            Expr::Template(parts) => {
                let mut out = String::new();
                for part in parts {
                    let v = self.eval(part, env.clone())?;
                    out.push_str(&self.to_string(&v));
                    check_len("chuỗi template", out.chars().count(), MAX_STR_CHARS)?;
                }
                Ok(Value::Str(out))
            }
            Expr::Variable(name) => env.borrow().get(name),
            Expr::Assign { name, value } => {
                let v = self.eval(value, env.clone())?;
                env.borrow_mut().assign(name, v.clone())?;
                Ok(v)
            }
            Expr::Unary { op, expr } => {
                let v = self.eval(expr, env)?;
                match op {
                    TokenKind::Minus => match v {
                        Value::Int(n) => n
                            .checked_neg()
                            .map(Value::Int)
                            .ok_or_else(|| "Tràn số khi lấy '-'".to_string()),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err("Toán tử '-' cần số".to_string()),
                    },
                    TokenKind::Not => Ok(Value::Bool(!self.truthy(&v))),
                    _ => unreachable!(),
                }
            }
            Expr::Binary { left, op, right } => self.eval_binary(left, op, right, env),
            Expr::Call { callee, args } => {
                let func = self.eval(callee, env.clone())?;
                let mut arg_values = Vec::with_capacity(args.len());
                for a in args {
                    arg_values.push(self.eval(a, env.clone())?);
                }
                self.call(func, arg_values)
            }
            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.eval(it, env.clone())?);
                }
                Ok(Value::List(out))
            }
            Expr::Dict(entries) => {
                let mut out: IndexMap<Value, Value> = IndexMap::with_capacity(entries.len());
                for (k, v) in entries {
                    let key = self.eval(k, env.clone())?;
                    let value = self.eval(v, env.clone())?;
                    out.insert(key, value);
                }
                Ok(Value::Map(out))
            }
            Expr::Match { value, arms } => self.eval_match(value, arms, env),
        }
    }

    fn eval_match(&mut self, value: &Expr, arms: &[crate::ast::MatchArm], env: EnvRef) -> Result<Value, String> {
        let matched = self.eval(value, env.clone())?;
        for arm in arms {
            let pattern_matches = match &arm.pattern {
                Pattern::Wildcard => true,
                Pattern::Literal(lit) => {
                    let lit_value = self.eval(lit, env.clone())?;
                    lit_value == matched
                }
                Pattern::Binding(_) => true,
            };
            if pattern_matches {
                match &arm.pattern {
                    Pattern::Binding(name) => {
                        // Biến bắt chỉ tồn tại trong nhánh đó (scope riêng).
                        let scope =
                            std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                        scope.borrow_mut().define(name, matched.clone());
                        return self.eval(&arm.body, scope);
                    }
                    _ => return self.eval(&arm.body, env),
                }
            }
        }
        Err("Không có nhánh nào của match khớp (thiếu default?)".to_string())
    }

    fn eval_binary(
        &mut self,
        left: &Expr,
        op: &TokenKind,
        right: &Expr,
        env: EnvRef,
    ) -> Result<Value, String> {
        // Đánh giá lười cho and/or
        match op {
            TokenKind::And => {
                let lv = self.eval(left, env.clone())?;
                if !self.truthy(&lv) {
                    return Ok(Value::Bool(false));
                }
                let rv = self.eval(right, env.clone())?;
                return Ok(Value::Bool(self.truthy(&rv)));
            }
            TokenKind::Or => {
                let lv = self.eval(left, env.clone())?;
                if self.truthy(&lv) {
                    return Ok(Value::Bool(true));
                }
                let rv = self.eval(right, env.clone())?;
                return Ok(Value::Bool(self.truthy(&rv)));
            }
            _ => {}
        }

        let lv = self.eval(left, env.clone())?;
        let rv = self.eval(right, env)?;

        match op {
            // `+`: nếu có chuỗi thì nối; ngược lại số học (int/float).
            TokenKind::Plus => {
                if matches!(lv, Value::Str(_)) || matches!(rv, Value::Str(_)) {
                    let left = self.to_string(&lv);
                    let right = self.to_string(&rv);
                    let len = left.chars().count() + right.chars().count();
                    check_len("chuỗi nối", len, MAX_STR_CHARS)?;
                    Ok(Value::Str(format!("{}{}", left, right)))
                } else {
                    self.num_arith(&lv, &rv, op)
                }
            }
            TokenKind::Minus => self.num_arith(&lv, &rv, op),
            TokenKind::Star => self.num_arith(&lv, &rv, op),
            TokenKind::Slash => self.num_arith(&lv, &rv, op),
            TokenKind::Percent => self.num_arith(&lv, &rv, op),
            TokenKind::Eq => Ok(Value::Bool(lv == rv)),
            TokenKind::NotEq => Ok(Value::Bool(lv != rv)),
            TokenKind::Lt => self.cmp_values(&lv, &rv, op),
            TokenKind::Gt => self.cmp_values(&lv, &rv, op),
            TokenKind::Le => self.cmp_values(&lv, &rv, op),
            TokenKind::Ge => self.cmp_values(&lv, &rv, op),
            _ => unreachable!(),
        }
    }

    // Số học: int+int → int (checked), có float → float.
    fn num_arith(&self, l: &Value, r: &Value, op: &TokenKind) -> Result<Value, String> {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                let result = match op {
                    TokenKind::Plus => a
                        .checked_add(*b)
                        .ok_or_else(|| "Tràn số khi cộng".to_string())?,
                    TokenKind::Minus => a
                        .checked_sub(*b)
                        .ok_or_else(|| "Tràn số khi trừ".to_string())?,
                    TokenKind::Star => a
                        .checked_mul(*b)
                        .ok_or_else(|| "Tràn số khi nhân".to_string())?,
                    TokenKind::Slash => {
                        if *b == 0 {
                            return Err("Chia cho 0".to_string());
                        }
                        a.checked_div(*b)
                            .ok_or_else(|| "Tràn số khi chia".to_string())?
                    }
                    TokenKind::Percent => {
                        if *b == 0 {
                            return Err("Chia lấy dư cho 0".to_string());
                        }
                        a.checked_rem(*b)
                            .ok_or_else(|| "Tràn số khi lấy dư".to_string())?
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Int(result))
            }
            _ => {
                let a = self.as_f64(l, op)?;
                let b = self.as_f64(r, op)?;
                let result = match op {
                    TokenKind::Plus => a + b,
                    TokenKind::Minus => a - b,
                    TokenKind::Star => a * b,
                    TokenKind::Slash => {
                        if b == 0.0 {
                            return Err("Chia cho 0".to_string());
                        }
                        a / b
                    }
                    TokenKind::Percent => {
                        if b == 0.0 {
                            return Err("Chia lấy dư cho 0".to_string());
                        }
                        a % b
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Float(result))
            }
        }
    }

    fn as_f64(&self, v: &Value, op: &TokenKind) -> Result<f64, String> {
        match v {
            Value::Int(n) => Ok(*n as f64),
            Value::Float(f) => Ok(*f),
            _ => Err(format!("Toán tử '{}' cần số", self.op_symbol(op))),
        }
    }

    // So sánh số (int/float); chuỗi/khác kiểu không so bằng < > được.
    fn cmp_values(&self, l: &Value, r: &Value, op: &TokenKind) -> Result<Value, String> {
        let result = if let (Value::Int(a), Value::Int(b)) = (l, r) {
            match op {
                TokenKind::Lt => a < b,
                TokenKind::Gt => a > b,
                TokenKind::Le => a <= b,
                TokenKind::Ge => a >= b,
                _ => unreachable!(),
            }
        } else {
            let a = self.as_f64(l, op)?;
            let b = self.as_f64(r, op)?;
            match op {
                TokenKind::Lt => a < b,
                TokenKind::Gt => a > b,
                TokenKind::Le => a <= b,
                TokenKind::Ge => a >= b,
                _ => unreachable!(),
            }
        };
        Ok(Value::Bool(result))
    }

    fn op_symbol(&self, op: &TokenKind) -> &str {
        match op {
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::Le => "<=",
            TokenKind::Ge => ">=",
            _ => "?",
        }
    }

    fn call(&mut self, func: Value, args: Vec<Value>) -> Result<Value, String> {
        let def = match func {
            Value::Native(n) => {
                // map/filter là higher-order: cần self để gọi lại hàm → xử lý riêng.
                if n.name == "map" {
                    return self.native_map_call(args);
                }
                if n.name == "filter" {
                    return self.native_filter_call(args);
                }
                return (n.func)(&args);
            }
            Value::Func(def) => def,
            _ => return Err("Chỉ gọi được hàm, giá trị này không phải hàm".to_string()),
        };
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(format!(
                "Độ sâu gọi hàm vượt giới hạn {} (có thể đệ quy vô hạn)",
                MAX_CALL_DEPTH
            ));
        }
        if def.params.len() != args.len() {
            return Err(format!(
                "Hàm '{}' cần {} đối số, nhận {}",
                def.name,
                def.params.len(),
                args.len()
            ));
        }

        // Môi trường mới có cha là closure của hàm (hỗ trợ biến non-local).
        let fn_name = def.name.clone();
        // Ghi dòng gọi TRƯỚC khi chạy thân hàm (frame chỉ đúng chỗ gọi).
        let call_site_line = self.current_line;
        let env = std::rc::Rc::new(std::cell::RefCell::new(Env::child(def.closure.clone())));
        for (param, arg) in def.params.iter().zip(args) {
            env.borrow_mut().define(param, arg);
        }
        self.call_depth += 1;
        let result = self.exec_block(&def.body, env);
        self.call_depth -= 1;
        // Khi hàm gặp lỗi, đính kèm tên hàm + dòng gọi vào chuỗi lỗi (stack trace).
        let flow = result.map_err(|e| annotate_stack(e, &fn_name, call_site_line))?;
        match flow {
            Flow::Return(v) => Ok(v),
            Flow::Break | Flow::Continue => Err(format!(
                "'break'/'continue' không hợp lệ trong hàm '{}'",
                def.name
            )),
            Flow::Normal => Ok(Value::Nil),
        }
    }

    /// map(list, hàm) — áp hàm lên từng phần tử, trả list kết quả.
    fn native_map_call(&mut self, args: Vec<Value>) -> Result<Value, String> {
        if args.len() != 2 {
            return Err("map() cần 2 đối số (list, hàm)".to_string());
        }
        let Value::List(items) = &args[0] else {
            return Err("map() cần list ở đối số đầu".to_string());
        };
        let f = args[1].clone();
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(self.call(f.clone(), vec![item.clone()])?);
            check_len("map() kết quả", out.len(), MAX_LIST_LEN)?;
        }
        Ok(Value::List(out))
    }

    /// filter(list, hàm) — giữ các phần tử mà hàm trả về "đúng".
    fn native_filter_call(&mut self, args: Vec<Value>) -> Result<Value, String> {
        if args.len() != 2 {
            return Err("filter() cần 2 đối số (list, hàm)".to_string());
        }
        let Value::List(items) = &args[0] else {
            return Err("filter() cần list ở đối số đầu".to_string());
        };
        let f = args[1].clone();
        let mut out = Vec::new();
        for item in items {
            let result = self.call(f.clone(), vec![item.clone()])?;
            if self.truthy(&result) {
                out.push(item.clone());
                check_len("filter() kết quả", out.len(), MAX_LIST_LEN)?;
            }
        }
        Ok(Value::List(out))
    }

    // Chuẩn Python: 0, chuỗi rỗng, list rỗng là falsy — quan trọng cho agent
    // đọc exit code: `if exit_code` phải đúng nghĩa "code khác 0".
    fn truthy(&self, v: &Value) -> bool {
        match v {
            Value::Bool(b) => *b,
            Value::Nil => false,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Map(entries) => !entries.is_empty(),
            _ => true,
        }
    }

    fn to_string(&self, v: &Value) -> String {
        match v {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Str(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::List(items) => {
                let inner = items
                    .iter()
                    .map(|it| self.to_string(it))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{}]", inner)
            }
            Value::Map(entries) => {
                let inner = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.to_string(k), self.to_string(v)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{}}}", inner)
            }
            Value::Func(f) => format!("<hàm {}>", f.name),
            Value::Native(n) => format!("<hàm {}>", n.name),
            Value::Nil => "nil".to_string(),
        }
    }

    fn join_printed(&mut self, exprs: &[Expr], env: EnvRef) -> Result<String, String> {
        let mut out = String::new();
        for e in exprs {
            let v = self.eval(e, env.clone())?;
            out.push_str(&self.to_string(&v));
        }
        Ok(out)
    }
}

// ---------- Hàm native (builtin) ----------

fn native_higher_order_stub(_args: &[Value]) -> Result<Value, String> {
    Err("map/filter là builtin cần interpreter, không gọi trực tiếp".to_string())
}

fn native_len(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("len() cần 1 đối số".to_string());
    }
    match &args[0] {
        Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
        Value::List(items) => Ok(Value::Int(items.len() as i64)),
        Value::Map(entries) => Ok(Value::Int(entries.len() as i64)),
        _ => Err("len() chỉ áp dụng cho chuỗi, list hoặc map".to_string()),
    }
}

fn native_get(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("get() cần 2 đối số (list/map, chỉ số/khóa)".to_string());
    }
    match &args[0] {
        Value::List(items) => {
            let Value::Int(i) = args[1] else {
                return Err("get(list, i) cần số nguyên ở đối số thứ 2".to_string());
            };
            if i < 0 || i as usize >= items.len() {
                return Err(format!(
                    "get(): chỉ số {} ngoài phạm vi (len={})",
                    i,
                    items.len()
                ));
            }
            Ok(items[i as usize].clone())
        }
        Value::Map(entries) => entries.get(&args[1]).cloned().ok_or_else(|| {
            "get(map, khóa) — không có khóa này trong map".to_string()
        }),
        _ => Err("get() cần list hoặc map ở đối số đầu".to_string()),
    }
}

fn native_contains(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("contains() cần 2 đối số (list/chuỗi/map, x)".to_string());
    }
    match &args[0] {
        Value::List(items) => Ok(Value::Bool(items.contains(&args[1]))),
        Value::Str(s) => {
            let Value::Str(sub) = &args[1] else {
                return Err("contains() trên chuỗi cần chuỗi con".to_string());
            };
            Ok(Value::Bool(s.contains(sub)))
        }
        Value::Map(entries) => Ok(Value::Bool(entries.contains_key(&args[1]))),
        _ => Err("contains() cần list, chuỗi hoặc map ở đối số đầu".to_string()),
    }
}

fn native_map_set(args: &[Value]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err("map_set() cần 3 đối số (map, khóa, giá trị)".to_string());
    }
    let Value::Map(entries) = &args[0] else {
        return Err("map_set() cần map ở đối số đầu".to_string());
    };
    check_len("map", entries.len() + 1, MAX_LIST_LEN)?;
    let mut out = entries.clone();
    out.insert(args[1].clone(), args[2].clone());
    Ok(Value::Map(out))
}

fn native_map_keys(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("map_keys() cần 1 đối số".to_string());
    }
    let Value::Map(entries) = &args[0] else {
        return Err("map_keys() cần map ở đối số đầu".to_string());
    };
    Ok(Value::List(entries.keys().cloned().collect()))
}

fn native_chars(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("chars() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("chars() cần chuỗi".to_string());
    };
    check_len("chars() kết quả", s.chars().count(), MAX_LIST_LEN)?;
    Ok(Value::List(
        s.chars().map(|c| Value::Str(c.to_string())).collect(),
    ))
}

fn native_to_int(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("to_int() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("to_int() cần chuỗi".to_string());
    };
    s.trim().parse::<i64>().map(Value::Int).map_err(|_| {
        format!("to_int(): '{}' không phải số nguyên", s)
    })
}

fn native_substring(args: &[Value]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err("substring() cần 3 đối số (chuỗi, bắt đầu, độ dài)".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("substring() cần chuỗi ở đối số đầu".to_string());
    };
    let Value::Int(start) = args[1] else {
        return Err("substring() cần số nguyên (vị trí bắt đầu)".to_string());
    };
    let Value::Int(len) = args[2] else {
        return Err("substring() cần số nguyên (độ dài)".to_string());
    };
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    if start < 0 || len < 0 || start > n {
        return Err(format!(
            "substring(): vị trí {} ngoài phạm vi (len={})",
            start, n
        ));
    }
    // saturating_add tránh tràn số khi len rất lớn.
    let end = start.saturating_add(len).min(n);
    Ok(Value::Str(chars[start as usize..end as usize].iter().collect()))
}

fn native_to_upper(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("to_upper() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("to_upper() cần chuỗi".to_string());
    };
    Ok(Value::Str(s.to_uppercase()))
}

fn native_to_lower(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("to_lower() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("to_lower() cần chuỗi".to_string());
    };
    Ok(Value::Str(s.to_lowercase()))
}

fn native_list_join(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("list_join() cần 2 đối số (list, dấu phân tách)".to_string());
    }
    let Value::List(items) = &args[0] else {
        return Err("list_join() cần list ở đối số đầu".to_string());
    };
    let Value::Str(sep) = &args[1] else {
        return Err("list_join() cần dấu phân tách là chuỗi".to_string());
    };
    let strings: Vec<String> = items.iter().map(stringify_value).collect();
    Ok(Value::Str(strings.join(sep)))
}

fn native_ord(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("ord() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("ord() cần chuỗi 1 ký tự".to_string());
    };
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(Value::Int(c as i64)),
        (None, _) => Err("ord(): chuỗi rỗng".to_string()),
        (Some(_), Some(_)) => Err(format!(
            "ord(): cần đúng 1 ký tự, nhận '{}' ({} ký tự)",
            s,
            s.chars().count()
        )),
    }
}

fn native_map_remove(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("map_remove() cần 2 đối số (map, khóa)".to_string());
    }
    let Value::Map(entries) = &args[0] else {
        return Err("map_remove() cần map ở đối số đầu".to_string());
    };
    let mut out = entries.clone();
    out.shift_remove(&args[1]);
    Ok(Value::Map(out))
}

fn native_read_file(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("read_file() cần 1 đối số (đường dẫn)".to_string());
    }
    let Value::Str(path) = &args[0] else {
        return Err("read_file() cần chuỗi đường dẫn".to_string());
    };
    let resolved = resolve_sandbox(path)?;
    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| format!("read_file('{}') thất bại: {}", path, e))?;
    Ok(Value::Str(content))
}

fn native_write_file(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("write_file() cần 2 đối số (đường dẫn, nội dung)".to_string());
    }
    let Value::Str(path) = &args[0] else {
        return Err("write_file() cần chuỗi đường dẫn".to_string());
    };
    let Value::Str(content) = &args[1] else {
        return Err("write_file() cần nội dung là chuỗi".to_string());
    };
    let resolved = resolve_sandbox(path)?;
    std::fs::write(&resolved, content)
        .map_err(|e| format!("write_file('{}') thất bại: {}", path, e))?;
    Ok(Value::Bool(true))
}

fn native_range(args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("range() cần 2-3 đối số (start, end[, step])".to_string());
    }
    let start = expect_int(&args[0], "range")?;
    let end = expect_int(&args[1], "range")?;
    let step = if args.len() == 3 {
        expect_int(&args[2], "range")?
    } else {
        1
    };
    if step == 0 {
        return Err("range(): step không được bằng 0".to_string());
    }
    // Ước lượng số phần tử để chặn range quá lớn (tránh ăn hết RAM).
    // Dùng checked_sub/checked_neg tránh tràn khi start/end cực trị.
    let count: i64 = if step > 0 && start < end {
        match end.checked_sub(start) {
            Some(delta) => match delta.checked_add(step - 1) {
                Some(d) => d / step,
                None => i64::MAX,
            },
            None => i64::MAX,
        }
    } else if step < 0 && start > end {
        match (start.checked_sub(end), step.checked_neg()) {
            (Some(delta), Some(abs)) if abs >= 1 => {
                delta.saturating_add(abs - 1) / abs
            }
            _ => i64::MAX,
        }
    } else {
        0
    };
    if count > MAX_RANGE_LEN {
        return Err(format!(
            "range(): quá lớn (~{} phần tử, giới hạn {}) — nghi ngờ sai phạm vi",
            count, MAX_RANGE_LEN
        ));
    }

    let mut out = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < end {
            out.push(Value::Int(i));
            // checked_add để tránh tràn số khi end gần i64::MAX.
            match i.checked_add(step) {
                Some(next) => i = next,
                None => break,
            }
        }
    } else {
        while i > end {
            out.push(Value::Int(i));
            match i.checked_add(step) {
                Some(next) => i = next,
                None => break,
            }
        }
    }
    Ok(Value::List(out))
}

fn native_split(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("split() cần 2 đối số (chuỗi, dấu phân tách)".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("split() cần chuỗi ở đối số đầu".to_string());
    };
    let Value::Str(sep) = &args[1] else {
        return Err("split() cần chuỗi phân tách ở đối số thứ 2".to_string());
    };
    if sep.is_empty() {
        // Rust split("") sinh ["a","b"] kèm 2 phần tử rỗng — hầu như luôn là
        // lỗi người dùng, nên báo rõ thay vì cho kết quả lạ.
        return Err("split(): dấu phân tách không được rỗng".to_string());
    }
    let parts: Vec<Value> = s
        .split(sep)
        .map(|p| Value::Str(p.to_string()))
        .collect();
    check_len("split() kết quả", parts.len(), MAX_LIST_LEN)?;
    Ok(Value::List(parts))
}

fn native_starts_with(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("starts_with() cần 2 đối số (chuỗi, tiền tố)".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("starts_with() cần chuỗi ở đối số đầu".to_string());
    };
    let Value::Str(prefix) = &args[1] else {
        return Err("starts_with() cần tiền tố là chuỗi".to_string());
    };
    Ok(Value::Bool(s.starts_with(prefix)))
}

fn native_ends_with(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("ends_with() cần 2 đối số (chuỗi, hậu tố)".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("ends_with() cần chuỗi ở đối số đầu".to_string());
    };
    let Value::Str(suffix) = &args[1] else {
        return Err("ends_with() cần hậu tố là chuỗi".to_string());
    };
    Ok(Value::Bool(s.ends_with(suffix)))
}

fn native_trim(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("trim() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("trim() cần chuỗi".to_string());
    };
    Ok(Value::Str(s.trim().to_string()))
}

fn native_replace(args: &[Value]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err("replace() cần 3 đối số (chuỗi, chuỗi cũ, chuỗi mới)".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("replace() cần chuỗi ở đối số đầu".to_string());
    };
    let Value::Str(from) = &args[1] else {
        return Err("replace() cần chuỗi cũ là chuỗi".to_string());
    };
    let Value::Str(to) = &args[2] else {
        return Err("replace() cần chuỗi mới là chuỗi".to_string());
    };
    Ok(Value::Str(s.replace(from, to)))
}

fn expect_int(v: &Value, fname: &str) -> Result<i64, String> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("{}() cần đối số là số nguyên", fname)),
    }
}

/// Chọn thư mục gốc cho sandbox file I/O.
///
/// Mặc định = thư mục hiện hành (an toàn). Đặt `KINET_SANDBOX_ROOT=/đường/dẫn`
/// để chỉ định root khác. Đặt `KINET_SANDBOX=none` để tắt sandbox (chỉ khi
/// chạy code đáng tin cậy).
fn sandbox_root() -> Result<std::path::PathBuf, String> {
    if let Ok(root) = std::env::var("KINET_SANDBOX_ROOT") {
        return Ok(std::path::PathBuf::from(root));
    }
    std::env::current_dir()
        .map_err(|e| format!("Không xác định được thư mục hiện hành: {}", e))
}

/// Giải quyết đường dẫn cho read_file/write_file/import — an toàn với symlink.
///
/// 1) `..`/đường tuyệt đối được chuẩn hóa theo chuỗi (chặn thoát theo ký pháp).
/// 2) canonicalize (giải symlink) rồi kiểm tra vẫn nằm trong root — chống
///    symlink trỏ ra ngoài. File chưa tồn tại (ghi mới) thì canonicalize tổ
///    tiên còn tồn tại gần nhất, vẫn phải nằm trong root.
fn resolve_sandbox(raw: &str) -> Result<std::path::PathBuf, String> {
    // Cho phép tắt sandbox hoàn toàn khi chạy code đáng tin cậy.
    if std::env::var("KINET_SANDBOX").as_deref() == Ok("none") {
        return Ok(std::path::PathBuf::from(raw));
    }

    let root = sandbox_root()?;
    let root_canon = root.canonicalize().map_err(|_| {
        format!("sandbox root '{}' không tồn tại/hợp lệ", root.display())
    })?;

    // Bước 1: chuẩn hóa theo chuỗi, chặn `..` thoát root.
    let lex = lexical_join(&root, raw)?;

    // Bước 2: giải symlink của (đường dẫn hoặc tổ tiên gần nhất) rồi kiểm tra trong root.
    let canon = canonicalize_loose(&lex);
    if !canon.starts_with(&root_canon) {
        return Err(format!(
            "Đường dẫn '{}' thoát khỏi sandbox '{}' (symlink hoặc ..)",
            raw,
            root_canon.display()
        ));
    }
    Ok(canon)
}

fn lexical_join(root: &std::path::Path, raw: &str) -> Result<std::path::PathBuf, String> {
    let rel: &str = raw.strip_prefix('/').unwrap_or(raw);
    let mut out = root.to_path_buf();
    let mut depth: i64 = 0; // độ sâu tính từ root
    for comp in std::path::Path::new(rel).components() {
        match comp {
            std::path::Component::Normal(c) => {
                depth += 1;
                out.push(c);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // `..` ở độ sâu 0 nghĩa là muốn thoát khỏi root → chặn.
                if depth <= 0 {
                    return Err(format!(
                        "Không được vượt ra ngoài sandbox root '{}'",
                        root.display()
                    ));
                }
                depth -= 1;
                out.pop();
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err("Đường dẫn tuyệt đối bị chặn trong sandbox".to_string());
            }
        }
    }
    Ok(out)
}

/// canonicalize nếu tồn tại; nếu không, canonicalize tổ tiên gần nhất rồi
/// nối lại phần đuôi còn lại (dùng cho write vào file chưa có).
fn canonicalize_loose(path: &std::path::Path) -> std::path::PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        match cur.canonicalize() {
            Ok(c) => {
                let mut out = c;
                for t in tail.iter().rev() {
                    out.push(t);
                }
                return out;
            }
            Err(_) => match cur.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    if !cur.pop() {
                        return path.to_path_buf();
                    }
                }
                None => return path.to_path_buf(),
            },
        }
    }
}

/// Chuyển một Value thành chuỗi hiển thị (dùng trong list_join và những chỗ
/// không có sẵn Interpreter::to_string).
/// Định dạng float: số nguyên hiện "3.0", còn lại hiện ngắn gọn (3.14).
fn format_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 {
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

fn stringify_value(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Float(f) => format_float(*f),
        Value::Str(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::List(items) => {
            let inner: Vec<String> = items.iter().map(stringify_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Map(entries) => {
            let inner: Vec<String> = entries
                .iter()
                .map(|(k, val)| format!("{}: {}", stringify_value(k), stringify_value(val)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Func(f) => format!("<hàm {}>", f.name),
        Value::Native(n) => format!("<hàm {}>", n.name),
        Value::Nil => "nil".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn program(src: &str) -> Vec<Stmt> {
        Parser::new(Lexer::new(src).tokenize().unwrap())
            .parse()
            .unwrap()
    }

    // Đường dẫn tạm nằm TRONG sandbox mặc định (thư mục hiện hành).
    fn sandbox_tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join("test_tmp");
        std::fs::create_dir_all(&dir).expect("tạo target/test_tmp");
        dir.join(name)
    }

    // Chạy một closure sinh ra Result<(), String> trên thread stack 64MB (giống
    // production) để đệ quy sâu không tràn stack thread test.
    fn run_on_big_stack<F>(f: F) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        let handle = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .expect("spawn thread lớn thất bại");
        handle.join().expect("thread panicked")
    }

    fn run(src: &str) -> Result<(), String> {
        let mut it = Interpreter::new();
        it.run(&program(src))
    }

    fn run_and_get(src: &str, var: &str) -> Value {
        let mut it = Interpreter::new();
        it.run(&program(src)).unwrap();
        it.globals.borrow().get(var).unwrap()
    }

    #[test]
    fn arithmetic_follows_precedence() {
        assert_eq!(run_and_get("let x = 2 + 3 * 4;", "x"), Value::Int(14));
        assert_eq!(run_and_get("let x = (2 + 3) * 4;", "x"), Value::Int(20));
        assert_eq!(run_and_get("let x = 7 % 3;", "x"), Value::Int(1));
    }

    #[test]
    fn string_concat() {
        assert_eq!(
            run_and_get(r#"let x = "a" + "b" + 1;"#, "x"),
            Value::Str("ab1".into())
        );
    }

    #[test]
    fn variables_and_assignment() {
        let src = "let a = 5; a = a + 2; let b = a * 2;";
        assert_eq!(run_and_get(src, "b"), Value::Int(14));
    }

    #[test]
    fn function_recursion() {
        let src = "func fact(n) { if n <= 1 { return 1; } return n * fact(n - 1); } let x = fact(5);";
        assert_eq!(run_and_get(src, "x"), Value::Int(120));
    }

    #[test]
    fn function_closure_reads_outer() {
        let src = "let base = 10; func add(x) { return base + x; } let y = add(5);";
        assert_eq!(run_and_get(src, "y"), Value::Int(15));
    }

    #[test]
    fn while_loop() {
        let src = "let total = 0; let i = 1; while i <= 5 { total = total + i; i = i + 1; }";
        assert_eq!(run_and_get(src, "total"), Value::Int(15));
    }

    #[test]
    fn for_with_break() {
        let src = "let last = 0; for i in range(0, 100) { if i == 5 { break; } last = i; }";
        assert_eq!(run_and_get(src, "last"), Value::Int(4));
    }

    #[test]
    fn for_with_continue() {
        let src = "let n = 0; for i in range(0, 5) { if i == 2 { continue; } n = n + 1; }";
        assert_eq!(run_and_get(src, "n"), Value::Int(4));
    }

    #[test]
    fn for_over_list() {
        let src = r#"let s = ""; for x in ["a", "b", "c"] { s = s + x; }"#;
        assert_eq!(run_and_get(src, "s"), Value::Str("abc".into()));
    }

    #[test]
    fn match_selects_by_ordinal() {
        let src = r#"let r = match "ok" { case "ok" => 1; case e => 2; };"#;
        assert_eq!(run_and_get(src, "r"), Value::Int(1));
    }

    #[test]
    fn match_binding_captures_value() {
        let src = r#"let r = match "xyz" { case "ok" => "ok"; case e => "got:" + e; };"#;
        assert_eq!(run_and_get(src, "r"), Value::Str("got:xyz".into()));
    }

    #[test]
    fn list_builtins() {
        let src = "let n = [1, 2, 3]; let l = len(n); let h = contains(n, 2); let ap = append(n, 4); let g = get(ap, 3);";
        assert_eq!(run_and_get(src, "l"), Value::Int(3));
        assert_eq!(run_and_get(src, "h"), Value::Bool(true));
        assert_eq!(
            run_and_get(src, "ap"),
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)])
        );
        assert_eq!(run_and_get(src, "g"), Value::Int(4));
    }

    #[test]
    fn string_functions() {
        let src = r#"let a = starts_with("run: x", "run:");
            let b = ends_with("file.kinet", ".kinet");
            let c = trim("  hi  ");
            let d = replace("a-b-c", "-", "+");
            let e = get(split("x,y,z", ","), 2);
            let f = contains("xin chao", "chao");"#;
        assert_eq!(run_and_get(src, "a"), Value::Bool(true));
        assert_eq!(run_and_get(src, "b"), Value::Bool(true));
        assert_eq!(run_and_get(src, "c"), Value::Str("hi".into()));
        assert_eq!(run_and_get(src, "d"), Value::Str("a+b+c".into()));
        assert_eq!(run_and_get(src, "e"), Value::Str("z".into()));
        assert_eq!(run_and_get(src, "f"), Value::Bool(true));
    }

    #[test]
    fn range_with_step() {
        assert_eq!(
            run_and_get("let r = range(5, 0, -2);", "r"),
            Value::List(vec![Value::Int(5), Value::Int(3), Value::Int(1)])
        );
    }

    #[test]
    fn divide_by_zero_errors() {
        let err = run("let x = 1 / 0;").unwrap_err();
        assert!(err.contains("Chia cho 0"));
    }

    #[test]
    fn undefined_variable_errors() {
        let err = run("println(unknown_var);").unwrap_err();
        assert!(err.contains("Chưa khai báo biến"));
    }

    #[test]
    fn break_outside_loop_errors() {
        let err = run("break;").unwrap_err();
        assert!(err.contains("ngoài vòng lặp"));
    }

    #[test]
    fn match_without_default_errors_when_no_match() {
        let err = run(r#"let r = match "c" { case "a" => 1; case "b" => 2; };"#).unwrap_err();
        assert!(err.contains("khớp"));
    }

    #[test]
    fn for_over_non_list_errors() {
        let err = run("for x in 5 { }").unwrap_err();
        assert!(err.contains("cần list"));
    }

    // ----- Regression: bug tìm được trong audit -----

    #[test]
    fn integer_overflow_is_error_not_panic() {
        let err = run("let x = 9223372036854775807 + 1;").unwrap_err();
        assert!(err.contains("Tràn số"));
        let err = run("let x = 9223372036854775807 * 2;").unwrap_err();
        assert!(err.contains("Tràn số khi nhân"));
    }

    #[test]
    fn division_by_minus_one_overflow_is_error() {
        // 0 - 9223372036854775808 không viết được literal vì số quá lớn,
        // nhưng MIN đạt được qua phép tính: dùng checked ops, kết quả tràn.
        let err = run("let x = -9223372036854775807 - 1; let y = x / 0;").unwrap_err();
        assert!(err.contains("Chia cho 0"));
    }

    #[test]
    fn deep_recursion_hits_limit_not_stack_overflow() {
        // Chạy trên thread stack lớn để giới hạn MAX_CALL_DEPTH (chứ không phải
        // stack OS) kích hoạt — nếu không, thread test stack nhỏ sẽ tràn trước.
        let src = "func f(n) { return f(n + 1); } f(0);".to_string();
        let run_result = run_on_big_stack(move || Interpreter::new().run(&program(&src)));
        let err = run_result.unwrap_err();
        assert!(err.contains("Độ sâu"));
    }

    #[test]
    fn error_reports_source_line() {
        let src = "let a = 1;\nlet b = 2;\nlet c = 1 / 0;\n";
        let err = run(src).unwrap_err();
        assert!(err.contains("Lỗi dòng 3: Chia cho 0"), "lỗi lạ: {}", err);
    }

    #[test]
    fn stack_trace_includes_call_lines() {
        let src = "func a() {\n    return b();\n}\nfunc b() {\n    let x = 1 / 0;\n    return x;\n}\na();\n";
        let err = run(src).unwrap_err();
        assert!(
            err.contains("tại hàm b (dòng "),
            "thiếu dòng trong frame b:\n{}",
            err
        );
        assert!(
            err.contains("tại hàm a (dòng "),
            "thiếu dòng trong frame a:\n{}",
            err
        );
    }

    #[test]
    fn stack_trace_shows_function_names() {
        let src = "func a() { return b(); } func b() { return c(); } func c() { let x = 1 / 0; } a();";
        let err = run(src).unwrap_err();
        assert!(err.contains("Chia cho 0"));
        assert!(err.contains("tại hàm c"), "thiếu frame c:\n{}", err);
        assert!(err.contains("tại hàm b"), "thiếu frame b:\n{}", err);
        assert!(err.contains("tại hàm a"), "thiếu frame a:\n{}", err);
    }

    #[test]
    fn stack_trace_capped_on_deep_recursion() {
        let src = "func r(n) { return r(n + 1); } r(0);".to_string();
        let outcome = run_on_big_stack(move || Interpreter::new().run(&program(&src)));
        let err = outcome.unwrap_err();
        assert!(err.contains("Độ sâu"));
        // Không được phình ra hàng nghìn frame.
        assert!(err.matches("tại hàm").count() <= 31, "quá nhiều frame:\n{}", err);
    }

    #[test]
    fn legit_deep_recursion_works_after_limit_raise() {
        // Kiểm tra đệ quy sâu hợp lệ (lớn hơn giới hạn cũ 128) vẫn chạy được.
        let src = "func f(n) { if n >= 1000 { return 0; } return f(n + 1); }".to_string();
        let run_result = run_on_big_stack(move || Interpreter::new().run(&program(&src)));
        run_result.unwrap();
    }

    #[test]
    fn split_empty_separator_errors() {
        let err = run(r#"let a = split("ab", "");"#).unwrap_err();
        assert!(err.contains("rỗng"));
    }

    // ----- map (M4.1) -----

    #[test]
    fn map_operations() {
        let src = r#"let m = { "a": 1, "b": 2 };
            let v = get(m, "a");
            let has = contains(m, "b");
            let not_has = contains(m, "z");
            let size = len(m);
            let keys = map_keys(m);
            let m2 = map_set(m, "c", 3);
            let size2 = len(m2);
            let rem = map_remove(m2, "a");
            let rem_size = len(rem);"#;
        assert_eq!(run_and_get(src, "v"), Value::Int(1));
        assert_eq!(run_and_get(src, "has"), Value::Bool(true));
        assert_eq!(run_and_get(src, "not_has"), Value::Bool(false));
        assert_eq!(run_and_get(src, "size"), Value::Int(2));
        assert_eq!(
            run_and_get(src, "keys"),
            Value::List(vec![Value::Str("a".into()), Value::Str("b".into())])
        );
        assert_eq!(run_and_get(src, "size2"), Value::Int(3));
        assert_eq!(run_and_get(src, "rem_size"), Value::Int(2));
    }

    #[test]
    fn map_nested() {
        let src = r#"let inner = { "x": 10 }; let outer = { "p": inner }; let v = get(get(outer, "p"), "x");"#;
        assert_eq!(run_and_get(src, "v"), Value::Int(10));
    }

    #[test]
    fn map_missing_key_errors() {
        let err = run(r#"let m = { "a": 1 }; let v = get(m, "zz");"#).unwrap_err();
        assert!(err.contains("khóa"));
    }

    #[test]
    fn map_dup_key_overwrites() {
        assert_eq!(
            run_and_get(r#"let m = { "a": 1, "a": 2 };"#, "m"),
            Value::Map(indexmap::IndexMap::from([(Value::Str("a".into()), Value::Int(2))]))
        );
    }

    // ----- file I/O (M4.2) -----

    #[test]
    fn file_io_roundtrip() {
        let path = sandbox_tmp("kinet_m4_io.tmp");
        let rel = format!(
            "target/test_tmp/{}",
            path.file_name().unwrap().to_str().unwrap()
        );
        run(&format!("write_file(\"{}\", \"hello demo\");", rel)).unwrap();
        let got = run_and_get(&format!("let s = read_file(\"{}\");", rel), "s");
        assert_eq!(got, Value::Str("hello demo".into()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_file_missing_errors() {
        let err = run(r#"let s = read_file("/no/such/file_kinet_xyz");"#).unwrap_err();
        assert!(err.contains("thất bại"));
    }

    #[test]
    fn dict_literal_creates_map() {
        let src = r#"let m = { "a": 1, "b": 2 };
            let v = get(m, "b");
            let has = contains(m, "a");
            let size = len(m);"#;
        assert_eq!(run_and_get(src, "v"), Value::Int(2));
        assert_eq!(run_and_get(src, "has"), Value::Bool(true));
        assert_eq!(run_and_get(src, "size"), Value::Int(2));
    }

    #[test]
    fn dict_literal_nested_and_empty() {
        assert_eq!(
            run_and_get(r#"let cfg = { "opts": { "retry": 3 } }; let r = get(get(cfg, "opts"), "retry");"#, "r"),
            Value::Int(3)
        );
        assert_eq!(
            run_and_get(r#"let e = {};"#, "e"),
            Value::Map(indexmap::IndexMap::new())
        );
    }

    #[test]
    fn block_comment_is_ignored() {
        assert_eq!(
            run_and_get("/* chú thích\nnhiều dòng */ let x = 5;", "x"),
            Value::Int(5)
        );
    }

    #[test]
    fn string_helpers() {
        let src = r#"let up = to_upper("ab"); let lo = to_lower("AB"); let sub = substring("hello", 1, 3); let j = list_join(["x", "y"], "-");"#;
        assert_eq!(run_and_get(src, "up"), Value::Str("AB".into()));
        assert_eq!(run_and_get(src, "lo"), Value::Str("ab".into()));
        assert_eq!(run_and_get(src, "sub"), Value::Str("ell".into()));
        assert_eq!(run_and_get(src, "j"), Value::Str("x-y".into()));
    }

    #[test]
    fn substring_out_of_range_errors() {
        let err = run(r#"let s = substring("abc", 9, 1);"#).unwrap_err();
        assert!(err.contains("ngoài phạm vi"));
    }

    // ----- Hồi quy từ audit an toàn/sai sót -----

    #[test]
    fn list_join_uses_kinet_strings_not_debug() {
        assert_eq!(
            run_and_get(r#"let j = list_join([1, 2, 3], ",");"#, "j"),
            Value::Str("1,2,3".into())
        );
        assert_eq!(
            run_and_get(r#"let j = list_join(["a", 1, true], "-");"#, "j"),
            Value::Str("a-1-true".into())
        );
    }

    #[test]
    fn ord_rejects_multichar() {
        let err = run(r#"let c = ord("abc");"#).unwrap_err();
        assert!(err.contains("đúng 1 ký tự"));
    }

    #[test]
    fn substring_does_not_overflow() {
        // len khổng lồ không được tràn số hay panic — chỉ cắt tới hết chuỗi.
        assert_eq!(
            run_and_get(r#"let s = substring("abc", 1, 9223372036854775807);"#, "s"),
            Value::Str("bc".into())
        );
    }

    #[test]
    fn range_near_max_no_overflow() {
        // Không được panic khi i += step chạm i64::MAX.
        let len = run_and_get(
            "let n = len(range(9223372036854775806, 9223372036854775807));",
            "n",
        );
        assert_eq!(len, Value::Int(1));
    }

    #[test]
    fn range_too_big_rejected() {
        let err = run("let r = range(0, 10000000);").unwrap_err();
        assert!(err.contains("quá lớn"));
    }

    #[test]
    fn execution_budget_stops_infinite_loop() {
        let mut it = Interpreter::new();
        it.max_steps = 200;
        let err = it.run(&program("while true { }")).unwrap_err();
        assert!(err.contains("ngân sách"));
    }

    #[test]
    fn if_block_does_not_leak_variable() {
        let err = run("if true { let x = 5; } println(x);").unwrap_err();
        assert!(err.contains("Chưa khai báo biến"));
    }

    #[test]
    fn while_block_does_not_leak_variable() {
        let src = "let s = 0; while s < 2 { let t = 1; s = s + 1; } println(t);";
        let err = run(src).unwrap_err();
        assert!(err.contains("Chưa khai báo biến"));
    }

    #[test]
    fn while_can_still_mutate_outer_via_assignment() {
        // Gán (không khai báo let) vào biến ngoài vẫn hoạt động trong scope con.
        assert_eq!(
            run_and_get("let total = 0; let i = 0; while i < 5 { total = total + i; i = i + 1; }", "total"),
            Value::Int(10)
        );
    }

    #[test]
    fn bare_return_returns_nil() {
        let src = "func f() { return; } let v = f();";
        assert_eq!(run_and_get(src, "v"), Value::Nil);
    }

    // ----- Template / Lambda / Import / higher-order -----

    #[test]
    fn template_interpolates_expressions() {
        let src = r#"let name = "An"; let s = "Xin chào ${name}, có ${1 + 2} việc";"#;
        assert_eq!(
            run_and_get(src, "s"),
            Value::Str("Xin chào An, có 3 việc".into())
        );
    }

    #[test]
    fn template_with_nested_call() {
        let src = r#"let s = "len=${len(\"abc\")}";"#;
        assert_eq!(run_and_get(src, "s"), Value::Str("len=3".into()));
    }

    #[test]
    fn template_unclosed_errors() {
        // Lỗi này xảy ra ở giai đoạn parse (tách ${...}), phải parse trực tiếp.
        let tokens = Lexer::new(r#"let s = "chưa đóng ${name";"#).tokenize().unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.contains("chưa đóng"));
    }

    #[test]
    fn lambda_is_assignable_and_callable() {
        let src = "let d = func(x) { return x * 2; }; let v = d(21);";
        assert_eq!(run_and_get(src, "v"), Value::Int(42));
    }

    #[test]
    fn higher_order_apply_via_user_func() {
        let src = "func apply(f, x) { return f(x); } let v = apply(func(n) { return n + 1; }, 41);";
        assert_eq!(run_and_get(src, "v"), Value::Int(42));
    }

    #[test]
    fn native_map_with_lambda() {
        assert_eq!(
            run_and_get(
                "let r = map([1, 2, 3], func(x) { return x * x; });",
                "r"
            ),
            Value::List(vec![Value::Int(1), Value::Int(4), Value::Int(9)])
        );
    }

    #[test]
    fn native_filter_with_lambda() {
        assert_eq!(
            run_and_get(
                "let r = filter([1, 2, 3, 4], func(x) { return x % 2 == 0; });",
                "r"
            ),
            Value::List(vec![Value::Int(2), Value::Int(4)])
        );
    }

    #[test]
    fn import_runs_library_file() {
        let lib = sandbox_tmp("kinet_import_lib.kinet");
        std::fs::write(&lib, "func double(x) { return x * 2; }\n").unwrap();
        let rel = "target/test_tmp/kinet_import_lib.kinet";
        let src = format!("import \"{}\"; let v = double(21);", rel);
        assert_eq!(run_and_get(&src, "v"), Value::Int(42));
        let _ = std::fs::remove_file(&lib);
    }

    #[test]
    fn import_is_idempotent() {
        let lib = sandbox_tmp("kinet_import_lib2.kinet");
        std::fs::write(&lib, "let X = 7;    // chỉ đọc 1 lần\n").unwrap();
        let rel = "target/test_tmp/kinet_import_lib2.kinet";
        let src = format!("import \"{}\"; import \"{}\"; let v = X;", rel, rel);
        assert_eq!(run_and_get(&src, "v"), Value::Int(7));
        let _ = std::fs::remove_file(&lib);
    }

    // ----- Hồi quy bảo mật: sandbox mặc định, symlink, template escape, bộ nhớ, import -----

    #[test]
    fn sandbox_default_blocks_parent_escape() {
        // Mặc định root = thư mục hiện hành; '..' không thoát được.
        let err = run(r#"let x = write_file("../escape.txt", "x");"#).unwrap_err();
        assert!(
            err.contains("ngoài sandbox") || err.contains("Không được vượt"),
            "lỗi lạ: {}",
            err
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_blocked() {
        let outside = std::env::temp_dir().join("kinet_secret.txt");
        std::fs::write(&outside, "secret").unwrap();
        let link = std::env::current_dir()
            .unwrap()
            .join("target/test_tmp/evil_link.txt");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err =
            run(r#"let s = read_file("target/test_tmp/evil_link.txt");"#).unwrap_err();
        assert!(
            err.contains("thoát") || err.contains("sandbox"),
            "lỗi lạ: {}",
            err
        );
        let _ = std::fs::remove_file(&link);
        std::fs::remove_file(&outside).ok();
    }

    #[test]
    fn template_escape_keeps_literal() {
        assert_eq!(
            run_and_get(r#"let s = "a \${b} c";"#, "s"),
            Value::Str("a ${b} c".into())
        );
        assert_eq!(
            run_and_get(r#"let s = "\${x}";"#, "s"),
            Value::Str("${x}".into())
        );
    }

    #[test]
    fn string_concat_capped() {
        // `s = s + s` không được phình RAM vô hạn — chạm trần thì lỗi.
        let err = run("let s = \"a\"; let n = 0; while n < 100 { s = s + s; n = n + 1; }").unwrap_err();
        assert!(err.contains("quá lớn"), "lỗi lạ: {}", err);
    }

    #[test]
    fn import_cycle_detected() {
        let a = sandbox_tmp("cyc_a.kinet");
        let b = sandbox_tmp("cyc_b.kinet");
        std::fs::write(&a, "import \"target/test_tmp/cyc_b.kinet\"; let FROM_A = 1;\n").unwrap();
        std::fs::write(&b, "import \"target/test_tmp/cyc_a.kinet\"; let FROM_B = 2;\n").unwrap();
        let err = run("import \"target/test_tmp/cyc_a.kinet\";").unwrap_err();
        assert!(err.contains("import vòng"), "lỗi lạ: {}", err);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn import_error_not_cached_as_loaded() {
        // File import lỗi → không được đánh dấu "đã nạp"; import lại vẫn thấy lỗi.
        let lib = sandbox_tmp("bad_lib.kinet");
        std::fs::write(&lib, "let x = 1 / 0;\n").unwrap();
        let rel = "target/test_tmp/bad_lib.kinet";
        let err1 = run(&format!("import \"{}\";", rel)).unwrap_err();
        assert!(err1.contains("Chia cho 0"), "lỗi lạ: {}", err1);
        let err2 = run(&format!("import \"{}\";", rel)).unwrap_err();
        assert!(err2.contains("Chia cho 0"), "import lỗi bị bỏ qua: {}", err2);
        // Lỗi trong file import phải ghi tên file.
        assert!(err2.contains("từ file import"), "thiếu tên file: {}", err2);
        let _ = std::fs::remove_file(&lib);
    }

    // ----- try / catch -----

    #[test]
    fn try_catch_binds_error_message() {
        let src = r#"let got = "";
            try { let x = 1 / 0; } catch e { got = e; }
            let caught = len(got) > 0;"#;
        assert_eq!(run_and_get(src, "caught"), Value::Bool(true));
    }

    #[test]
    fn try_without_error_skips_catch() {
        let src = "let val = 0; try { val = 5; } catch e { val = -1; }";
        assert_eq!(run_and_get(src, "val"), Value::Int(5));
    }

    #[test]
    fn catch_without_binding_runs() {
        let src = "let val = 0; try { let x = 1 / 0; } catch { val = 9; }";
        assert_eq!(run_and_get(src, "val"), Value::Int(9));
    }

    #[test]
    fn return_inside_try_propagates() {
        let src = "func f() { try { return 42; } catch e { return 0; } } let v = f();";
        assert_eq!(run_and_get(src, "v"), Value::Int(42));
    }

    #[test]
    fn caught_error_message_has_content() {
        let src = r#"let msg = ""; try { let x = 1 / 0; } catch e { msg = e; } let has = contains(msg, "Chia cho 0");"#;
        assert_eq!(run_and_get(src, "has"), Value::Bool(true));
    }

    // ----- float -----

    #[test]
    fn float_literal_and_arith() {
        assert_eq!(run_and_get("let x = 1.5 + 2.5;", "x"), Value::Float(4.0));
        assert_eq!(run_and_get("let x = 2.5 * 2;", "x"), Value::Float(5.0));
        assert_eq!(run_and_get("let x = 10.0 / 4;", "x"), Value::Float(2.5));
        assert_eq!(run_and_get("let x = 7.5 % 2;", "x"), Value::Float(1.5));
    }

    #[test]
    fn int_division_stays_int() {
        assert_eq!(run_and_get("let x = 5 / 2;", "x"), Value::Int(2));
        assert_eq!(run_and_get("let x = 5.0 / 2;", "x"), Value::Float(2.5));
    }

    #[test]
    fn float_comparison_and_equality() {
        assert_eq!(run_and_get("let x = 1.5 < 2;", "x"), Value::Bool(true));
        assert_eq!(run_and_get("let x = 1 == 1.0;", "x"), Value::Bool(true));
        assert_eq!(run_and_get("let x = 1.0 != 2;", "x"), Value::Bool(true));
    }

    #[test]
    fn float_unary_minus_and_string() {
        assert_eq!(run_and_get("let x = -1.5;", "x"), Value::Float(-1.5));
        assert_eq!(
            run_and_get(r#"let s = "x = " + 3.0;"#, "s"),
            Value::Str("x = 3.0".into())
        );
    }

    #[test]
    fn float_zero_is_falsy() {
        assert_eq!(run_and_get("let x = 0.0; let r = not x;", "r"), Value::Bool(true));
    }

    #[test]
    fn zero_empty_are_falsy() {
        assert_eq!(run_and_get("let x = 0; let r = not x;", "r"), Value::Bool(true));
        assert_eq!(
            run_and_get(r#"let x = ""; let r = not x;"#, "r"),
            Value::Bool(true)
        );
        assert_eq!(
            run_and_get("let x = []; let r = not x;", "r"),
            Value::Bool(true)
        );
        assert_eq!(run_and_get("let x = 5; let r = not x;", "r"), Value::Bool(false));
    }
}

fn native_append(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("append() cần 2 đối số (list, x)".to_string());
    }
    let Value::List(items) = &args[0] else {
        return Err("append() cần list ở đối số đầu".to_string());
    };
    check_len("list", items.len() + 1, MAX_LIST_LEN)?;
    let mut out = items.clone();
    out.push(args[1].clone());
    Ok(Value::List(out))
}