//! Interpreter: chạy AST (tree-walking).
//!
//! Mô hình:
//!   Value  = giá trị runtime (số, chuỗi, bool, hàm, nil)
//!   Env    = phạm vi biến, có nối tới cha (cho closure/block)
//!   Flow   = tín hiệu điều khiển (return) trong khi chạy

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, Write};

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
    // Định nghĩa struct (dùng để tạo instance khi gọi Name(args)).
    StructType(StructTypeDef),
    // Một instance struct: fields + methods nhúng sẵn.
    Struct(StructVal),
    Nil,
}

/// Định nghĩa struct: tên + field mặc định + phương thức (closure lúc định nghĩa).
#[derive(Debug, Clone)]
pub struct StructTypeDef {
    pub name: String,
    pub fields: Vec<(String, Value)>,
    pub methods: IndexMap<String, FuncDef>,
}

/// Một instance struct: fields riêng + tham chiếu tới định nghĩa struct.
/// Dùng Rc để instance gọn và rẻ (không sao chép methods mỗi lần tạo).
#[derive(Debug, Clone)]
pub struct StructVal {
    pub ty: std::rc::Rc<StructTypeDef>,
    pub fields: IndexMap<String, Value>,
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
            // Struct type: so theo tên; instance: theo tên + nội dung field.
            (Value::StructType(a), Value::StructType(b)) => a.name == b.name,
            (Value::Struct(a), Value::Struct(b)) => a.ty.name == b.ty.name && a.fields == b.fields,
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
            Value::StructType(t) => {
                9u8.hash(state);
                t.name.hash(state);
            }
            Value::Struct(s) => {
                10u8.hash(state);
                s.ty.name.hash(state);
                // Băm từng (tên field, giá trị) theo thứ tự — khớp với eq.
                for (k, v) in &s.fields {
                    k.hash(state);
                    v.hash(state);
                }
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
        // Cho phép tăng ngân sách thực thi qua env (chạy thuật toán nặng).
        let max_steps = std::env::var("KINET_MAX_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_STEPS);
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
                ("exists", native_exists as fn(&[Value]) -> Result<Value, String>),
                ("append_file", native_append_file as fn(&[Value]) -> Result<Value, String>),
                ("list_dir", native_list_dir as fn(&[Value]) -> Result<Value, String>),
                ("read_lines", native_read_lines as fn(&[Value]) -> Result<Value, String>),
                ("args", native_args as fn(&[Value]) -> Result<Value, String>),
                ("get_env", native_get_env as fn(&[Value]) -> Result<Value, String>),
                ("sleep", native_sleep as fn(&[Value]) -> Result<Value, String>),
                ("input_line", native_input_line as fn(&[Value]) -> Result<Value, String>),
                ("assert", native_assert as fn(&[Value]) -> Result<Value, String>),
                ("chars", native_chars as fn(&[Value]) -> Result<Value, String>),
                ("to_int", native_to_int as fn(&[Value]) -> Result<Value, String>),
                ("to_float", native_to_float as fn(&[Value]) -> Result<Value, String>),
                ("str", native_str as fn(&[Value]) -> Result<Value, String>),
                ("round", native_round as fn(&[Value]) -> Result<Value, String>),
                ("floor", native_floor as fn(&[Value]) -> Result<Value, String>),
                ("ceil", native_ceil as fn(&[Value]) -> Result<Value, String>),
                ("abs", native_abs as fn(&[Value]) -> Result<Value, String>),
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
            max_steps,
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
                finally_body,
            } => {
                // Kết quả sau try (+ catch nếu có lỗi và có catch). Lỗi runtime
                // (Err) thì catch bắt; return/break/continue vẫn đi qua.
                let mut outcome: Result<Flow, String> = match self.exec_block(body, env.clone()) {
                    Err(error_msg) => {
                        if let Some(catch_body) = catch_body {
                            let scope =
                                std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                            if let Some(var) = catch_var {
                                scope.borrow_mut().define(var, Value::Str(error_msg));
                            }
                            self.exec_block(catch_body, scope)
                        } else {
                            // Không có catch — giữ nguyên lỗi để chạy finally rồi
                            // trả lại lỗi gốc nếu finally bình thường.
                            Err(error_msg)
                        }
                    }
                    Ok(flow) => Ok(flow),
                };
                // finally luôn chạy (nếu có) và được quyền override outcome cũ:
                // finally lỗi → lỗi mới thay lỗi cũ; finally return/break/continue
                // → thay kết quả đang dở dang.
                if !finally_body.is_empty() {
                    match self.exec_block(finally_body, env.clone()) {
                        Ok(Flow::Normal) => {}
                        Ok(flow @ (Flow::Return(_) | Flow::Break | Flow::Continue)) => {
                            outcome = Ok(flow)
                        }
                        Err(e) => outcome = Err(e),
                    }
                }
                outcome
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
            Stmt::Struct {
                name,
                fields,
                methods,
            } => {
                // Đánh giá giá trị mặc định của field ngay khi định nghĩa struct,
                // và biến mỗi method thành FuncDef đóng quanh env hiện tại.
                let mut field_defaults: Vec<(String, Value)> = Vec::with_capacity(fields.len());
                for (fname, def) in fields {
                    let v = self.eval(def, env.clone())?;
                    field_defaults.push((fname.clone(), v));
                }
                let mut method_defs: IndexMap<String, FuncDef> = IndexMap::new();
                for m in methods {
                    method_defs.insert(
                        m.name.clone(),
                        FuncDef {
                            name: format!("{}.{}", name, m.name),
                            params: m.params.clone(),
                            body: m.body.clone(),
                            closure: env.clone(),
                        },
                    );
                }
                let st = StructTypeDef {
                    name: name.clone(),
                    fields: field_defaults,
                    methods: method_defs,
                };
                env.borrow_mut().define(name, Value::StructType(st));
                Ok(Flow::Normal)
            }
            Stmt::Return(e) => {
                let v = self.eval(e, env)?;
                Ok(Flow::Return(v))
            }
            Stmt::Throw(e) => {
                // Tạo lỗi runtime — try/catch hoặc báo ra ngoài (kèm số dòng).
                let v = self.eval(e, env)?;
                Err(self.to_string(&v))
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
            Expr::If {
                cond,
                then_expr,
                else_expr,
            } => {
                let c = self.eval(cond, env.clone())?;
                if self.truthy(&c) {
                    self.eval(then_expr, env)
                } else {
                    self.eval(else_expr, env)
                }
            }
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
            Expr::Index { target, index } => {
                let container = self.eval(target, env.clone())?;
                let idx = self.eval(index, env)?;
                self.index_get(&container, &idx)
            }
            Expr::IndexAssign {
                name,
                indices,
                value,
            } => {
                let root = env.borrow().get(name)?;
                if indices.is_empty() {
                    let v = self.eval(value, env.clone())?;
                    env.borrow_mut().assign(name, v.clone())?;
                    return Ok(v);
                }
                // Đi sâu tới "cha" của lá, nhớ lại các container dọc đường.
                let mut stack: Vec<Value> = Vec::new();
                let mut current = root;
                for idx in &indices[..indices.len() - 1] {
                    let iv = self.eval(idx, env.clone())?;
                    let next = self.index_get(&current, &iv)?;
                    stack.push(current);
                    current = next;
                }
                let last_iv = self.eval(indices.last().unwrap(), env.clone())?;
                let new_val = self.eval(value, env.clone())?;
                let mut rebuilt = self.index_set(&current, &last_iv, new_val)?;
                // Dựng ngược lại các lớp, rồi gán về biến gốc.
                for idx in indices[..indices.len() - 1].iter().rev() {
                    let container = stack.pop().unwrap();
                    let iv = self.eval(idx, env.clone())?;
                    rebuilt = self.index_set(&container, &iv, rebuilt)?;
                }
                env.borrow_mut().assign(name, rebuilt.clone())?;
                Ok(rebuilt)
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
            Expr::Slice {
                target,
                start,
                end,
                step,
            } => self.eval_slice(target, start, end, step, env),
            Expr::Comp { element, clauses } => {
                self.eval_list_comp(element, clauses, env)
            }
            Expr::DictComp {
                key,
                value,
                clauses,
            } => self.eval_dict_comp(key, value, clauses, env),
            Expr::MethodCall {
                target,
                name,
                args,
            } => {
                let receiver = self.eval(target, env.clone())?;
                let mut arg_values = Vec::with_capacity(args.len());
                for a in args {
                    arg_values.push(self.eval(a, env.clone())?);
                }
                self.call_method(receiver, name, arg_values)
            }
            Expr::Member { target, name } => {
                let v = self.eval(target, env)?;
                get_struct_field(&v, name)
            }
            Expr::MemberAssign {
                name,
                path,
                value,
            } => {
                // name.x.y.z = value — đi sâu, đặt field, dựng lại rồi gán biến gốc.
                let root = env.borrow().get(name)?;
                let mut current = root;
                let mut stack: Vec<Value> = Vec::new();
                for f in &path[..path.len() - 1] {
                    let next = get_struct_field(&current, f)?;
                    stack.push(current);
                    current = next;
                }
                let new_val = self.eval(value, env.clone())?;
                let mut rebuilt =
                    set_struct_field(&current, path.last().unwrap(), new_val)?;
                for f in path[..path.len() - 1].iter().rev() {
                    let container = stack.pop().unwrap();
                    rebuilt = set_struct_field(&container, f, rebuilt)?;
                }
                env.borrow_mut().assign(name, rebuilt.clone())?;
                Ok(rebuilt)
            }
        }
    }

fn eval_slice(
        &mut self,
        target: &Expr,
        start: &Option<Box<Expr>>,
        end: &Option<Box<Expr>>,
        step: &Option<Box<Expr>>,
        env: EnvRef,
    ) -> Result<Value, String> {
        let container = self.eval(target, env.clone())?;
        let s = match start {
            Some(e) => Some(expect_index(&self.eval(e, env.clone())?)?),
            None => None,
        };
        let en = match end {
            Some(e) => Some(expect_index(&self.eval(e, env.clone())?)?),
            None => None,
        };
        let st = match step {
            Some(e) => expect_index(&self.eval(e, env.clone())?)?,
            None => 1,
        };
        self.slice_value(&container, s, en, st)
    }

    fn eval_list_comp(
        &mut self,
        element: &Expr,
        clauses: &[crate::ast::CompClause],
        env: EnvRef,
    ) -> Result<Value, String> {
        let mut out: Vec<Value> = Vec::new();
        let mut leaf = |this: &mut Self, leaf_env: EnvRef| -> Result<(), String> {
            let v = this.eval(element, leaf_env)?;
            check_len("comprehension", out.len() + 1, MAX_LIST_LEN)?;
            out.push(v);
            Ok(())
        };
        self.comp_clause_walk(clauses, 0, env, &mut leaf)?;
        Ok(Value::List(out))
    }

    fn eval_dict_comp(
        &mut self,
        key: &Expr,
        value: &Expr,
        clauses: &[crate::ast::CompClause],
        env: EnvRef,
    ) -> Result<Value, String> {
        let mut out: IndexMap<Value, Value> = IndexMap::new();
        let mut leaf = |this: &mut Self, leaf_env: EnvRef| -> Result<(), String> {
            let k = this.eval(key, leaf_env.clone())?;
            let v = this.eval(value, leaf_env)?;
            check_len("dict comprehension", out.len() + 1, MAX_LIST_LEN)?;
            out.insert(k, v);
            Ok(())
        };
        self.comp_clause_walk(clauses, 0, env, &mut leaf)?;
        Ok(Value::Map(out))
    }

    // Đi qua chuỗi mệnh đề comprehension (for/if) và gọi `leaf` ở mỗi tổ hợp
    // biến khớp. `for` tạo scope riêng mỗi lần lặp (giống vòng for thường).
    fn comp_clause_walk(
        &mut self,
        clauses: &[crate::ast::CompClause],
        i: usize,
        env: EnvRef,
        leaf: &mut dyn FnMut(&mut Self, EnvRef) -> Result<(), String>,
    ) -> Result<(), String> {
        use crate::ast::CompClause;
        if i == clauses.len() {
            return leaf(self, env);
        }
        match &clauses[i] {
            CompClause::For { var, iterable } => {
                let iter = self.eval(iterable, env.clone())?;
                let Value::List(items) = iter else {
                    return Err(format!(
                        "comprehension 'for' cần list (vd: range(0,5)), gặp {}",
                        self.to_string(&iter)
                    ));
                };
                for item in items {
                    let scope =
                        std::rc::Rc::new(std::cell::RefCell::new(Env::child(env.clone())));
                    scope.borrow_mut().define(var, item);
                    self.comp_clause_walk(clauses, i + 1, scope, leaf)?;
                }
            }
            CompClause::If(cond) => {
                let c = self.eval(cond, env.clone())?;
                if self.truthy(&c) {
                    self.comp_clause_walk(clauses, i + 1, env, leaf)?;
                }
            }
        }
        Ok(())
    }

    // Lấy lát cắt theo quy tắc Python: a[start:end:step]; start/end/step None = mặc định.
    fn slice_value(
        &self,
        container: &Value,
        start: Option<i64>,
        end: Option<i64>,
        step: i64,
    ) -> Result<Value, String> {
        match container {
            Value::List(items) => {
                let (lo, hi) = adjust_slice(items.len(), start, end, step)?;
                let mut out = Vec::new();
                let mut i = lo;
                while (step > 0 && i < hi) || (step < 0 && i > hi) {
                    out.push(items[i as usize].clone());
                    check_len("slice list", out.len(), MAX_LIST_LEN)?;
                    i += step;
                }
                Ok(Value::List(out))
            }
            Value::Str(s) => {
                let chars: Vec<char> = s.chars().collect();
                let (lo, hi) = adjust_slice(chars.len(), start, end, step)?;
                let mut out = String::new();
                let mut i = lo;
                while (step > 0 && i < hi) || (step < 0 && i > hi) {
                    out.push(chars[i as usize]);
                    check_len("slice chuỗi", out.chars().count(), MAX_STR_CHARS)?;
                    i += step;
                }
                Ok(Value::Str(out))
            }
            _ => Err("slice `[a:b]` chỉ áp dụng cho list hoặc chuỗi".to_string()),
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

    // `x in y`: list chứa phần tử, chuỗi chứa chuỗi con, map chứa khóa.
    // Để ngoài eval_binary cho frame gọn (path nóng của đệ quy).
    fn eval_in(&mut self, lv: &Value, rv: &Value) -> Result<Value, String> {
        // struct có thể định nghĩa __contains__(x).
        if let Some(res) = self.call_magic(rv, "__contains__", vec![lv.clone()]) {
            let found = res?;
            return Ok(Value::Bool(self.truthy(&found)));
        }
        if let Value::Str(s) = rv {
            let Value::Str(needle) = lv else {
                return Err("vế trái của 'in' phải là chuỗi khi vế phải là chuỗi".to_string());
            };
            return Ok(Value::Bool(s.contains(needle)));
        }
        let found = match rv {
            Value::List(items) => items.contains(lv),
            Value::Map(entries) => entries.contains_key(lv),
            _ => {
                return Err(format!(
                    "toán tử 'in' cần list, chuỗi hoặc map ở vế phải, gặp {}",
                    self.to_string(rv)
                ));
            }
        };
        Ok(Value::Bool(found))
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

        // Magic methods của struct được ưu tiên trước hành vi mặc định.
        if let Some(r) = self.try_binop_magic(op, &lv, &rv)? {
            return Ok(r);
        }

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
            TokenKind::Star => {
                // "ab" * 3 → "ababab" (và 3 * "ab" → "ababab")
                if let (Value::Str(s), Value::Int(n)) = (&lv, &rv) {
                    return self.repeat_str(s, *n);
                }
                if let (Value::Int(n), Value::Str(s)) = (&lv, &rv) {
                    return self.repeat_str(s, *n);
                }
                self.num_arith(&lv, &rv, op)
            }
            TokenKind::Slash => self.num_arith(&lv, &rv, op),
            TokenKind::Percent => self.num_arith(&lv, &rv, op),
            TokenKind::Eq => Ok(Value::Bool(lv == rv)),
            TokenKind::NotEq => Ok(Value::Bool(lv != rv)),
            TokenKind::Lt => self.cmp_values(&lv, &rv, op),
            TokenKind::Gt => self.cmp_values(&lv, &rv, op),
            TokenKind::Le => self.cmp_values(&lv, &rv, op),
            TokenKind::Ge => self.cmp_values(&lv, &rv, op),
            TokenKind::In => self.eval_in(&lv, &rv),
            TokenKind::Band => self.bitop(&lv, &rv, op),
            TokenKind::Bor => self.bitop(&lv, &rv, op),
            TokenKind::Shl => self.bitop(&lv, &rv, op),
            TokenKind::Shr => self.bitop(&lv, &rv, op),
            _ => unreachable!(),
        }
    }

    // Phép bit: chỉ số nguyên, có check dịch chuyển vượt 63 bit.
    fn bitop(&self, l: &Value, r: &Value, op: &TokenKind) -> Result<Value, String> {
        let (Value::Int(a), Value::Int(b)) = (l, r) else {
            return Err(format!(
                "Toán tử '{}' cần hai số nguyên",
                self.op_symbol(op)
            ));
        };
        let result = match op {
            TokenKind::Band => a & b,
            TokenKind::Bor => a | b,
            TokenKind::Shl => {
                let n = *b as u32;
                if *b < 0 || n >= 64 {
                    return Err("dịch trái quá 63 bit".to_string());
                }
                a << n
            }
            TokenKind::Shr => {
                let n = *b as u32;
                if *b < 0 || n >= 64 {
                    return Err("dịch phải quá 63 bit".to_string());
                }
                a >> n
            }
            _ => unreachable!(),
        };
        Ok(Value::Int(result))
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

    fn repeat_str(&self, s: &str, n: i64) -> Result<Value, String> {
        if n < 0 {
            return Err("lặp chuỗi ('*') không được dùng số âm".to_string());
        }
        let total = s.chars().count().saturating_mul(n as usize);
        check_len("chuỗi lặp", total, MAX_STR_CHARS)?;
        Ok(Value::Str(s.repeat(n as usize)))
    }

    fn as_f64(&self, v: &Value, op: &TokenKind) -> Result<f64, String> {
        match v {
            Value::Int(n) => Ok(*n as f64),
            Value::Float(f) => Ok(*f),
            _ => Err(format!("Toán tử '{}' cần số", self.op_symbol(op))),
        }
    }

    // Đọc phần tử: list/chuỗi theo chỉ số int (âm = đếm từ cuối), map theo khóa,
    // struct theo __getitem__ (nếu định nghĩa).
    fn index_get(&mut self, container: &Value, idx: &Value) -> Result<Value, String> {
        if let Value::Struct(s) = container {
            if s.ty.methods.contains_key("__getitem__") {
                return self.call_method(container.clone(), "__getitem__", vec![idx.clone()]);
            }
            return Err(format!(
                "struct '{}' không hỗ trợ `[]` (thiếu __getitem__)",
                s.ty.name
            ));
        }
        match container {
            Value::List(items) => {
                let i = expect_and_normalize_index(idx, items.len())?;
                Ok(items[i].clone())
            }
            Value::Str(s) => {
                let chars: Vec<char> = s.chars().collect();
                let i = expect_and_normalize_index(idx, chars.len())?;
                Ok(Value::Str(chars[i].to_string()))
            }
            Value::Map(entries) => entries.get(idx).cloned().ok_or_else(|| {
                "map không có khóa này".to_string()
            }),
            _ => Err("chỉ số `[]` chỉ áp dụng cho list, chuỗi hoặc map".to_string()),
        }
    }

    // Ghi phần tử (bản sao cập nhật); dùng cho `arr[i] = v`. Chỉ số âm được phép.
    fn index_set(&mut self, container: &Value, idx: &Value, val: Value) -> Result<Value, String> {
        if let Value::Struct(s) = container {
            if s.ty.methods.contains_key("__setitem__") {
                return self.call_method(container.clone(), "__setitem__", vec![idx.clone(), val]);
            }
            return Err(format!(
                "struct '{}' không hỗ trợ gán `[]` (thiếu __setitem__)",
                s.ty.name
            ));
        }
        match container {
            Value::List(items) => {
                let i = expect_and_normalize_index(idx, items.len())?;
                let mut out = items.clone();
                out[i] = val;
                Ok(Value::List(out))
            }
            Value::Map(entries) => {
                check_len("map", entries.len() + 1, MAX_LIST_LEN)?;
                let mut out = entries.clone();
                out.insert(idx.clone(), val);
                Ok(Value::Map(out))
            }
            _ => Err("chỉ số `[]` chỉ áp dụng cho list hoặc map".to_string()),
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
            TokenKind::In => "in",
            TokenKind::Shl => "<<",
            TokenKind::Shr => ">>",
            TokenKind::Band => "&",
            TokenKind::Bor => "|",
            _ => "?",
        }
    }

    // `x.method(args)` — chọn native tương ứng theo kiểu của receiver rồi gọi
    // với [receiver, args...]. Mọi phương thức đều THUẦN (không sửa biến gốc),
    // nhất quán với thiết kế functional của Kinet (append/map_set trả bản mới).
    fn call_method(&mut self, receiver: Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
        let builtin: &str = match &receiver {
            Value::Str(_) => match name {
                "upper" => "to_upper",
                "lower" => "to_lower",
                "trim" => "trim",
                "split" => "split",
                "starts_with" => "starts_with",
                "ends_with" => "ends_with",
                "contains" => "contains",
                "replace" => "replace",
                "len" => "len",
                "chars" => "chars",
                "ord" => "ord",
                "to_int" => "to_int",
                "to_float" => "to_float",
                "substring" => "substring",
                "lines" => "read_lines",
                _ => return Err(self.method_err(&receiver, name)),
            },
            Value::List(_) => match name {
                "append" => "append",
                "len" => "len",
                "contains" => "contains",
                "get" => "get",
                "join" => "list_join",
                "map" => "map",
                "filter" => "filter",
                "reverse" => return self.slice_value(&receiver, None, None, -1),
                _ => return Err(self.method_err(&receiver, name)),
            },
            Value::Map(_) => match name {
                "get" => "get",
                "contains" => "contains",
                "keys" => "map_keys",
                "set" => "map_set",
                "remove" => "map_remove",
                "len" => "len",
                _ => return Err(self.method_err(&receiver, name)),
            },
            Value::Int(_) | Value::Float(_) => match name {
                "abs" => "abs",
                "round" => "round",
                "floor" => "floor",
                "ceil" => "ceil",
                "to_int" => "to_int",
                "to_float" => "to_float",
                _ => return Err(self.method_err(&receiver, name)),
            },
            Value::Struct(s) => {
                if let Some(def) = s.ty.methods.get(name) {
                    return self.call_struct_method(receiver.clone(), def.clone(), args);
                }
                return Err(self.method_err(&receiver, name));
            }
            _ => return Err(self.method_err(&receiver, name)),
        };
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(receiver.clone());
        all.extend(args);
        let func = self.globals.borrow().get(builtin)?;
        self.call(func, all)
    }

    // Gọi một method của struct: đưa các field vào env (dùng tên trực tiếp:
    // `x` thay vì `self.x`), đồng thời gán `self` = instance.
    fn call_struct_method(
        &mut self,
        receiver: Value,
        def: FuncDef,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        let env = std::rc::Rc::new(std::cell::RefCell::new(Env::child(def.closure.clone())));
        if let Value::Struct(s) = &receiver {
            for (k, v) in &s.fields {
                env.borrow_mut().define(k, v.clone());
            }
        }
        env.borrow_mut().define("self", receiver.clone());
        let bound = FuncDef {
            name: def.name.clone(),
            params: def.params.clone(),
            body: def.body.clone(),
            closure: env,
        };
        self.call(Value::Func(bound), args)
    }

    // Gọi magic method nếu struct có (ví dụ __add__). None = không có.
    fn call_magic(&mut self, v: &Value, name: &str, args: Vec<Value>) -> Option<Result<Value, String>> {
        if let Value::Struct(s) = v {
            if s.ty.methods.contains_key(name) {
                return Some(self.call_method(v.clone(), name, args));
            }
        }
        None
    }

    // Thử magic binop: toán hạng trái trước, rồi mới tới phải (như __add__/__radd__).
    fn try_magic_binop(
        &mut self,
        lv: &Value,
        rv: &Value,
        name: &str,
    ) -> Result<Option<Value>, String> {
        if let Some(res) = self.call_magic(lv, name, vec![rv.clone()]) {
            return Ok(Some(res?));
        }
        if let Some(res) = self.call_magic(rv, name, vec![lv.clone()]) {
            return Ok(Some(res?));
        }
        Ok(None)
    }

    // Magic cho toán tử nhị phân — tách ngoài eval_binary để frame hot gọn.
    fn try_binop_magic(
        &mut self,
        op: &TokenKind,
        lv: &Value,
        rv: &Value,
    ) -> Result<Option<Value>, String> {
        let (magic, negate) = match op {
            TokenKind::Plus => ("__add__", false),
            TokenKind::Minus => ("__sub__", false),
            TokenKind::Star => ("__mul__", false),
            TokenKind::Slash => ("__div__", false),
            TokenKind::Percent => ("__mod__", false),
            TokenKind::Eq => ("__eq__", false),
            TokenKind::NotEq => ("__eq__", true),
            _ => return Ok(None),
        };
        if let Some(r) = self.try_magic_binop(lv, rv, magic)? {
            if negate {
                return Ok(Some(Value::Bool(!self.truthy(&r))));
            }
            return Ok(Some(r));
        }
        Ok(None)
    }

    // Kết quả __len__ phải là số nguyên (int/float nguyên), chuyển về Int.
    fn to_index_len(&self, v: Value) -> Result<Value, String> {
        match v {
            Value::Int(n) => Ok(Value::Int(n)),
            Value::Float(f) if f.fract() == 0.0 => Ok(Value::Int(f as i64)),
            _ => Err(format!(
                "__len__() phải trả về số nguyên, nhận {}",
                self.to_string(&v)
            )),
        }
    }

    fn method_err(&self, receiver: &Value, name: &str) -> String {
        format!(
            "giá trị {} không có phương thức '{}'",
            self.to_string(receiver),
            name
        )
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
                // len/str có thể trỏ tới struct magic __len__/__str__.
                if n.name == "len" {
                    if let Some(Value::Struct(s)) = args.first() {
                        if s.ty.methods.contains_key("__len__") {
                            let v = self.call_method(args[0].clone(), "__len__", Vec::new())?;
                            return Ok(self.to_index_len(v)?);
                        }
                    }
                    return (n.func)(&args);
                }
                if n.name == "str" {
                    if let Some(Value::Struct(s)) = args.first() {
                        if s.ty.methods.contains_key("__str__") {
                            let v = self.call_method(args[0].clone(), "__str__", Vec::new())?;
                            return Ok(Value::Str(self.to_string(&v)));
                        }
                    }
                    return (n.func)(&args);
                }
                return (n.func)(&args);
            }
            Value::StructType(st) => {
                // Gọi `Tên(args)` → tạo instance: gán đối số cho field theo thứ tự.
                if args.len() > st.fields.len() {
                    return Err(format!(
                        "struct '{}' có {} field, nhận {} đối số",
                        st.name,
                        st.fields.len(),
                        args.len()
                    ));
                }
                let mut fields: IndexMap<String, Value> = st
                    .fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (i, arg) in args.into_iter().enumerate() {
                    let fname = st.fields[i].0.clone();
                    fields.insert(fname, arg);
                }
                return Ok(Value::Struct(StructVal {
                    ty: std::rc::Rc::new(st),
                    fields,
                }));
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
        is_truthy_value(v)
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
            Value::StructType(t) => format!("<struct {}>", t.name),
            Value::Struct(s) => {
                let inner = s
                    .fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, self.to_string(v)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{{}}}", s.ty.name, inner)
            }
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
        // Chỉ số âm đếm từ cuối (như `a[-1]`) — nhất quán với toán tử `[]`.
        Value::List(items) => {
            let i = expect_and_normalize_index(&args[1], items.len())?;
            Ok(items[i].clone())
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
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(f) => {
            if f.is_finite() {
                Ok(Value::Int(*f as i64))
            } else {
                Err("to_int(): số không hữu hạn".to_string())
            }
        }
        Value::Str(s) => s.trim().parse::<i64>().map(Value::Int).map_err(|_| {
            format!("to_int(): '{}' không phải số nguyên", s)
        }),
        _ => Err("to_int() cần chuỗi, số nguyên hoặc số thập phân".to_string()),
    }
}

fn native_to_float(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("to_float() cần 1 đối số".to_string());
    }
    match &args[0] {
        Value::Float(f) => Ok(Value::Float(*f)),
        Value::Int(n) => Ok(Value::Float(*n as f64)),
        Value::Str(s) => s
            .trim()
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| format!("to_float(): '{}' không phải số thập phân", s)),
        _ => Err("to_float() cần chuỗi, số nguyên hoặc số thập phân".to_string()),
    }
}

fn native_str(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("str() cần 1 đối số".to_string());
    }
    Ok(Value::Str(stringify_value(&args[0])))
}

fn native_round(args: &[Value]) -> Result<Value, String> {
    one_number(args, "round", |f| f.round())
}

fn native_floor(args: &[Value]) -> Result<Value, String> {
    one_number(args, "floor", |f| f.floor())
}

fn native_ceil(args: &[Value]) -> Result<Value, String> {
    one_number(args, "ceil", |f| f.ceil())
}

fn native_abs(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("abs() cần 1 đối số".to_string());
    }
    match &args[0] {
        Value::Int(n) => n
            .checked_abs()
            .map(Value::Int)
            .ok_or_else(|| "abs(): tràn số với i64::MIN".to_string()),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        _ => Err("abs() cần số".to_string()),
    }
}

