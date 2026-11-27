//! Parser: nhóm dãy token thành AST bằng phương pháp recursive descent.
//!
//! Độ ưu tiên toán tử (thấp → cao):
//!   or
//!   and
//!   == !=
//!   < > <= >=  (cùng bậc: in / not in)
//!   + -
//!   * / %
//!   unary (- not)
//!   primary / call

use crate::ast::{CallArg, CompClause, Expr, LetTarget, MatchArm, MethodDef, ParamDef, Pattern, Stmt};
use crate::lexer::Lexer;
use crate::token::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse(mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        while !self.at_end() {
            stmts.push(self.statement()?);
        }
        Ok(stmts)
    }

    // ---------- Statements ----------

    fn statement(&mut self) -> Result<Stmt, String> {
        let line = self.peek().line;
        let inner = self.statement_inner()?;
        Ok(Stmt::Ann {
            line,
            inner: Box::new(inner),
        })
    }

    fn statement_inner(&mut self) -> Result<Stmt, String> {
        let kind = self.peek().kind.clone();
        match kind {
            TokenKind::Let => self.let_stmt(),
            TokenKind::Func => self.func_stmt(),
            TokenKind::Struct => self.struct_stmt(),
            TokenKind::If => self.if_stmt(),
            TokenKind::While => self.while_stmt(),
            TokenKind::For => self.for_stmt(),
            TokenKind::Return => self.return_stmt(),
            TokenKind::Throw => self.throw_stmt(),
            TokenKind::Import => self.import_stmt(),
            TokenKind::Try => self.try_stmt(),
            TokenKind::Break => {
                self.advance();
                self.expect_semi("break")?;
                Ok(Stmt::Break)
            }
            TokenKind::Continue => {
                self.advance();
                self.expect_semi("continue")?;
                Ok(Stmt::Continue)
            }
            TokenKind::Print => {
                self.advance();
                let exprs = self.print_args("print")?;
                self.expect_semi("print")?;
                Ok(Stmt::Print(exprs))
            }
            TokenKind::Println => {
                self.advance();
                let exprs = self.print_args("println")?;
                self.expect_semi("println")?;
                Ok(Stmt::Println(exprs))
            }
            _ => {
                let expr = self.expression()?;
                self.expect_semi("biểu thức")?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn let_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // let
        let target = self.let_target()?;
        self.consume(TokenKind::Assign, "mong đợi '=' sau tên biến")?;
        let value = self.expression()?;
        self.expect_semi("let")?;
        Ok(Stmt::Let { target, value })
    }

    fn func_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // func
        let name = self.expect_ident("tên hàm")?;
        let params = self.param_list()?;
        let body = self.block()?;
        Ok(Stmt::Func { name, params, body })
    }

    // struct Tên { field = mặc_định; func method(params) { ... } }
    fn struct_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // struct
        let name = self.expect_ident("tên struct")?;
        self.consume(TokenKind::LBrace, "mong đợi '{' sau tên struct")?;
        let mut fields: Vec<(String, Expr)> = Vec::new();
        let mut methods: Vec<MethodDef> = Vec::new();
        while !self.check(&TokenKind::RBrace) {
            if self.check(&TokenKind::Func) {
                self.advance(); // func
                let mname = self.expect_ident("tên phương thức")?;
                if methods.iter().any(|m| m.name == mname)
                    || fields.iter().any(|(f, _)| f == &mname)
                {
                    return Err(self.error(format!("tên '{}' trùng trong struct", mname)));
                }
                let params = self.param_list()?;
                let body = self.block()?;
                methods.push(MethodDef {
                    name: mname,
                    params,
                    body,
                });
            } else {
                // Field: tên [= giá trị mặc định];
                let fname = self.expect_ident("tên field trong struct")?;
                if fields.iter().any(|(f, _)| f == &fname)
                    || methods.iter().any(|m| m.name == fname)
                {
                    return Err(self.error(format!("tên '{}' trùng trong struct", fname)));
                }
                let default = if self.match_or(TokenKind::Assign) {
                    self.expression()?
                } else {
                    Expr::Nil
                };
                self.expect_semi("field")?;
                fields.push((fname, default));
            }
        }
        self.consume(TokenKind::RBrace, "mong đợi '}' để đóng struct")?;
        Ok(Stmt::Struct {
            name,
            fields,
            methods,
        })
    }

    fn import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // import
        let token = self.advance();
        let path = match token.kind {
            TokenKind::Str(s) => s,
            other => {
                return Err(self.error_at(
                    token.line,
                    format!("mong đợi chuỗi đường dẫn sau import, gặp {:?}", other),
                ))
            }
        };
        self.expect_semi("import")?;
        Ok(Stmt::Import(path))
    }

    fn throw_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // throw
        let expr = self.expression()?;
        self.expect_semi("throw")?;
        Ok(Stmt::Throw(expr))
    }

    fn try_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // try
        let body = self.block()?;
        // catch [tên biến] { ... } — tùy chọn nếu có finally phía sau.
        let mut catch_var = None;
        let mut catch_body = None;
        if self.match_or(TokenKind::Catch) {
            catch_var = if matches!(self.peek().kind, TokenKind::Ident(_)) {
                let token = self.advance();
                if let TokenKind::Ident(name) = token.kind {
                    Some(name)
                } else {
                    unreachable!()
                }
            } else {
                None
            };
            catch_body = Some(self.block()?);
        }
        // finally { ... } — luôn chạy dù try/catch lỗi hay thành công.
        let mut finally_body = Vec::new();
        if self.match_or(TokenKind::Finally) {
            finally_body = self.block()?;
        }
        if catch_body.is_none() && finally_body.is_empty() {
            return Err(self.error("mong đợi 'catch' hoặc 'finally' sau khối try".to_string()));
        }
        Ok(Stmt::Try {
            body,
            catch_var,
            catch_body,
            finally_body,
        })
    }

    fn if_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // if
        let cond = self.expression()?;
        let then_branch = self.block()?;
        let mut else_branch = Vec::new();
        if self.match_or(TokenKind::Else) {
            if self.check(&TokenKind::If) {
                // else if ... -> gộp lại
                let nested = self.if_stmt()?;
                else_branch.push(nested);
            } else {
                else_branch = self.block()?;
            }
        }
        Ok(Stmt::If {
            cond,
            then_branch,
            else_branch,
        })
    }

    fn while_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // while
        let cond = self.expression()?;
        let body = self.block()?;
        Ok(Stmt::While { cond, body })
    }

    fn for_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // for
        let target = self.let_target()?;
        self.consume(TokenKind::In, "mong đợi 'in' sau biến lặp")?;
        let iterable = self.expression()?;
        let body = self.block()?;
        Ok(Stmt::For {
            target,
            iterable,
            body,
        })
    }

    fn return_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // return
        // `return;` không giá trị = trả về Nil; `return x, y;` = trả tuple.
        let expr = if self.check(&TokenKind::Semi) {
            Expr::Nil
        } else {
            let first = self.expression()?;
            if self.match_or(TokenKind::Comma) {
                let mut items = vec![first];
                loop {
                    if self.check(&TokenKind::Semi) {
                        break;
                    }
                    items.push(self.expression()?);
                    if !self.match_or(TokenKind::Comma) {
                        break;
                    }
                }
                Expr::Tuple(items)
            } else if let Expr::Tuple(_) = first {
                first
            } else {
                first
            }
        };
        self.expect_semi("return")?;
        Ok(Stmt::Return(expr))
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.consume(TokenKind::LBrace, "mong đợi '{'")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            stmts.push(self.statement()?);
        }
        self.consume(TokenKind::RBrace, "mong đợi '}' để đóng block")?;
        Ok(stmts)
    }

    // ---------- Expressions ----------

    fn expression(&mut self) -> Result<Expr, String> {
        self.assignment()
    }

    fn assignment(&mut self) -> Result<Expr, String> {
        let left = self.or()?;
        if self.match_or(TokenKind::Assign) {
            let value = self.assignment()?;
            // a = v
            if let Expr::Variable(name) = left {
                return Ok(Expr::Assign {
                    name,
                    value: Box::new(value),
                });
            }
            // arr[i] = v / grid[r][c] = v / m["k"] = v
            if let Some((name, indices)) = flatten_index(&left) {
                return Ok(Expr::IndexAssign {
                    name: name.to_string(),
                    indices,
                    value: Box::new(value),
                });
            }
            // p.x = v / a.b.c = v (field struct)
            if let Some((name, path)) = flatten_member(&left) {
                if !path.is_empty() {
                    return Ok(Expr::MemberAssign {
                        name: name.to_string(),
                        path,
                        value: Box::new(value),
                    });
                }
            }
            return Err(self.error(
                "vế trái của '=' phải là biến, mục lấy theo chỉ số (a[i]) hoặc field (p.x)".to_string(),
            ));
        }
        Ok(left)
    }

    fn or(&mut self) -> Result<Expr, String> {
        self.left_assoc_binary(&[TokenKind::Or], Self::and)
    }

    fn and(&mut self) -> Result<Expr, String> {
        self.left_assoc_binary(&[TokenKind::And], Self::equality)
    }

    fn equality(&mut self) -> Result<Expr, String> {
        self.left_assoc_binary(&[TokenKind::Eq, TokenKind::NotEq], Self::comparison)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.bitwise()?;
        loop {
            // Quy tắc chung: < > <= >= in, và "not in" (được desugar thành
            // Unary(Not, Binary(In))). Cùng bậc, kết hợp trái như Python.
            let op = if self.check(&TokenKind::Lt) {
                Some(TokenKind::Lt)
            } else if self.check(&TokenKind::Gt) {
                Some(TokenKind::Gt)
            } else if self.check(&TokenKind::Le) {
                Some(TokenKind::Le)
            } else if self.check(&TokenKind::Ge) {
                Some(TokenKind::Ge)
            } else if self.check(&TokenKind::In) {
                Some(TokenKind::In)
            } else if self.check(&TokenKind::Not) && self.peek_kind_at(1) == Some(TokenKind::In) {
                Some(TokenKind::NotIn)
            } else {
                None
            };
            let Some(op) = op else { break };
            self.advance(); // toán tử — riêng NotIn ăn thêm 'in'
            if op == TokenKind::NotIn {
                self.advance();
            }
            let right = self.bitwise()?;
            left = if op == TokenKind::NotIn {
                Expr::Unary {
                    op: TokenKind::Not,
                    expr: Box::new(Expr::Binary {
                        left: Box::new(left),
                        op: TokenKind::In,
                        right: Box::new(right),
                    }),
                }
            } else {
                Expr::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                }
            };
        }
        Ok(left)
    }

    // << >> & | (cùng bậc, kết hợp trái).
    fn bitwise(&mut self) -> Result<Expr, String> {
        self.left_assoc_binary(
            &[TokenKind::Shl, TokenKind::Shr, TokenKind::Band, TokenKind::Bor],
            Self::term,
        )
    }

    fn term(&mut self) -> Result<Expr, String> {
        self.left_assoc_binary(&[TokenKind::Plus, TokenKind::Minus], Self::factor)
    }

    fn factor(&mut self) -> Result<Expr, String> {
        self.left_assoc_binary(&[TokenKind::Star, TokenKind::Slash, TokenKind::Percent], Self::unary)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        let kind = self.peek().kind.clone();
        if kind == TokenKind::Minus || kind == TokenKind::Not {
            self.advance();
            let expr = self.unary()?;
            return Ok(Expr::Unary {
                op: kind,
                expr: Box::new(expr),
            });
        }
        self.call()
    }

    fn call(&mut self) -> Result<Expr, String> {
        let mut callee = self.primary()?;
        loop {
            if self.match_or(TokenKind::LParen) {
                let mut args = Vec::new();
                let mut saw_named = false;
                if !self.check(&TokenKind::RParen) {
                    loop {
                        let arg = self.call_arg()?;
                        if arg.name.is_some() {
                            saw_named = true;
                        } else if saw_named {
                            return Err(self.error(
                                "đối số vị trí không được đứng sau đối số keyword".to_string(),
                            ));
                        }
                        args.push(arg);
                        if !self.match_or(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.consume(TokenKind::RParen, "mong đợi ')' sau đối số")?;
                callee = Expr::Call {
                    callee: Box::new(callee),
                    args,
                };
            } else if self.match_or(TokenKind::Dot) {
                // Field hoặc phương thức: expr.name / expr.name(args)
                // — target là biểu thức bất kỳ, chuỗi được (x.f().g()).
                let name = self.expect_ident("tên field/phương thức sau '.'")?;
                if self.match_or(TokenKind::LParen) {
                    let mut margs = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            margs.push(self.expression()?);
                            if !self.match_or(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.consume(
                        TokenKind::RParen,
                        "mong đợi ')' sau đối số phương thức",
                    )?;
                    callee = Expr::MethodCall {
                        target: Box::new(callee),
                        name,
                        args: margs,
                    };
                } else {
                    callee = Expr::Member {
                        target: Box::new(callee),
                        name,
                    };
                }
            } else if self.match_or(TokenKind::LBracket) {
                // arr[i] / m["k"] / grid[r][c] — hoặc slice arr[i:j] / arr[:j] / arr[::k].
                if self.check(&TokenKind::Colon) {
                    // Bắt đầu bằng ':' → thiếu start: a[:end] / a[::step]
                    self.advance(); // ':'
                    let (end, step) = self.slice_end_step()?;
                    callee = Expr::Slice {
                        target: Box::new(callee),
                        start: None,
                        end,
                        step,
                    };
                } else {
                    let first = self.expression()?;
                    if self.match_or(TokenKind::Colon) {
                        let (end, step) = self.slice_end_step()?;
                        callee = Expr::Slice {
                            target: Box::new(callee),
                            start: Some(Box::new(first)),
                            end,
                            step,
                        };
                    } else {
                        self.consume(TokenKind::RBracket, "mong đợi ']' sau chỉ số")?;
                        callee = Expr::Index {
                            target: Box::new(callee),
                            index: Box::new(first),
                        };
                    }
                }
            } else {
                break;
            }
        }
        Ok(callee)
    }

    // Sau dấu ':' đầu tiên của slice: đọc `end` và `step` (cả hai tùy chọn),
    // rồi nuốt luôn ']'. Hỗ trợ a[1:end] / a[1:end:step] / a[1::step] / a[:] / a[::].
    fn slice_end_step(&mut self) -> Result<(Option<Box<Expr>>, Option<Box<Expr>>), String> {
        let mut end = None;
        let mut step = None;
        if self.match_or(TokenKind::Colon) {
            // '::' — không có end, chỉ có step (tùy chọn)
            if !self.check(&TokenKind::RBracket) {
                step = Some(Box::new(self.expression()?));
            }
        } else if !self.check(&TokenKind::RBracket) {
            end = Some(Box::new(self.expression()?));
            if self.match_or(TokenKind::Colon) {
                if !self.check(&TokenKind::RBracket) {
                    step = Some(Box::new(self.expression()?));
                }
            }
        }
        self.consume(TokenKind::RBracket, "mong đợi ']' sau slice")?;
        Ok((end, step))
    }

    // Một đối số lúc gọi: `expr` hoặc keyword `tên = expr`. Dùng or() (không ăn
    // '=') để phân biệt gán trong đối số với keyword arg.
    fn call_arg(&mut self) -> Result<CallArg, String> {
        let first = self.or()?;
        if let Expr::Variable(name) = &first {
            if self.match_or(TokenKind::Assign) {
                let value = self.assignment()?;
                return Ok(CallArg {
                    name: Some(name.clone()),
                    value,
                });
            }
        }
        Ok(CallArg { name: None, value: first })
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let token = self.advance();
        match token.kind {
            TokenKind::Number(n) => Ok(Expr::Number(n)),
            TokenKind::Float(f) => Ok(Expr::Float(f)),
            TokenKind::Str(s) => {
                if s.contains("${") {
                    self.template_expr(&s)
                } else {
                    Ok(Expr::Str(restore_dollar(&s)))
                }
            }
            TokenKind::True => Ok(Expr::Bool(true)),
            TokenKind::False => Ok(Expr::Bool(false)),
            TokenKind::Ident(name) => Ok(Expr::Variable(name)),
            TokenKind::LParen => {
                let first = self.expression()?;
                if self.match_or(TokenKind::Comma) {
                    // Tuple literal: (a, b) / với dấu phẩy cuối (a, b,)
                    let mut items = vec![first];
                    loop {
                        if self.check(&TokenKind::RParen) {
                            break;
                        }
                        items.push(self.expression()?);
                        if !self.match_or(TokenKind::Comma) {
                            break;
                        }
                    }
                    self.consume(TokenKind::RParen, "mong đợi ')' sau tuple")?;
                    Ok(Expr::Tuple(items))
                } else {
                    self.consume(
                        TokenKind::RParen,
                        "mong đợi ')' sau biểu thức",
                    )?;
                    Ok(first)
                }
            }
            TokenKind::Func => {
                // primary() đã ăn token 'func' — hàm ẩn danh (lambda) trong biểu thức.
                self.lambda_expr()
            }
            TokenKind::If => {
                // primary() đã ăn token 'if' — if TRẢ GIÁ TRỊ trong biểu thức.
                self.if_expr_after_if()
            }
            TokenKind::Match => self.match_expr(),
            TokenKind::LBracket => {
                if self.match_or(TokenKind::RBracket) {
                    Ok(Expr::List(Vec::new())) // []
                } else {
                    let first = self.expression()?;
                    if self.match_or(TokenKind::For) {
                        // List comprehension: [expr for x in iter if cond ...]
                        let clauses = self.comp_clauses()?;
                        self.consume(
                            TokenKind::RBracket,
                            "mong đợi ']' sau comprehension",
                        )?;
                        Ok(Expr::Comp {
                            element: Box::new(first),
                            clauses,
                        })
                    } else {
                        let mut items = vec![first];
                        while self.match_or(TokenKind::Comma) {
                            if self.check(&TokenKind::RBracket) {
                                break; // dấu phẩy cuối — cho phép
                            }
                            items.push(self.expression()?);
                        }
                        self.consume(
                            TokenKind::RBracket,
                            "mong đợi ']' sau danh sách",
                        )?;
                        Ok(Expr::List(items))
                    }
                }
            }
            TokenKind::LBrace => {
                // Trong ngữ cảnh biểu thức, '{' luôn là dict literal (block
                // chỉ xuất hiện ở vị trí câu lệnh do block() phân tích).
                self.dict_literal()
            }
            other => Err(self.error_at(token.line, format!("mong đợi biểu thức, gặp {:?}", other))),
        }
    }

    fn dict_literal(&mut self) -> Result<Expr, String> {
        // primary() đã ăn '{' trước khi gọi hàm này.
        if self.match_or(TokenKind::RBrace) {
            return Ok(Expr::Dict(Vec::new())); // {}
        }
        let key = self.expression()?;
        self.consume(TokenKind::Colon, "mong đợi ':' giữa khóa và giá trị trong dict")?;
        let value = self.expression()?;
        if self.match_or(TokenKind::For) {
            // Dict comprehension: { key: value for x in iter ... }
            let clauses = self.comp_clauses()?;
            self.consume(TokenKind::RBrace, "mong đợi '}' sau dict comprehension")?;
            return Ok(Expr::DictComp {
                key: Box::new(key),
                value: Box::new(value),
                clauses,
            });
        }
        let mut entries = vec![(key, value)];
        loop {
            if !self.match_or(TokenKind::Comma) {
                break;
            }
            if self.check(&TokenKind::RBrace) {
                break; // dấu phẩy cuối — cho phép
            }
            let k = self.expression()?;
            self.consume(TokenKind::Colon, "mong đợi ':' giữa khóa và giá trị trong dict")?;
            let v = self.expression()?;
            entries.push((k, v));
        }
        self.consume(TokenKind::RBrace, "mong đợi '}' để đóng dict")?;
        Ok(Expr::Dict(entries))
    }

    // Parse các mệnh đề comprehension sau keyword `for` (đã ăn).
    // Ví dụ: `for x in a if cond for y in b` → [For(x,a), If(cond), For(y,b)].
    // Vế biểu thức của iterable/cond tự dừng trước `for`/`if`/`]`.
    fn comp_clauses(&mut self) -> Result<Vec<CompClause>, String> {
        let mut clauses = Vec::new();
        loop {
            let var = self.expect_ident("tên biến trong comprehension")?;
            self.consume(TokenKind::In, "mong đợi 'in' trong comprehension")?;
            let iterable = self.expression()?;
            clauses.push(CompClause::For { var, iterable });
            while self.match_or(TokenKind::If) {
                let cond = self.expression()?;
                clauses.push(CompClause::If(cond));
            }
            if !self.match_or(TokenKind::For) {
                break;
            }
        }
        Ok(clauses)
    }

    fn if_expr_after_if(&mut self) -> Result<Expr, String> {
        // 'if' đã được ăn. Cú pháp: if cond { giá trị } else { giá trị }
        let cond = self.expression()?;
        let then_expr = self.if_branch()?;
        let mut else_expr = Expr::Nil;
        if self.match_or(TokenKind::Else) {
            if self.check(&TokenKind::If) {
                // else if ...
                self.advance();
                else_expr = self.if_expr_after_if()?;
            } else {
                else_expr = self.if_branch()?;
            }
        }
        Ok(Expr::If {
            cond: Box::new(cond),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        })
    }

    // Một nhánh `{ biểu thức }` trả giá trị.
    fn if_branch(&mut self) -> Result<Expr, String> {
        self.consume(TokenKind::LBrace, "mong đợi '{' trong nhánh if trả giá trị")?;
        let e = self.expression()?;
        self.consume(TokenKind::RBrace, "mong đợi '}' sau nhánh if")?;
        Ok(e)
    }

    fn lambda_expr(&mut self) -> Result<Expr, String> {
        // primary() đã ăn token 'func'. Cú pháp: func (params) { body }
        let params = self.param_list()?;
        let body = self.block()?;
        Ok(Expr::Lambda { params, body })
    }

    fn template_expr(&mut self, contents: &str) -> Result<Expr, String> {
        let parts = split_template(contents)?;
        let mut exprs = Vec::with_capacity(parts.len());
        for (is_expr, text) in parts {
            if is_expr {
                // Phần ${...} là một biểu thức Kinet — lex + parse lại.
                let tokens = Lexer::new(&text).tokenize()?;
                let mut sub = Parser::new(tokens);
                let e = sub.expression()?;
                if !sub.at_end() {
                    return Err(format!(
                        "Template: sau khi parse '{}' còn dư token",
                        text
                    ));
                }
                exprs.push(e);
            } else {
                exprs.push(Expr::Str(text));
            }
        }
        Ok(Expr::Template(exprs))
    }

    fn match_expr(&mut self) -> Result<Expr, String> {
        // primary() đã ăn token 'match' trước khi gọi hàm này.
        let value = self.expression()?;
        self.consume(TokenKind::LBrace, "mong đợi '{' sau giá trị match")?;
        let mut arms = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_end() {
            arms.push(self.match_arm()?);
        }
        self.consume(TokenKind::RBrace, "mong đợi '}' để đóng match")?;
        Ok(Expr::Match {
            value: Box::new(value),
            arms,
        })
    }

    fn match_arm(&mut self) -> Result<MatchArm, String> {
        // default => expr ;
        if self.match_or(TokenKind::Default) {
            self.consume(TokenKind::Arrow, "mong đợi '=>' sau default")?;
            let body = self.expression()?;
            self.match_or(TokenKind::Semi);
            return Ok(MatchArm {
                pattern: Pattern::Wildcard,
                body,
            });
        }
        // case pattern => expr ;
        self.consume(TokenKind::Case, "mong đợi 'case' hoặc 'default' trong match")?;
        let pattern = self.pattern()?;
        self.consume(TokenKind::Arrow, "mong đợi '=>' sau pattern")?;
        let body = self.expression()?;
        self.match_or(TokenKind::Semi);
        Ok(MatchArm { pattern, body })
    }

    fn pattern(&mut self) -> Result<Pattern, String> {
        let token = self.advance();
        match token.kind {
            TokenKind::Ident(name) if name == "_" => Ok(Pattern::Wildcard),
            TokenKind::Ident(name) => Ok(Pattern::Binding(name)),
            TokenKind::Number(n) => Ok(Pattern::Literal(Expr::Number(n))),
            TokenKind::Float(f) => Ok(Pattern::Literal(Expr::Float(f))),
            TokenKind::Str(s) => Ok(Pattern::Literal(Expr::Str(s))),
            TokenKind::True => Ok(Pattern::Literal(Expr::Bool(true))),
            TokenKind::False => Ok(Pattern::Literal(Expr::Bool(false))),
            other => Err(self.error_at(token.line, format!("pattern không hợp lệ: {:?}", other))),
        }
    }

    // Helper: nhị phân kết hợp trái theo danh sách toán tử cùng bậc.
    fn left_assoc_binary(
        &mut self,
        ops: &[TokenKind],
        next: fn(&mut Self) -> Result<Expr, String>,
    ) -> Result<Expr, String> {
        let mut left = next(self)?;
        while let Some(op) = ops
            .iter()
            .find(|op| self.check(op))
            .cloned()
        {
            self.advance();
            let right = next(self)?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    // ---------- Token helpers ----------

    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn at_end(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn advance(&mut self) -> Token {
        let token = self.peek().clone();
        if !self.at_end() {
            self.pos += 1;
        }
        token
    }

    fn check(&self, kind: &TokenKind) -> bool {
        &self.peek().kind == kind
    }

    // Loại token cách vị trí hiện tại `offset` bước (0 = peek, 1 = kế tiếp).
    fn peek_kind_at(&self, offset: usize) -> Option<TokenKind> {
        let i = (self.pos + offset).min(self.tokens.len().saturating_sub(1));
        self.tokens.get(i).map(|t| t.kind.clone())
    }

    fn consume(&mut self, kind: TokenKind, msg: &str) -> Result<bool, String> {
        if self.check(&kind) {
            self.advance();
            Ok(true)
        } else {
            Err(self.error(msg.to_string()))
        }
    }

    // Ăn token nếu khớp, trả về false nếu không (không báo lỗi).
    fn match_or(&mut self, kind: TokenKind) -> bool {
        if self.check(&kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, String> {
        let token = self.advance();
        match token.kind {
            TokenKind::Ident(name) => Ok(name),
            other => Err(self.error_at(token.line, format!("mong đợi {}, gặp {:?}", what, other))),
        }
    }

    fn expect_semi(&mut self, after_what: &str) -> Result<(), String> {
        self.consume(
            TokenKind::Semi,
            &format!("mong đợi ';' sau {}", after_what),
        )
        .map(|_| ())
    }

    // Parse danh sách tham số `(a, b = 5, c)` — có thể rỗng; ăn luôn dấu ngoặc.
    fn param_list(&mut self) -> Result<Vec<ParamDef>, String> {
        self.consume(TokenKind::LParen, "mong đợi '(' sau tên hàm")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let name = self.expect_ident("tên tham số")?;
                if params.iter().any(|p: &ParamDef| p.name == name) {
                    return Err(self.error(format!("tham số trùng tên '{}'", name)));
                }
                let default = if self.match_or(TokenKind::Assign) {
                    Some(self.expression()?)
                } else {
                    None
                };
                params.push(ParamDef { name, default });
                if !self.match_or(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume(TokenKind::RParen, "mong đợi ')' sau danh sách tham số")?;
        Ok(params)
    }

    // Vế trái của `let`/`for`: biến đơn `x` hoặc pattern `(a, b)`.
    fn let_target(&mut self) -> Result<LetTarget, String> {
        if self.check(&TokenKind::LParen) {
            self.advance(); // (
            let mut names = Vec::new();
            if !self.check(&TokenKind::RParen) {
                loop {
                    let n = self.expect_ident("tên biến trong pattern")?;
                    if names.contains(&n) {
                        return Err(self.error(format!("biến '{}' lặp lại trong pattern", n)));
                    }
                    names.push(n);
                    if !self.match_or(TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.consume(TokenKind::RParen, "mong đợi ')' sau pattern")?;
            if names.is_empty() {
                return Err(self.error("pattern rỗng '()' không hợp lệ".to_string()));
            }
            Ok(LetTarget::Tuple(names))
        } else {
            let name = self.expect_ident("tên biến")?;
            Ok(LetTarget::Var(name))
        }
    }

    // Parse (expr, expr, ...) cho print/println, ghép nhiều giá trị lại.
    fn print_args(&mut self, what: &str) -> Result<Vec<Expr>, String> {
        self.consume(
            TokenKind::LParen,
            &format!("mong đợi '(' sau {}", what),
        )?;
        let mut exprs = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                exprs.push(self.expression()?);
                if !self.match_or(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume(
            TokenKind::RParen,
            &format!("mong đợi ')' sau danh sách tham số {}", what),
        )?;
        Ok(exprs)
    }

    fn error(&self, msg: String) -> String {
        format!("Lỗi cú pháp dòng {}: {}", self.peek().line, msg)
    }

    // Dùng khi đã advance() nuốt token lỗi rồi: self.peek() giờ trỏ token kế,
    // nên phải dùng đúng dòng của token vừa nuốt để không lệch số dòng.
    fn error_at(&self, line: usize, msg: String) -> String {
        format!("Lỗi cú pháp dòng {}: {}", line, msg)
    }
}

/// Tách một chuỗi template thành các phần: `false` = chuỗi thường,
/// `true` = nội dung biểu thức giữa `${...}`. Theo dõi `{}` lồng nhau và
/// chuỗi con `"..."` để tìm đúng dấu đóng `}`.
fn split_template(contents: &str) -> Result<Vec<(bool, String)>, String> {
    let chars: Vec<char> = contents.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut parts: Vec<(bool, String)> = Vec::new();
    let mut literal = String::new();

    while i < n {
        if chars[i] == '$' && i + 1 < n && chars[i + 1] == '{' {
            if !literal.is_empty() {
                parts.push((false, restore_dollar(&literal)));
                literal.clear();
            }
            i += 2;
            let mut depth = 1usize;
            let mut expr = String::new();
            let mut in_str = false;
            while i < n {
                let c = chars[i];
                if in_str {
                    expr.push(c);
                    if c == '\\' && i + 1 < n {
                        expr.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    if c == '"' {
                        in_str = false;
                    }
                    i += 1;
                    continue;
                }
                match c {
                    '"' => {
                        in_str = true;
                        expr.push(c);
                    }
                    '{' => {
                        depth += 1;
                        expr.push(c);
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        expr.push(c);
                    }
                    _ => expr.push(c),
                }
                i += 1;
            }
            if depth != 0 {
                return Err("Template: chuỗi chưa đóng '}' khi dùng ${".to_string());
            }
            i += 1; // vượt qua '}'
            parts.push((true, expr));
        } else {
            literal.push(chars[i]);
            i += 1;
        }
    }

    if parts.is_empty() {
        parts.push((false, restore_dollar(&literal)));
    } else if !literal.is_empty() {
        parts.push((false, restore_dollar(&literal)));
    }
    Ok(parts)
}

/// Tách vế trái `name[i][j]...` thành tên biến gốc + danh sách chỉ số.
fn flatten_index(expr: &Expr) -> Option<(&str, Vec<Expr>)> {
    match expr {
        Expr::Variable(name) => Some((name, Vec::new())),
        Expr::Index { target, index } => {
            let (name, mut indices) = flatten_index(target)?;
            indices.push((**index).clone());
            Some((name, indices))
        }
        _ => None,
    }
}

/// Tách vế trái `name.x.y...` thành biến gốc + chuỗi field (cho `p.x = v`).
fn flatten_member(expr: &Expr) -> Option<(&str, Vec<String>)> {
    match expr {
        Expr::Variable(name) => Some((name, Vec::new())),
        Expr::Member { target, name } => {
            let (root, mut path) = flatten_member(target)?;
            path.push(name.clone());
            Some((root, path))
        }
        _ => None,
    }
}

/// Hoàn trả sentinel (do `\$`) về `$` khi tạo giá trị chuỗi thật.
fn restore_dollar(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == crate::token::ESCAPED_DOLLAR {
                '$'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    // Gỡ lớp Ann (số dòng) để test mổ cấu trúc câu lệnh.
    fn unann(s: &Stmt) -> &Stmt {
        match s {
            Stmt::Ann { inner, .. } => inner,
            other => other,
        }
    }

    #[test]
    fn precedence_2_plus_3_mul_4() {
        let stmts = parse("let x = 2 + 3 * 4;");
        let Stmt::Let { value, .. } = unann(&stmts[0]) else {
            panic!("mong đợi let");
        };
        let Expr::Binary { op: plus, right, .. } = value else {
            panic!("mong đợi Binary");
        };
        assert_eq!(*plus, TokenKind::Plus);
        let Expr::Binary { op: star, .. } = &**right else {
            panic!("mong đợi * ở vế phải");
        };
        assert_eq!(*star, TokenKind::Star);
    }

    #[test]
    fn parens_override_precedence() {
        let stmts = parse("let x = (2 + 3) * 4;");
        let Stmt::Let { value, .. } = unann(&stmts[0]) else {
            panic!();
        };
        let Expr::Binary { op, .. } = value else {
            panic!();
        };
        assert_eq!(*op, TokenKind::Star);
    }

    #[test]
    fn match_parses_arms() {
        let stmts = parse("let s = match x { case 1 => 10; case e => e; default => 0; };");
        let Stmt::Let { value, .. } = unann(&stmts[0]) else {
            panic!("mong đợi let");
        };
        let Expr::Match { arms, .. } = value else {
            panic!("mong đợi Match");
        };
        assert_eq!(arms.len(), 3);
        assert!(matches!(&arms[0].pattern, Pattern::Literal(Expr::Number(1))));
        assert!(matches!(&arms[1].pattern, Pattern::Binding(name) if name == "e"));
        assert!(matches!(&arms[2].pattern, Pattern::Wildcard));
    }

    #[test]
    fn for_stmt_parses() {
        let stmts = parse("for i in range(0, 5) { println(i); }");
        let Stmt::For {
            target,
            iterable,
            body,
        } = unann(&stmts[0])
        else {
            panic!("mong đợi For");
        };
        assert!(matches!(target, LetTarget::Var(name) if name == "i"));
        assert!(matches!(iterable, Expr::Call { .. }));
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn list_literal_parses() {
        let stmts = parse("let a = [1, \"hai\", true];");
        let Stmt::Let { value, .. } = unann(&stmts[0]) else {
            panic!();
        };
        let Expr::List(items) = value else {
            panic!("mong đợi List");
        };
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn nested_function_call() {
        let stmts = parse("let x = get(split(\"a,b\", \",\"), 1);");
        let Stmt::Let { value, .. } = unann(&stmts[0]) else {
            panic!();
        };
        assert!(matches!(value, Expr::Call { .. }));
    }

    #[test]
    fn syntax_error_reports_line() {
        let tokens = Lexer::new("let = 3;").tokenize().unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.contains("dòng 1"));
    }

    #[test]
    fn unterminated_block_errors() {
        let tokens = Lexer::new("if true { println(1);").tokenize().unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.contains("mong đợi '}'"));
    }

    #[test]
    fn duplicate_param_errors() {
        let tokens = Lexer::new("func f(a, a) { return a; }").tokenize().unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.contains("trùng tên"));
    }

    #[test]
    fn error_reports_correct_line_multiline() {
        // 'let d = ;' nằm dòng 5 (1,2,blank=3,c=4,d=5). Không được lệch thành 6.
        let src = "let a = 1;\nlet b = 2;\n\nlet c = 3;\nlet d = ;\n";
        let tokens = Lexer::new(src).tokenize().unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.contains("dòng 5"), "lỗi lệch số dòng: {}", err);
    }
}