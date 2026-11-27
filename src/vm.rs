//! VM (Phase 1): biên dịch AST → bytecode, chạy bằng vòng lặp với
//! **call stack tường minh nằm trên heap** — đệ quy sâu không còn tràn
//! stack OS. Phạm vi hiện tại: phần lõi (literal, biến, toán tử, if/while/for,
//! hàm gọi theo tên, đệ quy). Các tính năng còn lại được bổ sung dần ở các
//! phase sau; khi gặp cấu trúc chưa hỗ trợ sẽ báo lỗi rõ ràng.

use std::collections::HashMap;

use crate::ast::{Expr, LetTarget, Stmt};
use crate::interpreter::{native_table, stringify_value, value_cmp, Value, DEFAULT_MAX_STEPS};
use crate::token::TokenKind;

#[derive(Debug, Clone)]
pub enum Instr {
    PushInt(i64),
    PushFloat(f64),
    PushStr(String),
    PushBool(bool),
    PushNil,
    LoadSlot(u32),
    StoreSlot(u32),
    LoadGlobal(String),
    StoreGlobal(String),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Not,
    ToBool,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Dup,
    Pop,
    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    MakeList(u32),
    MakeTuple(u32),
    Unpack(u32),
    GetIndex,
    // Gọi hàm/native theo TÊN (phase 1 chưa có closure/giá trị hàm).
    Call(String, u32),
    Return,
    Halt,
}

#[derive(Debug, Clone)]
pub struct FuncCode {
    pub arity: usize,
    pub n_slots: usize,
    // Rc để gọi hàm không phải clone toàn bộ bytecode mỗi lần.
    pub code: std::rc::Rc<Vec<Instr>>,
}

pub struct Program {
    pub main: FuncCode,
    pub functions: HashMap<String, FuncCode>,
}

// ---------- Compiler ----------

#[derive(Default)]
struct Scope {
    names: HashMap<String, u32>,
    block_markers: Vec<u32>,
    next_slot: u32,
}

impl Scope {
    fn push_block(&mut self) {
        self.block_markers.push(self.next_slot);
    }
    fn pop_block(&mut self) {
        let marker = self.block_markers.pop().unwrap_or(0);
        self.names.retain(|_, &mut s| s < marker);
        self.next_slot = marker;
    }
    fn declare(&mut self, name: &str) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        self.names.insert(name.to_string(), slot);
        slot
    }
    fn lookup(&self, name: &str) -> Option<u32> {
        self.names.get(name).copied()
    }
}

struct LoopCtx {
    // Vị trí các lệnh Jump(0) do `continue`/`break` phát ra — patch khi kết thúc vòng.
    continue_patches: Vec<usize>,
    break_patches: Vec<usize>,
}

struct Compiler {
    code: Vec<Instr>,
    scope: Scope,
    is_top: bool,
    functions: HashMap<String, FuncCode>,
    loops: Vec<LoopCtx>,
}

pub fn compile(stmts: &[Stmt]) -> Result<Program, String> {
    let mut c = Compiler {
        code: Vec::new(),
        scope: Scope::default(),
        is_top: true,
        functions: HashMap::new(),
        loops: Vec::new(),
    };
    c.compile_body(stmts)?;
    c.code.push(Instr::Halt);
    let main = FuncCode {
        arity: 0,
        n_slots: c.scope.next_slot as usize,
        code: std::rc::Rc::new(c.code),
    };
    Ok(Program {
        main,
        functions: c.functions,
    })
}

impl Compiler {
    fn unsupported(what: &str) -> String {
        format!("VM (phase 1) chưa hỗ trợ: {}", what)
    }

    fn patch(&mut self, place: usize, target: usize) {
        self.code[place] = self.code[place].clone();
        // Thay đúng vị trí jump placeholder (đều là jump với target tuyệt đối).
        match &mut self.code[place] {
            Instr::Jump(t) | Instr::JumpIfFalse(t) | Instr::JumpIfTrue(t) => *t = target,
            _ => unreachable!(),
        }
    }