/// Giúp round/floor/ceil: chuyển int/float → áp hàm → trả Int.
fn one_number(args: &[Value], name: &str, op: fn(f64) -> f64) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("{}() cần 1 đối số", name));
    }
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(f) => {
            if f.is_finite() {
                Ok(Value::Int(op(*f) as i64))
            } else {
                Err(format!("{}(): số không hữu hạn", name))
            }
        }
        _ => Err(format!("{}() cần số", name)),
    }
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

fn native_exists(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("exists() cần 1 đối số (đường dẫn)".to_string());
    }
    let Value::Str(path) = &args[0] else {
        return Err("exists() cần chuỗi đường dẫn".to_string());
    };
    let resolved = resolve_sandbox(path)?;
    Ok(Value::Bool(resolved.exists()))
}

fn native_append_file(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("append_file() cần 2 đối số (đường dẫn, nội dung)".to_string());
    }
    let Value::Str(path) = &args[0] else {
        return Err("append_file() cần chuỗi đường dẫn".to_string());
    };
    let Value::Str(content) = &args[1] else {
        return Err("append_file() cần nội dung là chuỗi".to_string());
    };
    check_len("append_file() nội dung", content.chars().count(), MAX_STR_CHARS)?;
    let resolved = resolve_sandbox(path)?;
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&resolved)
        .map_err(|e| format!("append_file('{}') thất bại: {}", path, e))?;
    f.write_all(content.as_bytes())
        .map_err(|e| format!("append_file('{}') thất bại: {}", path, e))?;
    Ok(Value::Bool(true))
}

