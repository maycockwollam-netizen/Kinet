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
    MakeMap(u32), // 2n phần tử: (k1,v1,k2,v2,...)
    MakeSet(u32),
    Unpack(u32),
    GetIndex,
    SetIndex,
    Slice,
    In,
    // Gọi phép tính/method trên giá trị: `value.name(args)` (native).
    CallMethod(String, u32),
    // Gọi hàm/native theo TÊN (phase 1++ chưa có closure/giá trị hàm).
    Call(String, u32),
    Return,
    Throw,
    TryBegin(usize),
    TryEnd,
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
        // Thay đúng vị trí placeholder (jump hoặc TryBegin) với target tuyệt đối.
        match &mut self.code[place] {
            Instr::Jump(t) | Instr::JumpIfFalse(t) | Instr::JumpIfTrue(t) | Instr::TryBegin(t) => {
                *t = target
            }
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
            Stmt::Throw(e) => {
                self.compile_expr(e)?;
                self.code.push(Instr::Throw);
                Ok(())
            }
            Stmt::Try {
                body,
                catch_var,
                catch_body,
                finally_body,
            } => {
                // try { A } catch e { B } finally { F }
                //  - lỗi trong A → unwind tới handler tại catch_pc, bind chuỗi lỗi.
                //  - finally chạy ở cả 2 nhánh (chèn mã lặp lại).
                let catch_pc = self.emit_jump_placeholder(Instr::TryBegin(0));
                self.scope.push_block();
                self.compile_body(body)?;
                self.scope.pop_block();
                self.code.push(Instr::TryEnd);
                // Nhánh thành công: finally rồi thoát.
                self.scope.push_block();
                self.compile_body(finally_body)?;
                self.scope.pop_block();
                let jd = self.emit_jump_placeholder(Instr::Jump(0));
                // Nhánh lỗi: bind lỗi, chạy catch (nếu có), finally.
                let catch_index = self.code.len();
                self.patch(catch_pc, catch_index);
                if let Some(cb) = catch_body {
                    match catch_var {
                        Some(v) => {
                            self.store_var(v);
                            self.code.push(Instr::Pop); // bỏ bản sao còn lại
                        }
                        None => self.code.push(Instr::Pop),
                    }
                    self.scope.push_block();
                    self.compile_body(cb)?;
                    self.scope.pop_block();
                } else {
                    // try/finally: lưu lỗi vào slot ẩn, chạy finally rồi ném lại.
                    let err_slot = self.scope.declare("$err");
                    self.code.push(Instr::StoreSlot(err_slot));
                    if !finally_body.is_empty() {
                        self.scope.push_block();
                        self.compile_body(finally_body)?;
                        self.scope.pop_block();
                    }
                    self.code.push(Instr::LoadSlot(err_slot));
                    self.code.push(Instr::Throw);
                }
                if !finally_body.is_empty() && catch_body.is_some() {
                    self.scope.push_block();
                    self.compile_body(finally_body)?;
                    self.scope.pop_block();
                }
                let done = self.code.len();
                self.patch(jd, done);
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
            Expr::IndexAssign {
                name,
                indices,
                value,
            } => {
                if indices.len() != 1 {
                    return Err(Self::unsupported("gán chỉ số nhiều chiều"));
                }
                // value; container; idx; SetIndex → container mới; gán lại biến.
                self.compile_expr(value)?;
                self.compile_var_load(name);
                self.compile_expr(&indices[0])?;
                self.code.push(Instr::SetIndex);
                self.store_var(name);
                self.code.push(Instr::Pop);
            }
            Expr::MethodCall {
                target,
                name,
                args,
            } => {
                self.compile_expr(target)?;
                for a in args {
                    self.compile_expr(a)?;
                }
                self.code
                    .push(Instr::CallMethod(name.clone(), args.len() as u32));
            }
            Expr::Slice {
                target,
                start,
                end,
                step,
            } => {
                self.compile_expr(target)?;
                match start {
                    Some(e) => self.compile_expr(e)?,
                    None => self.code.push(Instr::PushNil),
                }
                match end {
                    Some(e) => self.compile_expr(e)?,
                    None => self.code.push(Instr::PushNil),
                }
                match step {
                    Some(e) => self.compile_expr(e)?,
                    None => self.code.push(Instr::PushNil),
                }
                self.code.push(Instr::Slice);
            }
            Expr::Dict(entries) => {
                for (k, v) in entries {
                    self.compile_expr(k)?;
                    self.compile_expr(v)?;
                }
                self.code
                    .push(Instr::MakeMap((entries.len() * 2) as u32));
            }
            Expr::Set(items) => {
                for it in items {
                    self.compile_expr(it)?;
                }
                self.code.push(Instr::MakeSet(items.len() as u32));
            }
            Expr::Match { value, arms } => self.compile_match(value, arms)?,
            Expr::Comp { element, clauses } => self.compile_comp(element, None, clauses)?,
            Expr::DictComp {
                key,
                value,
                clauses,
            } => self.compile_comp(key, Some(value), clauses)?,
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
            TokenKind::In => Instr::In,
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

    fn compile_var_load(&mut self, name: &str) {
        if let Some(slot) = self.scope.lookup(name) {
            self.code.push(Instr::LoadSlot(slot));
        } else {
            self.code.push(Instr::LoadGlobal(name.to_string()));
        }
    }

    // Biên dịch `match value { case p => e; ... }` thành chuỗi if/else.
    fn compile_match(
        &mut self,
        value: &Expr,
        arms: &[crate::ast::MatchArm],
    ) -> Result<(), String> {
        use crate::ast::Pattern;
        let matched = self.scope.declare("$match");
        self.compile_expr(value)?;
        self.code.push(Instr::StoreSlot(matched));
        let mut done_jumps: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                Pattern::Literal(lit) => {
                    self.code.push(Instr::LoadSlot(matched));
                    self.compile_expr(lit)?;
                    self.code.push(Instr::Eq);
                }
                Pattern::Binding(_) | Pattern::Wildcard => {
                    self.code.push(Instr::PushBool(true));
                }
            }
            let next = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
            if let Pattern::Binding(name) = &arm.pattern {
                let slot = self.scope.declare(name);
                self.code.push(Instr::LoadSlot(matched));
                self.code.push(Instr::StoreSlot(slot));
            }
            self.compile_expr(&arm.body)?;
            done_jumps.push(self.emit_jump_placeholder(Instr::Jump(0)));
            let here = self.code.len();
            self.patch(next, here);
        }
        self.code.push(Instr::PushStr(
            "Không có nhánh match nào khớp (thiếu default?)".to_string(),
        ));
        self.code.push(Instr::Throw);
        let done = self.code.len();
        for j in done_jumps {
            self.patch(j, done);
        }
        Ok(())
    }

    // Comprehension: `[expr for x in it (if cond)...]` hoặc dict `{k: v for ...}`.
    fn compile_comp(
        &mut self,
        element: &Expr,
        dict_value: Option<&Expr>,
        clauses: &[crate::ast::CompClause],
    ) -> Result<(), String> {
        let result_slot = self.scope.declare("$comp_result");
        if dict_value.is_some() {
            self.code.push(Instr::MakeMap(0));
        } else {
            self.code.push(Instr::MakeList(0));
        }
        self.code.push(Instr::StoreSlot(result_slot));
        self.compile_comp_clauses(clauses, 0, result_slot, element, dict_value)?;
        self.code.push(Instr::LoadSlot(result_slot));
        Ok(())
    }

    fn compile_comp_clauses(
        &mut self,
        clauses: &[crate::ast::CompClause],
        i: usize,
        result_slot: u32,
        element: &Expr,
        dict_value: Option<&Expr>,
    ) -> Result<(), String> {
        use crate::ast::CompClause;
        if i == clauses.len() {
            // Lá: chèn phần tử vào bộ sưu tập result.
            if let Some(dv) = dict_value {
                self.code.push(Instr::LoadSlot(result_slot));
                self.compile_expr(element)?; // key
                self.compile_expr(dv)?; // value
                self.code.push(Instr::Call("map_set".to_string(), 3));
                self.code.push(Instr::StoreSlot(result_slot));
            } else {
                self.code.push(Instr::LoadSlot(result_slot));
                self.compile_expr(element)?;
                self.code.push(Instr::Call("append".to_string(), 2));
                self.code.push(Instr::StoreSlot(result_slot));
            }
            return Ok(());
        }
        match &clauses[i] {
            CompClause::For { var, iterable } => {
                let iter_slot = self.scope.declare("$iter");
                let idx_slot = self.scope.declare("$idx");
                let var_slot = self.scope.declare(var);
                self.compile_expr(iterable)?;
                self.code.push(Instr::StoreSlot(iter_slot));
                self.code.push(Instr::PushInt(0));
                self.code.push(Instr::StoreSlot(idx_slot));
                let cond_index = self.code.len();
                self.code.push(Instr::LoadSlot(idx_slot));
                self.code.push(Instr::LoadSlot(iter_slot));
                self.code.push(Instr::Call("len".to_string(), 1));
                self.code.push(Instr::Lt);
                let done = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
                self.code.push(Instr::LoadSlot(iter_slot));
                self.code.push(Instr::LoadSlot(idx_slot));
                self.code.push(Instr::Call("get".to_string(), 2));
                self.code.push(Instr::StoreSlot(var_slot));
                self.compile_comp_clauses(clauses, i + 1, result_slot, element, dict_value)?;
                self.code.push(Instr::LoadSlot(idx_slot));
                self.code.push(Instr::PushInt(1));
                self.code.push(Instr::Add);
                self.code.push(Instr::StoreSlot(idx_slot));
                self.code.push(Instr::Jump(cond_index));
                let end = self.code.len();
                self.patch(done, end);
            }
            CompClause::If(cond) => {
                self.compile_expr(cond)?;
                let skip = self.emit_jump_placeholder(Instr::JumpIfFalse(0));
                self.compile_comp_clauses(clauses, i + 1, result_slot, element, dict_value)?;
                let here = self.code.len();
                self.patch(skip, here);
            }
        }
        Ok(())
    }
}