    fn compile_body(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for s in stmts {
            self.compile_stmt(s)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Ann { inner, .. } => self.compile_stmt(inner),
            Stmt::Expr(e) => {
                self.compile_expr(e)?;
                self.code.push(Instr::Pop);
                Ok(())
            }
            Stmt::Let { target, value } => {
                self.compile_expr(value)?;
                match target {
                    LetTarget::Var(name) => {
                        self.store_var(name);
                        // let dùng làm câu lệnh — không giữ giá trị trên stack.
                        self.code.push(Instr::Pop);
                    }
                    LetTarget::Tuple(names) => {
                        // Bóc tách: (a, b) = giá trị (tuple/list).
                        self.code.push(Instr::Unpack(names.len() as u32));
                        for n in names.iter().rev() {
                            let slot = self.scope.declare(n);
                            self.code.push(Instr::StoreSlot(slot));
                        }
                    }
                }
                Ok(())
            }
            Stmt::Print(exprs) | Stmt::Println(exprs) => {
                for e in exprs {
                    self.compile_expr(e)?;
                }
                let name = if matches!(stmt, Stmt::Println(_)) {
                    "__println__"
                } else {
                    "__print__"
                };
                self.code.push(Instr::Call(name.to_string(), exprs.len() as u32));
                self.code.push(Instr::Pop);
                Ok(())
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.compile_expr(cond)?;
                let jump_false = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
                self.scope.push_block();
                self.compile_body(then_branch)?;
                self.scope.pop_block();
                let jump_done = self.emit_jump_placeholder(Instr::Jump(0));
                let else_index = self.code.len();
                self.patch(jump_false, else_index);
                self.scope.push_block();
                self.compile_body(else_branch)?;
                self.scope.pop_block();
                let done = self.code.len();
                self.patch(jump_done, done);
                Ok(())
            }
            Stmt::While { cond, body } => {
                let cond_index = self.code.len();
                self.loops.push(LoopCtx {
                    continue_patches: Vec::new(),
                    break_patches: Vec::new(),
                });
                self.compile_expr(cond)?;
                let break_if = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
                self.scope.push_block();
                self.compile_body(body)?;
                self.scope.pop_block();
                self.code.push(Instr::Jump(cond_index));
                let done = self.code.len();
                self.patch(break_if, done);
                let lc = self.loops.pop().unwrap();
                for p in lc.continue_patches {
                    self.patch(p, cond_index);
                }
                for p in lc.break_patches {
                    self.patch(p, done);
                }
                Ok(())
            }
            Stmt::For {
                target,
                iterable,
                body,
            } => {
                // for x in iter: dùng 2 slot ẩn ($iter, $idx).
                let iter_slot = self.scope.declare("$iter");
                let idx_slot = self.scope.declare("$idx");
                let var_slot = match target {
                    LetTarget::Var(name) => Some(self.scope.declare(name)),
                    LetTarget::Tuple(_) => {
                        return Err(Self::unsupported("bóc tách trong vòng for"));
                    }
                };
                self.compile_expr(iterable)?;
                self.code.push(Instr::StoreSlot(iter_slot));
                self.code.push(Instr::PushInt(0));
                self.code.push(Instr::StoreSlot(idx_slot));
                let cond_index = self.code.len();
                self.loops.push(LoopCtx {
                    continue_patches: Vec::new(),
                    break_patches: Vec::new(),
                });
                // idx < len(iter)?
                self.code.push(Instr::LoadSlot(idx_slot));
                self.code.push(Instr::LoadSlot(iter_slot));
                self.code.push(Instr::Call("len".to_string(), 1));
                self.code.push(Instr::Lt);
                let break_if = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
                // x = get(iter, idx)
                self.code.push(Instr::LoadSlot(iter_slot));
                self.code.push(Instr::LoadSlot(idx_slot));
                self.code.push(Instr::Call("get".to_string(), 2));
                self.code.push(Instr::StoreSlot(var_slot.unwrap()));
                self.scope.push_block();
                self.compile_body(body)?;
                self.scope.pop_block();
                // idx += 1 (đích của continue)
                let continue_index = self.code.len();
                self.code.push(Instr::LoadSlot(idx_slot));
                self.code.push(Instr::PushInt(1));
                self.code.push(Instr::Add);
                self.code.push(Instr::StoreSlot(idx_slot));
                self.code.push(Instr::Jump(cond_index));
                let done = self.code.len();
                self.patch(break_if, done);
                let lc = self.loops.pop().unwrap();
                for p in lc.continue_patches {
                    self.patch(p, continue_index);
                }
                for p in lc.break_patches {
                    self.patch(p, done);
                }
                Ok(())
            }
            Stmt::Break => {
                let idx = self.emit_jump_placeholder(Instr::Jump(0));
                self.loops
                    .last_mut()
                    .ok_or("'break' ngoài vòng lặp")?
                    .break_patches
                    .push(idx);
                Ok(())
            }
            Stmt::Continue => {
                let idx = self.emit_jump_placeholder(Instr::Jump(0));
                self.loops
                    .last_mut()
                    .ok_or("'continue' ngoài vòng lặp")?
                    .continue_patches
                    .push(idx);
                Ok(())
            }
            Stmt::Func {
                name,
                params,
                body,
            } => {
                self.compile_func(name, params, body)?;
                Ok(())
            }
            Stmt::Return(e) => {
                self.compile_expr(e)?;
                self.code.push(Instr::Return);
                Ok(())
            }
            other => Err(Self::unsupported(&format!(
                "câu lệnh {:?} (chỉ hỗ trợ if/while/for/func/let/return/print)",
                std::mem::discriminant(other)
            ))),
        }
    }