fn native_list_dir(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("list_dir() cần 1 đối số (thư mục)".to_string());
    }
    let Value::Str(path) = &args[0] else {
        return Err("list_dir() cần chuỗi đường dẫn".to_string());
    };
    let resolved = resolve_sandbox(path)?;
    let mut names: Vec<String> = std::fs::read_dir(&resolved)
        .map_err(|e| format!("list_dir('{}') thất bại: {}", path, e))?
        .filter_map(|entry| {
            entry
                .ok()
                .and_then(|e| e.file_name().into_string().ok())
        })
        .collect();
    names.sort(); // in thứ tự ổn định
    Ok(Value::List(
        names.into_iter().map(Value::Str).collect(),
    ))
}

fn native_read_lines(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("read_lines() cần 1 đối số (chuỗi)".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("read_lines() cần chuỗi".to_string());
    };
    let lines: Vec<Value> = s.lines().map(|l| Value::Str(l.to_string())).collect();
    check_len("read_lines() kết quả", lines.len(), MAX_LIST_LEN)?;
    Ok(Value::List(lines))
}

fn native_args(_args: &[Value]) -> Result<Value, String> {
    // Bỏ tên binary; [0] = file đang chạy, [1..] = tham số người dùng.
    let list: Vec<Value> = std::env::args().skip(1).map(Value::Str).collect();
    Ok(Value::List(list))
}

