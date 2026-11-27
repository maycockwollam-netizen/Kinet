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
}

pub struct Interpreter {
    globals: EnvRef,
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
            ] {
                g.define(name, Value::Native(NativeFunc { name, func }));
            }
        }
        Self { globals }
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
            }
        }
        Ok(())
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
                for s in branch {
                    if let flow @ Flow::Return(_) = self.exec(s, env.clone())? {
                        return Ok(flow);
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::While { cond, body } => {
                while {
                    let cond_value = self.eval(cond, env.clone())?;
                    self.truthy(&cond_value)
                } {
                    for s in body {
                        if let flow @ Flow::Return(_) = self.exec(s, env.clone())? {
                            return Ok(flow);
                        }
                    }
                }
                Ok(Flow::Normal)
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
                        Value::Int(n) => Ok(Value::Int(-n)),
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
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
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
                Ok(Value::Int(a / b))
            }
            TokenKind::Percent => {
                let (a, b) = self.as_ints(&lv, &rv, op)?;
                if b == 0 {
                    return Err("Chia lấy dư cho 0".to_string());
                }
                Ok(Value::Int(a % b))
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
            TokenKind::Minus => a - b,
            TokenKind::Star => a * b,
            _ => unreachable!(),
        };
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
        for stmt in &def.body {
            if let Flow::Return(v) = self.exec(stmt, env.clone())? {
                return Ok(v);
            }
        }
        Ok(Value::Nil)
    }

    fn truthy(&self, v: &Value) -> bool {
        match v {
            Value::Bool(b) => *b,
            Value::Nil => false,
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

fn native_len(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("len() cần 1 đối số".to_string());
    }
    match &args[0] {
        Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
        Value::List(items) => Ok(Value::Int(items.len() as i64)),
        _ => Err("len() chỉ áp dụng cho chuỗi hoặc list".to_string()),
    }
}

fn native_get(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("get() cần 2 đối số (list, chỉ số)".to_string());
    }
    let Value::List(items) = &args[0] else {
        return Err("get() cần list ở đối số đầu".to_string());
    };
    let Value::Int(i) = args[1] else {
        return Err("get() cần số nguyên ở đối số thứ 2".to_string());
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

fn native_contains(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("contains() cần 2 đối số (list, x)".to_string());
    }
    let Value::List(items) = &args[0] else {
        return Err("contains() cần list ở đối số đầu".to_string());
    };
    Ok(Value::Bool(items.contains(&args[1])))
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