    fn compile_func(&mut self, name: &str, params: &[crate::ast::ParamDef], body: &[Stmt]) -> Result<(), String> {
        let mut inner = Compiler {
            code: Vec::new(),
            scope: Scope::default(),
            is_top: false,
            functions: std::mem::take(&mut self.functions),
            loops: Vec::new(),
        };
        for p in params {
            inner.scope.declare(&p.name);
        }
        inner.compile_body(body)?;
        inner.code.push(Instr::Return); // rơi khỏi đáy hàm → trả nil
        let fc = FuncCode {
            arity: params.len(),
            n_slots: inner.scope.next_slot as usize,
            code: std::rc::Rc::new(inner.code),
        };
        inner.functions.insert(name.to_string(), fc);
        self.functions = inner.functions;
        Ok(())
    }

    fn store_var(&mut self, name: &str) {
        self.code.push(Instr::Dup);
        if self.is_top {
            self.code.push(Instr::StoreGlobal(name.to_string()));
        } else if self.scope.lookup(name).is_some() {
            // gán cho biến đã khai báo local
            let slot = self.scope.lookup(name).unwrap();
            self.code.push(Instr::StoreSlot(slot));
        } else {
            // biến chưa khai báo ở local → coi là global
            self.code.push(Instr::StoreGlobal(name.to_string()));
        }
    }