fn native_get_env(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("get_env() cần 1 đối số (tên biến)".to_string());
    }
    let Value::Str(name) = &args[0] else {
        return Err("get_env() cần chuỗi tên biến".to_string());
    };
    Ok(Value::Str(std::env::var(name).unwrap_or_default()))
}

fn native_sleep(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("sleep() cần 1 đối số (mili giây)".to_string());
    }
    let Value::Int(ms) = args[0] else {
        return Err("sleep() cần số nguyên mili giây".to_string());
    };
    if ms < 0 {
        return Err("sleep() cần mili giây không âm".to_string());
    }
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    Ok(Value::Nil)
}

fn native_input_line(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err("input_line() không nhận đối số".to_string());
    }
    let mut line = String::new();
    let n = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("input_line() lỗi: {}", e))?;
    if n == 0 {
        return Ok(Value::Nil); // hết dữ liệu (EOF)
    }
    Ok(Value::Str(line.trim_end_matches(['\r', '\n']).to_string()))
}

fn native_assert(args: &[Value]) -> Result<Value, String> {
    if args.len() < 1 || args.len() > 2 {
        return Err("assert() cần 1-2 đối số (điều kiện, thông điệp?)".to_string());
    }
    if !is_truthy_value(&args[0]) {
        let msg = if args.len() == 2 {
            stringify_value(&args[1])
        } else {
            "điều kiện sai".to_string()
        };
        return Err(format!("Assert thất bại: {}", msg));
    }
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

fn expect_index(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(n) => Ok(*n),
        Value::Float(f) if f.fract() == 0.0 => Ok(*f as i64),
        _ => Err("chỉ số `[]` cần số nguyên".to_string()),
    }
}