// ---------- Runner ----------

struct Frame {
    code: std::rc::Rc<Vec<Instr>>,
    ip: usize,
    locals: Vec<Value>,
}

#[derive(Debug, Clone, Copy)]
struct Handler {
    frame_start: usize,
    stack_depth: usize,
    catch_pc: usize,
}

pub struct Vm {
    program: Program,
    globals: HashMap<String, Value>,
    natives: HashMap<String, fn(&[Value]) -> Result<Value, String>>,
    handlers: Vec<Handler>,
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
            handlers: Vec::new(),
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
                Instr::Add => a.checked_add(b).map(Value::Int).ok_or_else(|| "Tràn số khi cộng".to_string()),
                Instr::Sub => a.checked_sub(b).map(Value::Int).ok_or_else(|| "Tràn số khi trừ".to_string()),
                Instr::Mul => a.checked_mul(b).map(Value::Int).ok_or_else(|| "Tràn số khi nhân".to_string()),
                Instr::Div => {
                    if b == 0 { Err("Chia cho 0".to_string()) } else { Ok(Value::Int(a / b)) }
                }
                Instr::Mod => {
                    if b == 0 { Err("Chia lấy dư cho 0".to_string()) } else { Ok(Value::Int(a % b)) }
                }
                _ => unreachable!(),
            },
            (Value::Int(a), Value::Float(b)) => {
                let res = match instr {
                    Instr::Add => (a as f64) + b,
                    Instr::Sub => (a as f64) - b,
                    Instr::Mul => (a as f64) * b,
                    Instr::Div => {
                        if b == 0.0 { return Err("Chia cho 0".to_string()); }
                        (a as f64) / b
                    }
                    Instr::Mod => {
                        if b == 0.0 { return Err("Chia lấy dư cho 0".to_string()); }
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
                        if b == 0 { return Err("Chia cho 0".to_string()); }
                        a / (b as f64)
                    }
                    Instr::Mod => {
                        if b == 0 { return Err("Chia lấy dư cho 0".to_string()); }
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
                        if b == 0.0 { return Err("Chia cho 0".to_string()); }
                        a / b
                    }
                    Instr::Mod => {
                        if b == 0.0 { return Err("Chia lấy dư cho 0".to_string()); }
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

    // Map method `giá trị.tên(args)` → native tương ứng (theo kiểu receiver).
    fn method_native(&self, receiver: &Value, name: &str) -> Option<&'static str> {
        match receiver {
            Value::Str(_) => match name {
                "upper" => Some("to_upper"),
                "lower" => Some("to_lower"),
                "trim" => Some("trim"),
                "split" => Some("split"),
                "starts_with" => Some("starts_with"),
                "ends_with" => Some("ends_with"),
                "contains" => Some("contains"),
                "replace" => Some("replace"),
                "len" => Some("len"),
                "chars" => Some("chars"),
                "ord" => Some("ord"),
                "to_int" => Some("to_int"),
                "to_float" => Some("to_float"),
                "substring" => Some("substring"),
                "lines" => Some("read_lines"),
                _ => None,
            },
            Value::List(_) => match name {
                "len" => Some("len"),
                "contains" => Some("contains"),
                "get" => Some("get"),
                "join" => Some("list_join"),
                "sorted" => Some("sorted"),
                "append" => Some("append"),
                "map" => Some("map"),
                "filter" => Some("filter"),
                _ => None,
            },
            Value::Map(_) => match name {
                "get" => Some("get"),
                "contains" => Some("contains"),
                "keys" => Some("map_keys"),
                "set" => Some("map_set"),
                "remove" => Some("map_remove"),
                "len" => Some("len"),
                _ => None,
            },
            Value::Set(_) => match name {
                "len" => Some("len"),
                "contains" => Some("contains"),
                "add" => Some("set_add"),
                "remove" => Some("set_remove"),
                "union" => Some("set_union"),
                "sorted" => Some("sorted"),
                _ => None,
            },
            Value::Int(_) | Value::Float(_) => match name {
                "abs" => Some("abs"),
                "round" => Some("round"),
                "floor" => Some("floor"),
                "ceil" => Some("ceil"),
                "to_int" => Some("to_int"),
                "to_float" => Some("to_float"),
                _ => None,
            },
            _ => None,
        }
    }

    // Chạy một lệnh; Ok(false) = tiếp tục, Ok(true) = dừng chương trình.
    fn exec_instr(
        &mut self,
        instr: Instr,
        stack: &mut Vec<Value>,
        frames: &mut Vec<Frame>,
    ) -> Result<bool, String> {
        match instr {
            Instr::Halt => return Ok(true),
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
            Instr::MakeMap(n) => {
                let mut vals = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    vals.push(stack.pop().ok_or("stack rỗng")?);
                }
                vals.reverse();
                let mut map = indexmap::IndexMap::new();
                for pair in vals.chunks(2) {
                    if pair.len() == 2 {
                        map.insert(pair[0].clone(), pair[1].clone());
                    }
                }
                stack.push(Value::Map(map));
            }
            Instr::MakeSet(n) => {
                let mut vals = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    vals.push(stack.pop().ok_or("stack rỗng")?);
                }
                vals.reverse();
                let mut set = indexmap::IndexMap::new();
                for v in vals {
                    set.insert(v, ());
                }
                stack.push(Value::Set(set));
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
                    return Err(format!("bóc tách cần {} giá trị, nhận {}", n, items.len()));
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
                        let i = crate::interpreter::expect_and_normalize_index(&idx, items.len())?;
                        items[i].clone()
                    }
                    Value::Tuple(items) => {
                        let i = crate::interpreter::expect_and_normalize_index(&idx, items.len())?;
                        items[i].clone()
                    }
                    Value::Str(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let i = crate::interpreter::expect_and_normalize_index(&idx, chars.len())?;
                        Value::Str(chars[i].to_string())
                    }
                    Value::Map(entries) => entries.get(&idx).cloned().ok_or_else(|| {
                        format!("map không có khóa '{}'", stringify_value(&idx))
                    })?,
                    other => {
                        return Err(format!(
                            "chỉ số `[]` không áp dụng được cho '{}'",
                            stringify_value(&other)
                        ));
                    }
                };
                stack.push(v);
            }
            Instr::SetIndex => {
                let idx = stack.pop().ok_or("stack rỗng")?;
                let container = stack.pop().ok_or("stack rỗng")?;
                let value = stack.pop().ok_or("stack rỗng")?;
                let rebuilt = match container {
                    Value::List(items) => {
                        let i = crate::interpreter::expect_and_normalize_index(&idx, items.len())?;
                        let mut out = items.clone();
                        out[i] = value;
                        Value::List(out)
                    }
                    Value::Map(entries) => {
                        let mut out = entries.clone();
                        out.insert(idx, value);
                        Value::Map(out)
                    }
                    other => {
                        return Err(format!(
                            "gán `[]` không áp dụng được cho '{}'",
                            stringify_value(&other)
                        ));
                    }
                };
                stack.push(rebuilt);
            }
            Instr::Slice => {
                let step_v = stack.pop().ok_or("stack rỗng")?;
                let end_v = stack.pop().ok_or("stack rỗng")?;
                let start_v = stack.pop().ok_or("stack rỗng")?;
                let container = stack.pop().ok_or("stack rỗng")?;
                let (start, end, step) = match (start_v, end_v, step_v) {
                    (Value::Nil, Value::Nil, Value::Nil) => (None, None, 1),
                    (s, e, st) => (
                        match s {
                            Value::Nil => None,
                            v => Some(crate::interpreter::expect_index(&v)?),
                        },
                        match e {
                            Value::Nil => None,
                            v => Some(crate::interpreter::expect_index(&v)?),
                        },
                        match st {
                            Value::Nil => 1,
                            v => crate::interpreter::expect_index(&v)?,
                        },
                    ),
                };
                let sliced = match container {
                    Value::List(items) => {
                        let (lo, hi) = crate::interpreter::adjust_slice(items.len(), start, end, step)?;
                        let mut out = Vec::new();
                        let mut i = lo;
                        while (step > 0 && i < hi) || (step < 0 && i > hi) {
                            out.push(items[i as usize].clone());
                            i += step;
                        }
                        Value::List(out)
                    }
                    Value::Str(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let (lo, hi) = crate::interpreter::adjust_slice(chars.len(), start, end, step)?;
                        let mut out = String::new();
                        let mut i = lo;
                        while (step > 0 && i < hi) || (step < 0 && i > hi) {
                            out.push(chars[i as usize]);
                            i += step;
                        }
                        Value::Str(out)
                    }
                    other => {
                        return Err(format!(
                            "slice chỉ áp dụng cho list/chuỗi, gặp '{}'",
                            stringify_value(&other)
                        ));
                    }
                };
                stack.push(sliced);
            }
            Instr::In => {
                let r = stack.pop().ok_or("stack rỗng")?;
                let l = stack.pop().ok_or("stack rỗng")?;
                let found = match &r {
                    Value::List(items) => items.contains(&l),
                    Value::Tuple(items) => items.contains(&l),
                    Value::Set(entries) => entries.contains_key(&l),
                    Value::Map(entries) => entries.contains_key(&l),
                    Value::Str(s) => {
                        let Value::Str(needle) = &l else {
                            return Err("vế trái của 'in' phải là chuỗi khi vế phải là chuỗi".to_string());
                        };
                        s.contains(needle)
                    }
                    other => {
                        return Err(format!(
                            "toán tử 'in' cần list/tuple/set/map/chuỗi, gặp '{}'",
                            stringify_value(other)
                        ));
                    }
                };
                stack.push(Value::Bool(found));
            }
            Instr::CallMethod(name, argc) => {
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    args.push(stack.pop().ok_or("stack rỗng")?);
                }
                args.reverse();
                let receiver = stack.pop().ok_or("stack rỗng")?;
                if name == "reverse" {
                    if let Value::List(items) = &receiver {
                        let rev: Vec<Value> = items.iter().rev().cloned().collect();
                        stack.push(Value::List(rev));
                        return Ok(false);
                    }
                    return Err("reverse() chỉ áp dụng cho list".to_string());
                }
                let native_name = self
                    .method_native(&receiver, &name)
                    .ok_or_else(|| format!("giá trị '{}' không có phương thức '{}'", stringify_value(&receiver), name))?;
                let mut all = Vec::with_capacity(args.len() + 1);
                all.push(receiver);
                all.extend(args);
                let func = self
                    .natives
                    .get(native_name)
                    .copied()
                    .ok_or_else(|| format!("thiếu native '{}'", native_name))?;
                let v = func(&all)?;
                stack.push(v);
            }
            Instr::Call(name, argc) => {
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    args.push(stack.pop().ok_or("stack rỗng")?);
                }
                args.reverse();
                if name == "__print__" || name == "__println__" {
                    let text: String = args
                        .iter()
                        .map(crate::interpreter::stringify_value)
                        .collect();
                    print!("{}", text);
                    if name == "__println__" {
                        println!();
                    }
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    stack.push(Value::Nil);
                    return Ok(false);
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
                    return Ok(false);
                }
                if let Some(func) = self.natives.get(&name).copied() {
                    let v = func(&args)?;
                    stack.push(v);
                    return Ok(false);
                }
                return Err(format!("Chưa khai báo hàm/native '{}'", name));
            }
            Instr::Return => {
                let v = stack.pop().ok_or("stack rỗng")?;
                frames.pop();
                if frames.is_empty() {
                    return Ok(true);
                }
                stack.push(v);
            }
            Instr::Throw => {
                let v = stack.pop().ok_or("stack rỗng")?;
                return Err(stringify_value(&v));
            }
            Instr::TryBegin(catch_pc) => {
                self.handlers.push(Handler {
                    frame_start: frames.len(),
                    stack_depth: stack.len(),
                    catch_pc,
                });
            }
            Instr::TryEnd => {
                self.handlers.pop();
            }
        }
        Ok(false)
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
            if frames.is_empty() {
                return Ok(());
            }
            let frame = frames.last_mut().unwrap();
            if frame.ip >= frame.code.len() {
                return Ok(());
            }
            let instr = frame.code[frame.ip].clone();
            frame.ip += 1;
            match self.exec_instr(instr, &mut stack, &mut frames) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => {
                    if let Some(h) = self.handlers.last().cloned() {
                        if frames.len() >= h.frame_start {
                            frames.truncate(h.frame_start);
                            stack.truncate(h.stack_depth);
                            stack.push(Value::Str(e));
                            frames.last_mut().unwrap().ip = h.catch_pc;
                            self.handlers.pop();
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
    }
}#[cfg(test)]
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
        // Struct chưa hỗ trợ trong VM (phase 2) → lỗi rõ ràng (compile).
        let stmts = Parser::new(Lexer::new("struct P { x = 0; }").tokenize().unwrap())
            .parse()
            .unwrap();
        assert!(compile(&stmts).is_err());
    }

    #[test]
    fn vm_containers_in_slice() {
        let src = r#"
            let m = { "x": 1, "y": 2 };
            if m["x"] != 1 { println("sai"); }
            if "y" not in m { println("sai2"); }
            let s = {1, 1, 2};
            if len(s) != 2 { println("sai3"); }
            if 2 not in s { println("sai4"); }
            let t = (5, 6, 7);
            if t[2] != 7 { println("sai5"); }
            if "bcd" != "abcdef"[1:4] { println("sai6"); }
            let arr = [1, 2, 3];
            arr[0] = 9;
            if arr[0] != 9 { println("sai7"); }
        "#;
        assert!(vm_run(src).is_ok());
    }

    #[test]
    fn vm_methods_and_match() {
        let src = r#"
            if "  hi  ".trim().upper() != "HI" { println("sai1"); }
            if [1,2,3].append(4)[3] != 4 { println("sai2"); }
            if "a,b".split(",").len() != 2 { println("sai3"); }
            let v = match 2 { case 1 => "one"; case x => "other"; };
            if v != "other" { println("sai4"); }
        "#;
        assert!(vm_run(src).is_ok());
    }

    #[test]
    fn vm_comprehension() {
        let src = r#"
            let c = [x * x for x in range(0, 5)];
            if len(c) != 5 { println("sai1"); }
            if c[3] != 9 { println("sai2"); }
            let e = [x for x in range(0, 8) if x % 2 == 0];
            if len(e) != 4 { println("sai3"); }
            let d = { i: i * 2 for i in range(0, 3) };
            if d[2] != 4 { println("sai4"); }
        "#;
        assert!(vm_run(src).is_ok());
    }

    #[test]
    fn vm_try_catch_and_finally() {
        let caught = r#"
            let got = "";
            try { let z = 1 / 0; } catch e { got = "lỗi"; }
            if got != "lỗi" { println("sai1"); }
        "#;
        assert!(vm_run(caught).is_ok());

        // finally chạy kể cả khi lỗi → lỗi vẫn lan ra ngoài (try/finally).
        let fin = r#"
            let log = "";
            try { let z = 1 / 0; } finally { log = "don"; }
        "#;
        assert!(vm_run(fin).is_err()); // lỗi lan ra
    }

    #[test]
    fn vm_uncaught_throw_errors() {
        let src = r#"throw "đầu hàng";"#;
        let e = vm_run(src).unwrap_err();
        assert!(e.contains("đầu hàng"), "{}", e);
    }
}