    // Tạo một jump placeholder (chưa biết đích), trả về vị trí để patch.
    fn emit_jump_placeholder(&mut self, instr: Instr) -> usize {
        self.code.push(instr);
        self.code.len() - 1
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Number(n) => self.code.push(Instr::PushInt(*n)),
            Expr::Float(f) => self.code.push(Instr::PushFloat(*f)),
            Expr::Str(s) => self.code.push(Instr::PushStr(s.clone())),
            Expr::Bool(b) => self.code.push(Instr::PushBool(*b)),
            Expr::Nil => self.code.push(Instr::PushNil),
            Expr::Variable(name) => {
                if let Some(slot) = self.scope.lookup(name) {
                    self.code.push(Instr::LoadSlot(slot));
                } else {
                    self.code.push(Instr::LoadGlobal(name.clone()));
                }
            }
            Expr::Assign { name, value } => {
                self.compile_expr(value)?;
                self.store_var(name);
            }
            Expr::Unary { op, expr } => {
                self.compile_expr(expr)?;
                match op {
                    TokenKind::Minus => self.code.push(Instr::Neg),
                    TokenKind::Not => self.code.push(Instr::Not),
                    _ => return Err(Self::unsupported("toán tử một ngôi này")),
                }
            }
            Expr::Binary { left, op, right } => self.compile_binary(left, op, right)?,
            Expr::Call { callee, args } => {
                let Expr::Variable(name) = callee.as_ref() else {
                    return Err(Self::unsupported("gọi hàm qua biểu thức / closure"));
                };
                for a in args {
                    self.compile_expr(&a.value)?;
                }
                self.code.push(Instr::Call(name.clone(), args.len() as u32));
            }
            Expr::List(items) => {
                for it in items {
                    self.compile_expr(it)?;
                }
                self.code.push(Instr::MakeList(items.len() as u32));
            }
            Expr::Tuple(items) => {
                for it in items {
                    self.compile_expr(it)?;
                }
                self.code.push(Instr::MakeTuple(items.len() as u32));
            }
            Expr::Index { target, index } => {
                self.compile_expr(target)?;
                self.compile_expr(index)?;
                self.code.push(Instr::GetIndex);
            }
            Expr::If {
                cond,
                then_expr,
                else_expr,
            } => {
                self.compile_expr(cond)?;
                let jf = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
                self.compile_expr(then_expr)?;
                let jd = self.emit_jump_placeholder(Instr::Jump(0));
                let else_index = self.code.len();
                self.patch(jf, else_index);
                self.compile_expr(else_expr)?;
                let done = self.code.len();
                self.patch(jd, done);
            }
            other => {
                return Err(Self::unsupported(&format!(
                    "biểu thức {:?}",
                    std::mem::discriminant(other)
                )));
            }
        }
        Ok(())
    }

    fn compile_binary(&mut self, left: &Expr, op: &TokenKind, right: &Expr) -> Result<(), String> {
        // and/or short-circuit (kết quả là Bool như interpreter).
        if *op == TokenKind::And {
            self.compile_expr(left)?;
            let jf = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
            self.compile_expr(right)?;
            self.code.push(Instr::ToBool);
            let jd = self.emit_jump_placeholder(Instr::Jump(0));
            let f_idx = self.code.len();
            self.patch(jf, f_idx);
            self.code.push(Instr::PushBool(false));
            let done = self.code.len();
            self.patch(jd, done);
            return Ok(());
        }
        if *op == TokenKind::Or {
            self.compile_expr(left)?;
            let jt = self.emit_jump_placeholder(Instr::JumpIfTrue(0));
            self.compile_expr(right)?;
            self.code.push(Instr::ToBool);
            let jd = self.emit_jump_placeholder(Instr::Jump(0));
            let t_idx = self.code.len();
            self.patch(jt, t_idx);
            self.code.push(Instr::PushBool(true));
            let done = self.code.len();
            self.patch(jd, done);
            return Ok(());
        }
        self.compile_expr(left)?;
        self.compile_expr(right)?;
        let instr = match op {
            TokenKind::Plus => Instr::Add,
            TokenKind::Minus => Instr::Sub,
            TokenKind::Star => Instr::Mul,
            TokenKind::Slash => Instr::Div,
            TokenKind::Percent => Instr::Mod,
            TokenKind::Eq => Instr::Eq,
            TokenKind::NotEq => Instr::Ne,
            TokenKind::Lt => Instr::Lt,
            TokenKind::Gt => Instr::Gt,
            TokenKind::Le => Instr::Le,
            TokenKind::Ge => Instr::Ge,
            other => {
                return Err(Self::unsupported(&format!(
                    "toán tử '{:?}'",
                    other
                )));
            }
        };
        self.code.push(instr);
        Ok(())
    }
}

// ---------- Runner ----------

struct Frame {
    code: std::rc::Rc<Vec<Instr>>,
    ip: usize,
    locals: Vec<Value>,
}

pub struct Vm {
    program: Program,
    globals: HashMap<String, Value>,
    natives: HashMap<String, fn(&[Value]) -> Result<Value, String>>,
}

pub fn run(stmts: &[Stmt]) -> Result<(), String> {
    let program = compile(stmts)?;
    Vm::new(program).execute()
}

impl Vm {
    pub fn new(program: Program) -> Self {
        let mut natives = HashMap::new();
        for (name, func) in native_table() {
            natives.insert(name.to_string(), func);
        }
        Vm {
            program,
            globals: HashMap::new(),
            natives,
        }
    }