/// Đọc field của một struct (dùng cho `p.x`).
fn get_struct_field(v: &Value, name: &str) -> Result<Value, String> {
    match v {
        Value::Struct(s) => s
            .fields
            .get(name)
            .cloned()
            .ok_or_else(|| format!("struct '{}' không có field '{}'", s.ty.name, name)),
        _ => Err(format!(
            "giá trị {} không có field '{}'",
            stringify_value(v),
            name
        )),
    }
}

/// Đặt field của struct (bản sao mới; dùng cho `p.x = v`).
fn set_struct_field(v: &Value, name: &str, val: Value) -> Result<Value, String> {
    match v {
        Value::Struct(s) => {
            if !s.fields.contains_key(name) {
                return Err(format!("struct '{}' không có field '{}'", s.ty.name, name));
            }
            let mut fields = s.fields.clone();
            fields.insert(name.to_string(), val);
            Ok(Value::Struct(StructVal {
                ty: s.ty.clone(),
                fields,
            }))
        }
        _ => Err(format!(
            "giá trị {} không có field '{}'",
            stringify_value(v),
            name
        )),
    }
}

/// Chuyển chỉ số sang usize; chỉ số âm đếm từ cuối như Python. Lỗi nếu ngoài phạm vi.
fn expect_and_normalize_index(v: &Value, len: usize) -> Result<usize, String> {
    let raw = expect_index(v)?;
    let n = len as i64;
    let normalized = if raw < 0 { raw + n } else { raw };
    if normalized < 0 || normalized as usize >= len {
        return Err(format!("chỉ số {} ngoài phạm vi (len={})", raw, len));
    }
    Ok(normalized as usize)
}

