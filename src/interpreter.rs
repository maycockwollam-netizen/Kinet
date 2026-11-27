//! Interpreter: chạy AST (tree-walking).
//!
//! Mô hình:
//!   Value  = giá trị runtime (số, chuỗi, bool, hàm, nil)
//!   Env    = phạm vi biến, có nối tới cha (cho closure/block)
//!   Flow   = tín hiệu điều khiển (return) trong khi chạy

use std::collections::HashMap;

use crate::ast::{Expr, Pattern, Stmt};
use crate::token::TokenKind;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Str(String),
    Bool(bool),
    List(Vec<Value>),
    // Cặp (khóa, giá trị), giữ thứ tự chèn để in/lặp có thứ tự ổn định.
    Map(Vec<(Value, Value)>),
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
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => {
                // Với map, thứ tự không quan trọng — so theo tập (khóa, giá trị).
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.iter().any(|(bk, bv)| k == bk && v == bv))
            }
            (Value::Func(a), Value::Func(b)) => a.name == b.name,
            (Value::Native(a), Value::Native(b)) => a.name == b.name,
            (Value::Nil, Value::Nil) => true,
            _ => false,
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
}

//: Giới hạn độ sâu gọi hàm tránh stack overflow. Main chạy trên thread 64MB
//: (xem main.rs) nên con số này thoải mái cho đệ quy hợp lệ.
const MAX_CALL_DEPTH: usize = 2000;

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
                ("agent_run", native_agent_run as fn(&[Value]) -> Result<Value, String>),
                ("map", native_map as fn(&[Value]) -> Result<Value, String>),
                ("map_set", native_map_set as fn(&[Value]) -> Result<Value, String>),
                ("map_keys", native_map_keys as fn(&[Value]) -> Result<Value, String>),
                ("map_remove", native_map_remove as fn(&[Value]) -> Result<Value, String>),
                ("read_file", native_read_file as fn(&[Value]) -> Result<Value, String>),
                ("write_file", native_write_file as fn(&[Value]) -> Result<Value, String>),
                ("chars", native_chars as fn(&[Value]) -> Result<Value, String>),
                ("to_int", native_to_int as fn(&[Value]) -> Result<Value, String>),
                ("ord", native_ord as fn(&[Value]) -> Result<Value, String>),
            ] {
                g.define(name, Value::Native(NativeFunc { name, func }));
            }
        }
        Self {
            globals,
            call_depth: 0,
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

    fn exec(&mut self, stmt: &Stmt, env: EnvRef) -> Result<Flow, String> {
        match stmt {
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
                self.exec_block(branch, env)
            }
            Stmt::While { cond, body } => {
                loop {
                    let cond_value = self.eval(cond, env.clone())?;
                    if !self.truthy(&cond_value) {
                        break;
                    }
                    match self.exec_block(body, env.clone())? {
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
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
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
            TokenKind::Plus => match (&lv, &rv) {
                (Value::Int(a), Value::Int(b)) => (*a)
                    .checked_add(*b)
                    .map(Value::Int)
                    .ok_or_else(|| "Tràn số khi cộng".to_string()),
                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                (Value::Str(a), Value::Int(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                (Value::Int(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                _ => Err("'+' cần hai số hoặc chuỗi".to_string()),
            },
            TokenKind::Minus => self.int_arith(&lv, &rv, op),
            TokenKind::Star => self.int_arith(&lv, &rv, op),
            TokenKind::Slash => {
                let (a, b) = self.as_ints(&lv, &rv, op)?;
                if b == 0 {
                    return Err("Chia cho 0".to_string());
                }
                a.checked_div(b)
                    .map(Value::Int)
                    .ok_or_else(|| "Tràn số khi chia".to_string())
            }
            TokenKind::Percent => {
                let (a, b) = self.as_ints(&lv, &rv, op)?;
                if b == 0 {
                    return Err("Chia lấy dư cho 0".to_string());
                }
                a.checked_rem(b)
                    .map(Value::Int)
                    .ok_or_else(|| "Tràn số khi lấy dư".to_string())
            }
            TokenKind::Eq => Ok(Value::Bool(lv == rv)),
            TokenKind::NotEq => Ok(Value::Bool(lv != rv)),
            TokenKind::Lt => self.int_cmp(&lv, &rv, op),
            TokenKind::Gt => self.int_cmp(&lv, &rv, op),
            TokenKind::Le => self.int_cmp(&lv, &rv, op),
            TokenKind::Ge => self.int_cmp(&lv, &rv, op),
            _ => unreachable!(),
        }
    }

    fn int_arith(&self, l: &Value, r: &Value, op: &TokenKind) -> Result<Value, String> {
        let (a, b) = self.as_ints(l, r, op)?;
        let result = match op {
            TokenKind::Minus => a
                .checked_sub(b)
                .ok_or_else(|| "Tràn số khi trừ".to_string()),
            TokenKind::Star => a
                .checked_mul(b)
                .ok_or_else(|| "Tràn số khi nhân".to_string()),
            _ => unreachable!(),
        }?;
        Ok(Value::Int(result))
    }

    fn int_cmp(&self, l: &Value, r: &Value, op: &TokenKind) -> Result<Value, String> {
        let (a, b) = self.as_ints(l, r, op)?;
        let result = match op {
            TokenKind::Lt => a < b,
            TokenKind::Gt => a > b,
            TokenKind::Le => a <= b,
            TokenKind::Ge => a >= b,
            _ => unreachable!(),
        };
        Ok(Value::Bool(result))
    }

    fn as_ints(&self, l: &Value, r: &Value, op: &TokenKind) -> Result<(i64, i64), String> {
        let (&Value::Int(a), &Value::Int(b)) = (l, r) else {
            return Err(format!("Toán tử '{}' cần hai số", self.op_symbol(op)));
        };
        Ok((a, b))
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
            Value::Native(n) => return (n.func)(&args),
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
        let env = std::rc::Rc::new(std::cell::RefCell::new(Env::child(def.closure.clone())));
        for (param, arg) in def.params.iter().zip(args) {
            env.borrow_mut().define(param, arg);
        }
        self.call_depth += 1;
        let result = self.exec_block(&def.body, env);
        self.call_depth -= 1;
        let flow = result?;
        match flow {
            Flow::Return(v) => Ok(v),
            Flow::Break | Flow::Continue => Err(format!(
                "'break'/'continue' không hợp lệ trong hàm '{}'",
                def.name
            )),
            Flow::Normal => Ok(Value::Nil),
        }
    }

    // Chuẩn Python: 0, chuỗi rỗng, list rỗng là falsy — quan trọng cho agent
    // đọc exit code: `if exit_code` phải đúng nghĩa "code khác 0".
    fn truthy(&self, v: &Value) -> bool {
        match v {
            Value::Bool(b) => *b,
            Value::Nil => false,
            Value::Int(n) => *n != 0,
            Value::Str(s) => !s.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Map(entries) => !entries.is_empty(),
            _ => true,
        }
    }

    fn to_string(&self, v: &Value) -> String {
        match v {
            Value::Int(n) => n.to_string(),
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

// ---------- Bridge sang Kinetic SDK ----------

/// agent_run(goal) — điều khiển agent Kinetic SDK thật.
///
/// Gọi `python3 <bridge>/kinet_agent.py <goal>` (SDK nằm tại `KINET_SDK_DIR`,
/// mặc định `/tmp/kinetic-sdk`). Cấu hình được qua biến môi trường `KINET_SDK_DIR`
/// và `KINET_BRIDGE`; đọc thêm `OPENAI_API_KEY` để dùng LLM thật nếu muốn.
fn native_agent_run(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("agent_run() cần 1 đối số (goal)".to_string());
    }
    let Value::Str(goal) = &args[0] else {
        return Err("agent_run() cần chuỗi goal ở đối số đầu".to_string());
    };

    let sdk_dir = std::env::var("KINET_SDK_DIR")
        .unwrap_or_else(|_| "/tmp/kinetic-sdk".to_string());
    let bridge = std::env::var("KINET_BRIDGE").unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|d| d.join("bridge").join("kinet_agent.py"))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "bridge/kinet_agent.py".to_string())
    });

    let output = std::process::Command::new("python3")
        .arg(&bridge)
        .arg(goal)
        .env("PYTHONPATH", &sdk_dir)
        .output()
        .map_err(|e| format!("agent_run() không chạy được python3: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        return Err(if msg.is_empty() {
            "agent_run() thất bại (SDK trả lỗi)".to_string()
        } else {
            format!("agent_run() thất bại: {}", msg)
        });
    }

    Ok(Value::Str(String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

// ---------- Hàm native (builtin) ----------

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

// Tìm vị trí của khóa trong map (theo thứ tự chèn). Khóa so bằng Value::eq.
fn map_index(entries: &[(Value, Value)], key: &Value) -> Option<usize> {
    entries.iter().position(|(k, _)| k == key)
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
        Value::Map(entries) => match map_index(entries, &args[1]) {
            Some(i) => Ok(entries[i].1.clone()),
            None => Err(format!("get(map, khóa) — không có khóa này trong map")),
        },
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
        Value::Map(entries) => Ok(Value::Bool(map_index(entries, &args[1]).is_some())),
        _ => Err("contains() cần list, chuỗi hoặc map ở đối số đầu".to_string()),
    }
}

fn native_map(args: &[Value]) -> Result<Value, String> {
    if args.len() % 2 != 0 {
        return Err("map() cần số chẵn đối số (khóa, giá trị, khóa, giá trị, ...)".to_string());
    }
    let mut entries: Vec<(Value, Value)> = Vec::with_capacity(args.len() / 2);
    let mut i = 0;
    while i < args.len() {
        let key = args[i].clone();
        let value = args[i + 1].clone();
        // Khóa trùng thì ghi đè thay vì thêm bản thứ hai.
        if let Some(idx) = map_index(&entries, &key) {
            entries[idx].1 = value;
        } else {
            entries.push((key, value));
        }
        i += 2;
    }
    Ok(Value::Map(entries))
}

fn native_map_set(args: &[Value]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err("map_set() cần 3 đối số (map, khóa, giá trị)".to_string());
    }
    let Value::Map(entries) = &args[0] else {
        return Err("map_set() cần map ở đối số đầu".to_string());
    };
    let mut out = entries.clone();
    let key = args[1].clone();
    if let Some(idx) = map_index(&out, &key) {
        out[idx].1 = args[2].clone();
    } else {
        out.push((key, args[2].clone()));
    }
    Ok(Value::Map(out))
}

fn native_map_keys(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("map_keys() cần 1 đối số".to_string());
    }
    let Value::Map(entries) = &args[0] else {
        return Err("map_keys() cần map ở đối số đầu".to_string());
    };
    Ok(Value::List(entries.iter().map(|(k, _)| k.clone()).collect()))
}

fn native_chars(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("chars() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("chars() cần chuỗi".to_string());
    };
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

fn native_ord(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("ord() cần 1 đối số".to_string());
    }
    let Value::Str(s) = &args[0] else {
        return Err("ord() cần chuỗi 1 ký tự".to_string());
    };
    match s.chars().next() {
        Some(c) => Ok(Value::Int(c as i64)),
        None => Err("ord(): chuỗi rỗng".to_string()),
    }
}

fn native_map_remove(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("map_remove() cần 2 đối số (map, khóa)".to_string());
    }
    let Value::Map(entries) = &args[0] else {
        return Err("map_remove() cần map ở đối số đầu".to_string());
    };
    let out: Vec<(Value, Value)> = entries
        .iter()
        .filter(|(k, _)| k != &args[1])
        .cloned()
        .collect();
    Ok(Value::Map(out))
}

fn native_read_file(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("read_file() cần 1 đối số (đường dẫn)".to_string());
    }
    let Value::Str(path) = &args[0] else {
        return Err("read_file() cần chuỗi đường dẫn".to_string());
    };
    let content = std::fs::read_to_string(path)
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
    std::fs::write(path, content)
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
    let mut out = Vec::new();
    if step > 0 {
        let mut i = start;
        while i < end {
            out.push(Value::Int(i));
            i += step;
        }
    } else {
        let mut i = start;
        while i > end {
            out.push(Value::Int(i));
            i += step;
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
        let src = r#"let m = map("a", 1, "b", 2);
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
        let src = r#"let inner = map("x", 10); let outer = map("p", inner); let v = get(get(outer, "p"), "x");"#;
        assert_eq!(run_and_get(src, "v"), Value::Int(10));
    }

    #[test]
    fn map_missing_key_errors() {
        let err = run(r#"let m = map("a", 1); let v = get(m, "zz");"#).unwrap_err();
        assert!(err.contains("khóa"));
    }

    #[test]
    fn map_dup_key_overwrites() {
        assert_eq!(
            run_and_get(r#"let m = map("a", 1, "a", 2);"#, "m"),
            Value::Map(vec![(Value::Str("a".into()), Value::Int(2))])
        );
    }

    // ----- file I/O (M4.2) -----

    #[test]
    fn file_io_roundtrip() {
        let path = std::env::temp_dir().join("kinet_m4_io.tmp");
        let path_str = path.display().to_string();
        run(&format!("write_file(\"{}\", \"hello demo\");", path_str)).unwrap();
        let got = run_and_get(&format!("let s = read_file(\"{}\");", path_str), "s");
        assert_eq!(got, Value::Str("hello demo".into()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_file_missing_errors() {
        let err = run(r#"let s = read_file("/no/such/file_kinet_xyz");"#).unwrap_err();
        assert!(err.contains("thất bại"));
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
    let mut out = items.clone();
    out.push(args[1].clone());
    Ok(Value::List(out))
}