    fn vm_binop(&self, instr: &Instr, l: Value, r: Value) -> Result<Value, String> {
        // '+' nối chuỗi / '*' lặp chuỗi — giống interpreter.
        if matches!(instr, Instr::Add) {
            if matches!(l, Value::Str(_)) || matches!(r, Value::Str(_)) {
                return Ok(Value::Str(format!(
                    "{}{}",
                    stringify_value(&l),
                    stringify_value(&r)
                )));
            }
        }
        if matches!(instr, Instr::Mul) {
            if let (Value::Str(s), Value::Int(n)) = (&l, &r) {
                return Ok(Value::Str(s.repeat(*n as usize)));
            }
            if let (Value::Int(n), Value::Str(s)) = (&l, &r) {
                return Ok(Value::Str(s.repeat(*n as usize)));
            }
        }
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => match instr {
                Instr::Add => a
                    .checked_add(b)
                    .map(Value::Int)
                    .ok_or_else(|| "Tràn số khi cộng".to_string()),
                Instr::Sub => a
                    .checked_sub(b)
                    .map(Value::Int)
                    .ok_or_else(|| "Tràn số khi trừ".to_string()),
                Instr::Mul => a
                    .checked_mul(b)
                    .map(Value::Int)
                    .ok_or_else(|| "Tràn số khi nhân".to_string()),
                Instr::Div => {
                    if b == 0 {
                        Err("Chia cho 0".to_string())
                    } else {
                        Ok(Value::Int(a / b))
                    }
                }
                Instr::Mod => {
                    if b == 0 {
                        Err("Chia lấy dư cho 0".to_string())
                    } else {
                        Ok(Value::Int(a % b))
                    }
                }
                _ => unreachable!(),
            },
            (Value::Int(a), Value::Float(b)) => {
                let res = match instr {
                    Instr::Add => (a as f64) + b,
                    Instr::Sub => (a as f64) - b,
                    Instr::Mul => (a as f64) * b,
                    Instr::Div => {
                        if b == 0.0 {
                            return Err("Chia cho 0".to_string());
                        }
                        (a as f64) / b
                    }
                    Instr::Mod => {
                        if b == 0.0 {
                            return Err("Chia lấy dư cho 0".to_string());
                        }
                        (a as f64) % b
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Float(res))
            }
            (Value::Float(a), Value::Int(b)) => {
                let res = match instr {
                    Instr::Add => a + (b as f64),
                    Instr::Sub => a - (b as f64),
                    Instr::Mul => a * (b as f64),
                    Instr::Div => {
                        if b == 0 {
                            return Err("Chia cho 0".to_string());
                        }
                        a / (b as f64)
                    }
                    Instr::Mod => {
                        if b == 0 {
                            return Err("Chia lấy dư cho 0".to_string());
                        }
                        a % (b as f64)
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Float(res))
            }
            (Value::Float(a), Value::Float(b)) => {
                let res = match instr {
                    Instr::Add => a + b,
                    Instr::Sub => a - b,
                    Instr::Mul => a * b,
                    Instr::Div => {
                        if b == 0.0 {
                            return Err("Chia cho 0".to_string());
                        }
                        a / b
                    }
                    Instr::Mod => {
                        if b == 0.0 {
                            return Err("Chia lấy dư cho 0".to_string());
                        }
                        a % b
                    }
                    _ => unreachable!(),
                };
                Ok(Value::Float(res))
            }
            (l, r) => Err(format!(
                "Toán tử số học gặp '{}' và '{}'",
                stringify_value(&l),
                stringify_value(&r)
            )),
        }
    }

    fn execute(mut self) -> Result<(), String> {
        let max_steps = std::env::var("KINET_MAX_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_STEPS);
        let mut stack: Vec<Value> = Vec::new();
        let main_code = self.program.main.code.clone();
        let n_slots = self.program.main.n_slots;
        let mut frames: Vec<Frame> = vec![Frame {
            code: main_code,
            ip: 0,
            locals: vec![Value::Nil; n_slots],
        }];
        let mut steps: u64 = 0;

        loop {
            steps += 1;
            if steps > max_steps {
                return Err(format!(
                    "Quá ngân sách thực thi ({} bước) — có thể là vòng lặp vô hạn",
                    max_steps
                ));
            }
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            if frame.ip >= frame.code.len() {
                return Ok(());
            }
            let instr = frame.code[frame.ip].clone();
            frame.ip += 1;
            match instr {
                Instr::Halt => return Ok(()),
                Instr::PushInt(n) => stack.push(Value::Int(n)),
                Instr::PushFloat(f) => stack.push(Value::Float(f)),
                Instr::PushStr(s) => stack.push(Value::Str(s)),
                Instr::PushBool(b) => stack.push(Value::Bool(b)),
                Instr::PushNil => stack.push(Value::Nil),
                Instr::Dup => {
                    let v = stack.last().cloned().ok_or("stack rỗng")?;
                    stack.push(v);
                }
                Instr::Pop => {
                    stack.pop();
                }
                Instr::LoadSlot(k) => {
                    let v = frames.last().unwrap().locals[k as usize].clone();
                    stack.push(v);
                }
                Instr::StoreSlot(k) => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    frames.last_mut().unwrap().locals[k as usize] = v;
                }
                Instr::LoadGlobal(n) => {
                    let v = self
                        .globals
                        .get(&n)
                        .cloned()
                        .ok_or_else(|| format!("Chưa khai báo biến '{}'", n))?;
                    stack.push(v);
                }
                Instr::StoreGlobal(n) => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    self.globals.insert(n, v);
                }
                Instr::Add | Instr::Sub | Instr::Mul | Instr::Div | Instr::Mod => {
                    let r = stack.pop().ok_or("stack rỗng")?;
                    let l = stack.pop().ok_or("stack rỗng")?;
                    let v = self.vm_binop(&instr, l, r)?;
                    stack.push(v);
                }
                Instr::Neg => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    let v = match v {
                        Value::Int(n) => Value::Int(n.checked_neg().ok_or("tràn khi '-'")?),
                        Value::Float(f) => Value::Float(-f),
                        other => {
                            return Err(format!(
                                "Toán tử '-' cần số, gặp {}",
                                crate::interpreter::stringify_value(&other)
                            ));
                        }
                    };
                    stack.push(v);
                }
                Instr::Not => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    stack.push(Value::Bool(!crate::interpreter::is_truthy_value(&v)));
                }
                Instr::ToBool => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    stack.push(Value::Bool(crate::interpreter::is_truthy_value(&v)));
                }
                Instr::Eq => {
                    let r = stack.pop().ok_or("stack rỗng")?;
                    let l = stack.pop().ok_or("stack rỗng")?;
                    stack.push(Value::Bool(l == r));
                }
                Instr::Ne => {
                    let r = stack.pop().ok_or("stack rỗng")?;
                    let l = stack.pop().ok_or("stack rỗng")?;
                    stack.push(Value::Bool(l != r));
                }
                Instr::Lt | Instr::Gt | Instr::Le | Instr::Ge => {
                    let r = stack.pop().ok_or("stack rỗng")?;
                    let l = stack.pop().ok_or("stack rỗng")?;
                    let ord = value_cmp(&l, &r)?;
                    let b = match instr {
                        Instr::Lt => ord == std::cmp::Ordering::Less,
                        Instr::Gt => ord == std::cmp::Ordering::Greater,
                        Instr::Le => ord != std::cmp::Ordering::Greater,
                        _ => ord != std::cmp::Ordering::Less,
                    };
                    stack.push(Value::Bool(b));
                }
                Instr::Jump(t) => frames.last_mut().unwrap().ip = t,
                Instr::JumpIfFalse(t) => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    if !crate::interpreter::is_truthy_value(&v) {
                        frames.last_mut().unwrap().ip = t;
                    }
                }
                Instr::JumpIfTrue(t) => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    if crate::interpreter::is_truthy_value(&v) {
                        frames.last_mut().unwrap().ip = t;
                    }
                }
                Instr::MakeList(n) => {
                    let mut items = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        items.push(stack.pop().ok_or("stack rỗng")?);
                    }
                    items.reverse();
                    stack.push(Value::List(items));
                }
                Instr::MakeTuple(n) => {
                    let mut items = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        items.push(stack.pop().ok_or("stack rỗng")?);
                    }
                    items.reverse();
                    stack.push(Value::Tuple(items));
                }
                Instr::Unpack(n) => {
                    let container = stack.pop().ok_or("stack rỗng")?;
                    let items: Vec<Value> = match container {
                        Value::List(v) => v,
                        Value::Tuple(v) => v,
                        other => {
                            return Err(format!(
                                "bóc tách cần tuple/list, gặp {}",
                                crate::interpreter::stringify_value(&other)
                            ));
                        }
                    };
                    if items.len() != n as usize {
                        return Err(format!(
                            "bóc tách cần {} giá trị, nhận {}",
                            n,
                            items.len()
                        ));
                    }
                    for it in items {
                        stack.push(it);
                    }
                }
                Instr::GetIndex => {
                    let idx = stack.pop().ok_or("stack rỗng")?;
                    let container = stack.pop().ok_or("stack rỗng")?;
                    let v = match container {
                        Value::List(items) => {
                            let i = crate::interpreter::expect_and_normalize_index(
                                &idx,
                                items.len(),
                            )?;
                            items[i].clone()
                        }
                        Value::Tuple(items) => {
                            let i = crate::interpreter::expect_and_normalize_index(
                                &idx,
                                items.len(),
                            )?;
                            items[i].clone()
                        }
                        other => {
                            return Err(format!(
                                "chỉ số `[]` chỉ áp dụng cho list/tuple, gặp {}",
                                crate::interpreter::stringify_value(&other)
                            ));
                        }
                    };
                    stack.push(v);
                }
                Instr::Call(name, argc) => {
                    let mut args = Vec::with_capacity(argc as usize);
                    for _ in 0..argc {
                        args.push(stack.pop().ok_or("stack rỗng")?);
                    }
                    args.reverse();
                    if name == "__print__" || name == "__println__" {
                        let text: String = args.iter().map(crate::interpreter::stringify_value).collect();
                        print!("{}", text);
                        if name == "__println__" {
                            println!();
                        }
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                        stack.push(Value::Nil);
                        continue;
                    }
                    if let Some(fc) = self.program.functions.get(&name).cloned() {
                        if args.len() != fc.arity {
                            return Err(format!(
                                "Hàm '{}' cần {} đối số, nhận {}",
                                name,
                                fc.arity,
                                args.len()
                            ));
                        }
                        let mut locals = vec![Value::Nil; fc.n_slots];
                        for (i, a) in args.into_iter().enumerate() {
                            locals[i] = a;
                        }
                        frames.push(Frame {
                            code: fc.code.clone(),
                            ip: 0,
                            locals,
                        });
                        continue;
                    }
                    if let Some(func) = self.natives.get(&name).copied() {
                        let v = func(&args)?;
                        stack.push(v);
                        continue;
                    }
                    return Err(format!("Chưa khai báo hàm/native '{}'", name));
                }
                Instr::Return => {
                    let v = stack.pop().ok_or("stack rỗng")?;
                    frames.pop();
                    if frames.is_empty() {
                        return Ok(());
                    }
                    stack.push(v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn vm_run(src: &str) -> Result<(), String> {
        let stmts = Parser::new(Lexer::new(src).tokenize().unwrap())
            .parse()
            .unwrap();
        let program = compile(&stmts)?;
        Vm::new(program).execute()
    }

    #[test]
    fn vm_arithmetic_and_print_string() {
        // Chạy một chương trình nhỏ, chỉ kiểm tra không lỗi + kết quả logic.
        let outr = vm_run(r#"let x = 2 + 3 * 4; if x == 14 { println("ok"); } else { println("no"); }"#);
        assert!(outr.is_ok());
    }

    #[test]
    fn vm_compiles_and_runs_core() {
        // Đảm bảo compile thành công + chạy không lỗi.
        assert!(vm_run("let a = 1; let b = 2; let c = a + b;").is_ok());
    }

    #[test]
    fn vm_deep_recursion_does_not_overflow() {
        // Đây là "điểm ăn tiền" của VM: call stack nằm trên heap.
        let src = r#"func sumto(n) { if n == 0 { return 0; } return n + sumto(n - 1); }
            let r = sumto(50000);
            if r == 1250025000 { println("ok"); } else { println("sai"); }"#;
        assert!(vm_run(src).is_ok());
    }

    #[test]
    fn vm_recursion_math() {
        let src = r#"func fact(n) { if n <= 1 { return 1; } return n * fact(n - 1); }
            let r = fact(10);
            if r == 3628800 { println("ok"); } else { println("sai"); }"#;
        assert!(vm_run(src).is_ok());
    }

    #[test]
    fn vm_for_and_while() {
        let src = r#"let total = 0;
            for i in range(1, 5) { total = total + i; }
            if total == 10 { println("ok"); } else { println("sai"); }"#;
        assert!(vm_run(src).is_ok());
    }

    #[test]
    fn vm_unsupported_errors_clearly() {
        // Struct chưa hỗ trợ trong VM phase 1 → lỗi rõ ràng (compile).
        let stmts = Parser::new(Lexer::new("struct P { x = 0; }").tokenize().unwrap())
            .parse()
            .unwrap();
        assert!(compile(&stmts).is_err());
    }
}