/// Điều chỉnh start/end của slice theo quy tắc Python (PySlice_AdjustIndices):
/// xử lý step âm/dương và clamp biên. Trả về (lo, hi) để lặp `i` từ lo tới hi.
fn adjust_slice(
    len: usize,
    start: Option<i64>,
    end: Option<i64>,
    step: i64,
) -> Result<(i64, i64), String> {
    if step == 0 {
        return Err("slice: bước nhảy (step) không được bằng 0".to_string());
    }
    let n = len as i64;
    let mut lo = match start {
        Some(s) => s,
        None => {
            if step > 0 {
                0
            } else {
                n - 1
            }
        }
    };
    // Mặc định của "end" khi step âm dùng sentinel cực nhỏ (như PY_SSIZE_T_MIN của
    // CPython) để qua bước chuẩn hóa vẫn về -1 — không nhầm với giá trị do người
    // dùng cung cấp (vd: a[2:0:-1] dừng đúng ở 0).
    let mut hi = match end {
        Some(e) => e,
        None => {
            if step > 0 {
                n
            } else {
                i64::MIN
            }
        }
    };
    if step < 0 {
        if lo < 0 {
            lo += n;
        }
        if lo < 0 {
            lo = -1;
        }
        if lo >= n {
            lo = n - 1;
        }
        if hi < 0 {
            hi += n;
        }
        if hi < 0 {
            hi = -1;
        } else if hi >= n {
            hi = n - 1;
        }
    } else {
        if lo < 0 {
            lo += n;
        }
        if lo < 0 {
            lo = 0;
        }
        if lo > n {
            lo = n;
        }
        if hi < 0 {
            hi += n;
        }
        if hi < 0 {
            hi = 0;
        } else if hi > n {
            hi = n;
        }
    }
    Ok((lo, hi))
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

/// Chuẩn truthiness dùng chung (0/""/[]/{} rỗng là false).
fn is_truthy_value(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Nil => false,
        Value::Int(n) => *n != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::List(items) => !items.is_empty(),
        Value::Map(entries) => !entries.is_empty(),
        Value::Struct(s) => !s.fields.is_empty(),
        _ => true,
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
        Value::StructType(t) => format!("<struct {}>", t.name),
        Value::Struct(s) => {
            let inner: Vec<String> = s
                .fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, stringify_value(v)))
                .collect();
            format!("{} {{{}}}", s.ty.name, inner.join(", "))
        }
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
            .stack_size(128 * 1024 * 1024)
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

    // ----- finally -----

    #[test]
    fn finally_runs_on_success() {
        let src = r#"let log = "";
            try { log = log + "try;"; }
            catch e { log = log + "catch;"; }
            finally { log = log + "finally;"; }"#;
        assert_eq!(run_and_get(src, "log"), Value::Str("try;finally;".into()));
    }

    #[test]
    fn finally_runs_on_caught_error() {
        let src = r#"let log = "";
            try { let x = 1 / 0; log = log + "try;"; }
            catch e { log = log + "catch;"; }
            finally { log = log + "finally;"; }"#;
        assert_eq!(run_and_get(src, "log"), Value::Str("catch;finally;".into()));
    }

    #[test]
    fn finally_runs_then_error_propagates() {
        // try/finally không có catch: lỗi vẫn lan ra ngoài nhưng finally đã chạy.
        let src = r#"let cleaned = "";
            try { let x = 1 / 0; } finally { cleaned = "da_don"; }"#;
        let mut it = Interpreter::new();
        let res = it.run(&program(src));
        let err = res.unwrap_err();
        assert!(err.contains("Chia cho 0"), "lỗi gốc phải còn nguyên: {}", err);
        assert_eq!(
            it.globals.borrow().get("cleaned").unwrap(),
            Value::Str("da_don".into())
        );
    }

    #[test]
    fn finally_runs_on_return_and_preserves_value() {
        let src = "let g = 0; func f() { try { return 42; } finally { g = 99; } } let v = f();";
        assert_eq!(run_and_get(src, "v"), Value::Int(42));
        assert_eq!(run_and_get(src, "g"), Value::Int(99));
    }

    #[test]
    fn finally_return_overrides_pending_return() {
        let src = "func f() { try { return 1; } finally { return 2; } } let v = f();";
        assert_eq!(run_and_get(src, "v"), Value::Int(2));
    }

    #[test]
    fn finally_error_overrides_pending_throw_and_return() {
        // finally ném lỗi → lỗi mới thay cả return đang dở dang.
        let src = r#"func f() { try { return 1; } finally { throw "lỗi ở finally"; } } f();"#;
        let err = run(src).unwrap_err();
        assert!(err.contains("lỗi ở finally"), "{}", err);

        // finally ném lỗi → đè cả lỗi do try gây ra (chưa có catch).
        let src2 = r#"try { let x = 1 / 0; } finally { throw "lỗi ở finally"; }"#;
        let err2 = run(src2).unwrap_err();
        assert!(err2.contains("lỗi ở finally"), "{}", err2);
    }

    #[test]
    fn try_finally_without_catch_allowed() {
        let src = "let x = 0; try { x = x + 1; } finally { x = x + 10; }";
        assert_eq!(run_and_get(src, "x"), Value::Int(11));
    }

    #[test]
    fn finally_runs_when_break_exits_try() {
        // break trong try → finally vẫn chạy rồi break mới có hiệu lực.
        let src = r#"let n = 0;
            for i in range(0, 10) {
                try { if i == 2 { break; } } finally { n = n + 1; }
            }"#;
        assert_eq!(run_and_get(src, "n"), Value::Int(3));
    }

    #[test]
    fn try_requires_catch_or_finally() {
        let src = "try { let x = 1; }";
        let res = Parser::new(Lexer::new(src).tokenize().unwrap()).parse();
        assert!(res.is_err(), "try không có catch/finally phải báo lỗi cú pháp");
    }

    // ----- in / slice / comprehension -----

    #[test]
    fn in_operator_list_string_map() {
        assert_eq!(
            run_and_get(r#"let r = 3 in [1, 2, 3];"#, "r"),
            Value::Bool(true)
        );
        assert_eq!(
            run_and_get(r#"let r = 9 in [1, 2, 3];"#, "r"),
            Value::Bool(false)
        );
        assert_eq!(
            run_and_get(r#"let r = "bc" in "abcd";"#, "r"),
            Value::Bool(true)
        );
        let src = r#"let m = { "x": 1 }; let r = "x" in m;"#;
        assert_eq!(run_and_get(src, "r"), Value::Bool(true));
        let src2 = r#"let m = { "x": 1 }; let r = "y" in m;"#;
        assert_eq!(run_and_get(src2, "r"), Value::Bool(false));
    }

    #[test]
    fn not_in_operator() {
        assert_eq!(
            run_and_get(r#"let r = 9 not in [1, 2, 3];"#, "r"),
            Value::Bool(true)
        );
        assert_eq!(
            run_and_get(r#"let r = 1 not in [1, 2, 3];"#, "r"),
            Value::Bool(false)
        );
    }

    #[test]
    fn in_requires_valid_right_side() {
        let err = run(r#"let r = 1 in 5;"#).unwrap_err();
        assert!(err.contains("in"), "lỗi lạ: {}", err);
    }

    #[test]
    fn list_slice_matches_python() {
        let src = r#"
            let a = [1, 2, 3, 4, 5];
            let s1 = a[1:4];
            let s2 = a[:3];
            let s3 = a[2:];
            let s4 = a[::-1];
            let s5 = a[::2];
            let s6 = a[1:5:2];
            let s7 = a[4:1:-1];
            let s8 = a[2:0:-1];
            let s9 = a[:];
            let s10 = a[-1:-4:-1];
            let s11 = a[-100:2];
            let s12 = a[3:1];
            let s13 = a[100:];
            let s14 = a[-2:];
            let s15 = a[:-2];
            let s16 = a[2::-1];
            let out = str(s1) + "|" + str(s2) + "|" + str(s3) + "|" + str(s4)
                + "|" + str(s5) + "|" + str(s6) + "|" + str(s7) + "|" + str(s8)
                + "|" + str(s9) + "|" + str(s10) + "|" + str(s11) + "|" + str(s12)
                + "|" + str(s13) + "|" + str(s14) + "|" + str(s15) + "|" + str(s16);
        "#;
        let expected =
            "[2, 3, 4]|[1, 2, 3]|[3, 4, 5]|[5, 4, 3, 2, 1]|[1, 3, 5]|[2, 4]|[5, 4, 3]|[3, 2]|[1, 2, 3, 4, 5]|[5, 4, 3]|[1, 2]|[]|[]|[4, 5]|[1, 2, 3]|[3, 2, 1]";
        assert_eq!(run_and_get(src, "out"), Value::Str(expected.into()));
    }

    #[test]
    fn string_slice_by_chars() {
        let src = r#"
            let s = "xin chào";
            let r1 = s[0:3];
            let r2 = s[::-1];
            let r3 = s[-1];
        "#;
        assert_eq!(run_and_get(src, "r1"), Value::Str("xin".into()));
        assert_eq!(run_and_get(src, "r2"), Value::Str("oàhc nix".into()));
        assert_eq!(run_and_get(src, "r3"), Value::Str("o".into()));
    }

    #[test]
    fn negative_index_for_lists_and_get() {
        assert_eq!(run_and_get(r#"let r = [1,2,3][-1];"#, "r"), Value::Int(3));
        assert_eq!(run_and_get(r#"let r = [1,2,3][-3];"#, "r"), Value::Int(1));
        assert_eq!(run_and_get(r#"let r = get([10, 20, 30], -1);"#, "r"), Value::Int(30));
        // Gán vào ô âm
        assert_eq!(
            run_and_get(r#"let a = [1,2,3]; a[-1] = 99; let r = a[2];"#, "r"),
            Value::Int(99)
        );
    }

    #[test]
    fn negative_index_out_of_range_errors() {
        assert!(run(r#"let r = [1,2][-5];"#).is_err());
        assert!(run(r#"let r = [1,2][5];"#).is_err());
        assert!(run(r#"let r = "abc"[-4];"#).is_err());
    }

    #[test]
    fn slice_step_zero_errors() {
        let src = r#"let a = [1,2,3]; let r = a[::0];"#;
        assert!(run(src).is_err());
    }

    #[test]
    fn slice_on_map_errors() {
        let src = r#"let m = { "a": 1 }; let r = m[0:2];"#;
        assert!(run(src).is_err());
    }

    #[test]
    fn list_comprehension_basic_and_filter() {
        assert_eq!(
            run_and_get("let r = [x * x for x in range(0, 6)];", "r"),
            Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(4), Value::Int(9), Value::Int(16), Value::Int(25)])
        );
        assert_eq!(
            run_and_get("let r = [x for x in range(0, 10) if x % 2 == 0];", "r"),
            Value::List(vec![Value::Int(0), Value::Int(2), Value::Int(4), Value::Int(6), Value::Int(8)])
        );
    }

    #[test]
    fn list_comprehension_over_list_and_nested() {
        assert_eq!(
            run_and_get(r#"let r = [to_upper(x) for x in ["a", "b"]];"#, "r"),
            Value::List(vec![Value::Str("A".into()), Value::Str("B".into())])
        );
        // Nhiều vòng: phép nhân kiểu Python
        assert_eq!(
            run_and_get("let r = [x + y for x in [1, 2] for y in [10, 20]];", "r"),
            Value::List(vec![Value::Int(11), Value::Int(21), Value::Int(12), Value::Int(22)])
        );
    }

    #[test]
    fn dict_comprehension() {
        let src = "let m = { i: i * i for i in range(1, 4) }; let r = get(m, 2);";
        assert_eq!(run_and_get(src, "r"), Value::Int(4));
        assert_eq!(run_and_get(src, "m"), Value::Map(
            [(Value::Int(1), Value::Int(1)), (Value::Int(2), Value::Int(4)), (Value::Int(3), Value::Int(9))]
                .into_iter()
                .collect()
        ));
    }

    #[test]
    fn comprehension_variable_does_not_leak() {
        // Biến `x` trong comprehension không rò ra ngoài (scope riêng).
        let src = "let x = 100; let r = [x for x in range(0, 3)]; let y = x;";
        assert_eq!(run_and_get(src, "y"), Value::Int(100));
        assert_eq!(run_and_get(src, "r"), Value::List(
            vec![Value::Int(0), Value::Int(1), Value::Int(2)]
        ));
    }

    // ----- method calls -----

    #[test]
    fn string_methods() {
        assert_eq!(
            run_and_get(r#"let r = "  Kinet  ".trim().upper();"#, "r"),
            Value::Str("KINET".into())
        );
        assert_eq!(
            run_and_get(r#"let r = "a,b,c".split(",").len();"#, "r"),
            Value::Int(3)
        );
        assert_eq!(
            run_and_get(r#"let r = "hello".contains("ell");"#, "r"),
            Value::Bool(true)
        );
        assert_eq!(
            run_and_get(r#"let r = "abcabc".replace("a", "X");"#, "r"),
            Value::Str("XbcXbc".into())
        );
        assert_eq!(
            run_and_get(r#"let r = "42".to_int() + 1;"#, "r"),
            Value::Int(43)
        );
        assert_eq!(
            run_and_get(r#"let r = "xin chào".len();"#, "r"),
            Value::Int(8)
        );
    }

    #[test]
    fn list_methods_are_pure() {
        // append/reverse không sửa biến gốc (thiết kế functional của Kinet).
        let src = r#"let a = [1, 2, 3];
            let b = a.append(4);
            let r1 = b;
            let r2 = a;"#;
        assert_eq!(
            run_and_get(src, "r1"),
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)])
        );
        assert_eq!(
            run_and_get(src, "r2"),
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
        assert_eq!(
            run_and_get(r#"let r = [1, 2, 3].reverse();"#, "r"),
            Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
        );
        assert_eq!(
            run_and_get(r#"let r = [1, 2, 3].get(1);"#, "r"),
            Value::Int(2)
        );
        assert_eq!(
            run_and_get(r#"let r = ["a", "b"].join("-");"#, "r"),
            Value::Str("a-b".into())
        );
    }

    #[test]
    fn list_methods_higher_order() {
        assert_eq!(
            run_and_get(
                r#"let r = [1, 2, 3].map(func(x) { return x * 10; });"#,
                "r"
            ),
            Value::List(vec![Value::Int(10), Value::Int(20), Value::Int(30)])
        );
        assert_eq!(
            run_and_get(
                r#"let r = [1, 2, 3, 4].filter(func(x) { return x % 2 == 0; });"#,
                "r"
            ),
            Value::List(vec![Value::Int(2), Value::Int(4)])
        );
    }

    #[test]
    fn map_methods_are_pure() {
        let src = r#"let m = { "x": 1 };
            let m2 = m.set("y", 2);
            let r1 = m2.get("y");
            let r2 = m.contains("y");
            let r3 = m.keys().len();"#;
        assert_eq!(run_and_get(src, "r1"), Value::Int(2));
        assert_eq!(run_and_get(src, "r2"), Value::Bool(false));
        assert_eq!(run_and_get(src, "r3"), Value::Int(1));
        assert_eq!(run_and_get(r#"let m = {"a": 5}; let r = m.get("a");"#, "r"), Value::Int(5));
    }

    #[test]
    fn number_methods() {
        assert_eq!(run_and_get(r#"let r = (-7).abs();"#, "r"), Value::Int(7));
        assert_eq!(run_and_get(r#"let r = 2.6.round();"#, "r"), Value::Int(3));
        assert_eq!(run_and_get(r#"let r = 2.6.floor();"#, "r"), Value::Int(2));
        assert_eq!(run_and_get(r#"let r = 2.4.ceil();"#, "r"), Value::Int(3));
        assert_eq!(run_and_get(r#"let r = 3.to_float();"#, "r"), Value::Float(3.0));
    }

    #[test]
    fn method_chain_on_complex_target() {
        // Target phức: kết quả hàm, index, slice... đều gọi được phương thức.
        let src = r#"let r = " a b ".trim().split(" ").len();"#;
        assert_eq!(run_and_get(src, "r"), Value::Int(2));
        let src2 = r#"let r = [1, 2, 3, 4][1:].reverse().append(99);"#;
        assert_eq!(
            run_and_get(src2, "r"),
            Value::List(vec![Value::Int(4), Value::Int(3), Value::Int(2), Value::Int(99)])
        );
    }

    #[test]
    fn unknown_or_wrong_type_method_errors() {
        let err = run(r#"let r = [1].upper();"#).unwrap_err();
        assert!(err.contains("upper"), "lỗi lạ: {}", err);
        let err2 = run(r#"let r = "abc".append(1);"#).unwrap_err();
        assert!(err2.contains("append"), "lỗi lạ: {}", err2);
        let err3 = run(r#"let r = 5.not_a_method();"#).unwrap_err();
        assert!(err3.contains("not_a_method"), "lỗi lạ: {}", err3);
    }

    #[test]
    fn field_access_on_primitive_errors() {
        // `x.name` không có '(' giờ là FIELD access (struct); với chuỗi thì
        // lỗi runtime vì chuỗi không có field "upper".
        let err = run(r#"let r = "a".upper;"#).unwrap_err();
        assert!(err.contains("field"), "lỗi lạ: {}", err);
    }

    // ----- struct & magic methods -----

    #[test]
    fn struct_create_and_field_access() {
        let src = r#"struct Point { x = 0; y = 0; }
            let p = Point(3, 4);
            let r = p.x * 10 + p.y;"#;
        assert_eq!(run_and_get(src, "r"), Value::Int(34));
        // Giá trị mặc định khi không truyền đối số.
        assert_eq!(run_and_get(r#"struct P { a = 7; b = 9; } let r = P().a;"#, "r"), Value::Int(7));
        // Truyền một phần đối số: phần còn lại giữ mặc định.
        assert_eq!(run_and_get(r#"struct P { a = 7; b = 9; } let r = P(1).b;"#, "r"), Value::Int(9));
    }

    #[test]
    fn struct_methods_use_self() {
        let src = r#"struct Vec2 {
                x = 0; y = 0;
                func mag() { return x * x + y * y; }
                func dot(v) { return x * v.x + y * v.y; }
                func length() { return self.mag(); }
            }
            let v = Vec2(3, 4);
            let r1 = v.mag();
            let r2 = v.dot(Vec2(1, 1));
            let r3 = v.length();"#;
        assert_eq!(run_and_get(src, "r1"), Value::Int(25));
        assert_eq!(run_and_get(src, "r2"), Value::Int(7));
        assert_eq!(run_and_get(src, "r3"), Value::Int(25));
    }

    #[test]
    fn struct_field_assign_is_value_semantics() {
        // Gán field cập nhật bản sao; biến khác tham chiếu không bị ảnh hưởng.
        let src = r#"struct P { x = 0; y = 0; }
            let p = P(1, 2);
            let q = p;
            p.x = 100;
            let r1 = p.x;
            let r2 = q.x;"#;
        assert_eq!(run_and_get(src, "r1"), Value::Int(100));
        assert_eq!(run_and_get(src, "r2"), Value::Int(1));
    }

    #[test]
    fn struct_nested_field_assign() {
        let src = r#"struct Inner { v = 0; }
            struct Outer { i = Inner(); }
            let o = Outer();
            o.i.v = 42;
            let r = o.i.v;"#;
        assert_eq!(run_and_get(src, "r"), Value::Int(42));
    }

    #[test]
    fn struct_magic_add_len_str() {
        let src = r#"struct Money {
                amount = 0.0;
                currency = "VND";
                func __add__(o) { return Money(amount + o.amount, currency); }
                func __len__() { return 3; }
                func __str__() { return str(amount) + " " + currency; }
            }
            let m = Money(10.5, "USD");
            let s = m + Money(1.5, "USD");
            let r1 = s.amount;
            let r2 = len(m);
            let r3 = str(m);"#;
        assert_eq!(run_and_get(src, "r1"), Value::Float(12.0));
        assert_eq!(run_and_get(src, "r2"), Value::Int(3));
        assert_eq!(run_and_get(src, "r3"), Value::Str("10.5 USD".into()));
    }

    #[test]
    fn struct_magic_eq_mul_div() {
        let src = r#"struct V {
                x = 0; y = 0;
                func __eq__(o) { return x == o.x and y == o.y; }
                func __mul__(k) { return V(x * k, y * k); }
                func __div__(k) { return V(x / k, y / k); }
            }
            let a = V(2, 4);
            let b = V(2, 4);
            let r1 = a == b;
            let r2 = a != V(9, 9);
            let r3 = (a * 2).x;
            let r4 = (V(6, 6) / 3).y;"#;
        assert_eq!(run_and_get(src, "r1"), Value::Bool(true));
        assert_eq!(run_and_get(src, "r2"), Value::Bool(true));
        assert_eq!(run_and_get(src, "r3"), Value::Int(4));
        assert_eq!(run_and_get(src, "r4"), Value::Int(2));
    }

    #[test]
    fn struct_magic_getitem_setitem_contains() {
        let src = r#"struct V {
                x = 0; y = 0;
                func __getitem__(i) { if i == 0 { return x; } return y; }
                func __setitem__(i, v) { if i == 0 { self.x = v; } else { self.y = v; } return self; }
                func __contains__(n) { return n == x or n == y; }
            }
            let v = V(3, 4);
            let r1 = v[0];
            v[1] = 9;
            let r2 = v.y;
            let r3 = 3 in v;
            let r4 = 99 in v;"#;
        assert_eq!(run_and_get(src, "r1"), Value::Int(3));
        assert_eq!(run_and_get(src, "r2"), Value::Int(9));
        assert_eq!(run_and_get(src, "r3"), Value::Bool(true));
        assert_eq!(run_and_get(src, "r4"), Value::Bool(false));
    }

    #[test]
    fn struct_as_map_value_and_equality() {
        // Struct dùng được trong map (cần Hash nhất quán với PartialEq).
        let src = r#"struct P { x = 0; y = 0; }
            let m = { "a": P(1, 2) };
            let r1 = m.get("a").x;
            let r2 = P(1, 2) == P(1, 2);
            let r3 = P(1, 2) == P(9, 2);"#;
        assert_eq!(run_and_get(src, "r1"), Value::Int(1));
        assert_eq!(run_and_get(src, "r2"), Value::Bool(true));
        assert_eq!(run_and_get(src, "r3"), Value::Bool(false));
    }

    #[test]
    fn struct_errors() {
        // Quá nhiều đối số.
        let src1 = r#"struct P { x = 0; } let p = P(1, 2);"#;
        assert!(run(src1).is_err());
        // Field không tồn tại.
        let src2 = r#"struct P { x = 0; } let p = P(); let r = p.z;"#;
        let e2 = run(src2).unwrap_err();
        assert!(e2.contains("field 'z'"), "{}", e2);
        // Gán field không tồn tại.
        let src3 = r#"struct P { x = 0; } let p = P(); p.z = 1;"#;
        assert!(run(src3).is_err());
        // Gọi phương thức không có.
        let src4 = r#"struct P { x = 0; } let p = P(); p.nope();"#;
        assert!(run(src4).is_err());
    }

    #[test]
    fn struct_magic_len_must_be_int() {
        let src = r#"struct P { x = 0; func __len__() { return "abc"; } } let r = len(P());"#;
        assert!(run(src).is_err());
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

    // ----- Đồ nghề hằng ngày: chuyển đổi kiểu + file I/O mở rộng -----

    #[test]
    fn type_conversions() {
        assert_eq!(run_and_get(r#"let x = to_float("3.14");"#, "x"), Value::Float(3.14));
        assert_eq!(run_and_get("let x = to_float(2);", "x"), Value::Float(2.0));
        assert_eq!(run_and_get("let x = to_int(3.9);", "x"), Value::Int(3));
        assert_eq!(run_and_get(r#"let x = str(42);"#, "x"), Value::Str("42".into()));
        assert_eq!(run_and_get(r#"let x = str(2.5);"#, "x"), Value::Str("2.5".into()));
        assert_eq!(
            run_and_get(r#"let x = str({ "a": 1 });"#, "x"),
            Value::Str("{a: 1}".into())
        );
    }

    #[test]
    fn exists_and_append_file_and_read_lines() {
        let path = sandbox_tmp("kinet_extra_io.txt");
        let rel = "target/test_tmp/kinet_extra_io.txt";
        // exists trước khi tạo
        assert_eq!(run_and_get(&format!("let e = exists(\"{}\");", rel), "e"), Value::Bool(false));
        run(&format!("write_file(\"{}\", \"dong 1\n\");", rel)).unwrap();
        run(&format!("append_file(\"{}\", \"dong 2\n\");", rel)).unwrap();
        assert_eq!(run_and_get(&format!("let e = exists(\"{}\");", rel), "e"), Value::Bool(true));
        let content = run_and_get(&format!("let s = read_file(\"{}\");", rel), "s");
        assert_eq!(content, Value::Str("dong 1\ndong 2\n".into()));
        let lines = run_and_get(&format!("let l = read_lines(read_file(\"{}\"));", rel), "l");
        assert_eq!(
            lines,
            Value::List(vec![Value::Str("dong 1".into()), Value::Str("dong 2".into())])
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn list_dir_returns_sorted_names() {
        let dir = sandbox_tmp("kinet_dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.txt"), "1").unwrap();
        std::fs::write(dir.join("a.txt"), "2").unwrap();
        let got = run_and_get(r#"let d = list_dir("target/test_tmp/kinet_dir");"#, "d");
        assert_eq!(
            got,
            Value::List(vec![Value::Str("a.txt".into()), Value::Str("b.txt".into())])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ----- index [] / throw / bitwise / \r -----

    #[test]
    fn index_read_and_write() {
        let src = "let a = [10, 20, 30]; let x = a[1]; a[1] = 99; let y = a[1]; let n = a[0];";
        assert_eq!(run_and_get(src, "x"), Value::Int(20));
        assert_eq!(run_and_get(src, "y"), Value::Int(99));
        assert_eq!(run_and_get(src, "n"), Value::Int(10));
    }

    #[test]
    fn index_nested_grid_write() {
        let src = "let g = [[1, 2], [3, 4]]; g[1][0] = 9; let v = g[1][0];";
        assert_eq!(run_and_get(src, "v"), Value::Int(9));
    }

    #[test]
    fn index_map_write_read() {
        let src = r#"let m = { "a": 1 }; let before = m["a"]; m["b"] = 2; let b = m["b"]; let has = contains(m, "b");"#;
        assert_eq!(run_and_get(src, "before"), Value::Int(1));
        assert_eq!(run_and_get(src, "b"), Value::Int(2));
        assert_eq!(run_and_get(src, "has"), Value::Bool(true));
    }

    #[test]
    fn index_out_of_range_errors() {
        let err = run("let a = [1]; let x = a[5];").unwrap_err();
        assert!(err.contains("ngoài phạm vi"));
    }

    #[test]
    fn throw_caught_by_try() {
        let src = r#"let msg = ""; try { throw "có lỗi XYZ"; } catch e { msg = e; }"#;
        let msg = run_and_get(src, "msg");
        let Value::Str(s) = msg else {
            panic!("mong đợi chuỗi, gặp {:?}", msg);
        };
        assert!(s.contains("XYZ"));
    }

    #[test]
    fn throw_uncaught_errors() {
        let err = run(r#"throw "đầu hàng";"#).unwrap_err();
        assert!(err.contains("đầu hàng"));
    }

    #[test]
    fn bitwise_ops() {
        assert_eq!(run_and_get("let x = 6 & 3;", "x"), Value::Int(2));
        assert_eq!(run_and_get("let x = 4 | 1;", "x"), Value::Int(5));
        assert_eq!(run_and_get("let x = 1 << 4;", "x"), Value::Int(16));
        assert_eq!(run_and_get("let x = 16 >> 2;", "x"), Value::Int(4));
    }

    #[test]
    fn escape_carriage_return() {
        assert_eq!(run_and_get(r#"let x = len("a\rb");"#, "x"), Value::Int(3));
    }

    // ----- Nâng cấp: trailing comma, lặp chuỗi, round/floor/ceil/abs -----

    #[test]
    fn trailing_comma_in_literals() {
        let src = r#"let l = [1, 2, 3,];
            let m = { "a": 1, "b": 2, };"#;
        assert_eq!(run_and_get(src, "l"), Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
        assert_eq!(run_and_get(src, "m"), Value::Map(indexmap::IndexMap::from([
            (Value::Str("a".into()), Value::Int(1)),
            (Value::Str("b".into()), Value::Int(2)),
        ])));
    }

    #[test]
    fn string_repeat_star() {
        assert_eq!(run_and_get(r#"let s = "ab" * 3;"#, "s"), Value::Str("ababab".into()));
        assert_eq!(run_and_get(r#"let s = 2 * "x";"#, "s"), Value::Str("xx".into()));
        assert_eq!(run_and_get(r#"let s = "-" * 0;"#, "s"), Value::Str("".into()));
        let err = run(r#"let s = "a" * -1;"#).unwrap_err();
        assert!(err.contains("số âm") || err.contains("quá lớn"));
    }

    #[test]
    fn round_floor_ceil_abs() {
        assert_eq!(run_and_get("let x = round(2.6);", "x"), Value::Int(3));
        assert_eq!(run_and_get("let x = floor(2.9);", "x"), Value::Int(2));
        assert_eq!(run_and_get("let x = ceil(2.1);", "x"), Value::Int(3));
        assert_eq!(run_and_get("let x = abs(-7);", "x"), Value::Int(7));
        assert_eq!(run_and_get("let x = abs(-2.5);", "x"), Value::Float(2.5));
    }

    // ----- if biểu thức + assert -----

    #[test]
    fn if_as_expression() {
        assert_eq!(
            run_and_get(r#"let x = if 5 > 3 { "lon" } else { "be" };"#, "x"),
            Value::Str("lon".into())
        );
        assert_eq!(
            run_and_get(r#"let x = if 5 > 9 { 1 } else if 5 > 4 { 2 } else { 3 };"#, "x"),
            Value::Int(2)
        );
        assert_eq!(
            run_and_get(r#"let x = if false { 1 } else { 2 };"#, "x"),
            Value::Int(2)
        );
    }

    #[test]
    fn if_expr_in_lambda_and_map() {
        assert_eq!(
            run_and_get(
                r#"let r = map([1, 2, 3, 4], func(n) { return if n % 2 == 0 { "chẵn" } else { "lẻ" }; });"#,
                "r"
            ),
            Value::List(vec![
                Value::Str("lẻ".into()),
                Value::Str("chẵn".into()),
                Value::Str("lẻ".into()),
                Value::Str("chẵn".into()),
            ])
        );
    }

    #[test]
    fn assert_ok_and_fail() {
        run("assert(1 + 1 == 2);").unwrap();
        run("assert(\"abc\", \"chuỗi rỗng mới false\");").unwrap();
        let err = run(r#"assert(1 == 2, "sai rồi");"#).unwrap_err();
        assert!(err.contains("sai rồi"), "lỗi lạ: {}", err);
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