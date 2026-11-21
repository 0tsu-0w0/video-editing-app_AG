//! v0.9 #14: Pure-Rust 決定論的サンドボックス エクスプレッションエンジン（式アニメーション）
//!
//! ゼロ `unsafe`・新規外部依存なし・ステップ数上限保護付きの再帰下降パーサ＋バイトコード VM。
//!
//! 対応変数:
//! - `time` (シーン時刻秒), `local_time` / `t` (クリップ内時刻秒), `duration` (クリップ尺秒),
//!   `progress` (`0.0..=1.0`), `fps`, `frame`, `value` (キーフレーム評価後の元値),
//!   `index` (レイヤー番号), `width`, `height`, `PI`, `TAU`, `E`
//!
//! 対応関数:
//! - 数学: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `abs`, `floor`, `ceil`,
//!   `round`, `sqrt`, `pow`, `exp`, `log`, `min`, `max`, `clamp`, `sign`, `fract`, `mod`, `deg`, `rad`
//! - 補間: `lerp(a, b, t)`, `linear(t, t0, t1, v0, v1)`, `ease(t, t0, t1, v0, v1)`
//! - プロシージャル: `wiggle(freq, amp[, octaves])`, `noise(x)`, `random(seed)`
//! - ループ: `loopOut(["cycle"|"pingpong"|"offset"|"continue"])`
//! - 音声連動: `audio_band("low"|"mid"|"high"|"rms")`
//! - 他レイヤー参照: `layer("名前または番号").x` (`.y`, `.scale_x`, `.scale_y`, `.rotation_deg`, `.opacity`, `.z`, `.rotation_x_deg`, `.rotation_y_deg`)

use crate::animation::{Interpolation, Keyframe, evaluate_track};
use crate::camera::Transform3D;
use crate::command::EditCommand;
use crate::project::{ItemKind, Scene, TimelineItem, Transform2D};
use crate::time::RationalTime;
use std::fmt;

const MAX_EXPR_LEN: usize = 2048;
const MAX_PARSE_DEPTH: usize = 48;
const MAX_VM_STEPS: usize = 4096;
const MAX_STACK_DEPTH: usize = 128;

/// エクスプレッション構文・評価エラー
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionError {
    pub message: String,
    pub offset: usize,
}

impl ExpressionError {
    fn new(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset,
        }
    }
}

impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (位置 {})", self.message, self.offset)
    }
}

impl std::error::Error for ExpressionError {}

/// `layer(target).prop` 参照解決コールバック型
pub type LayerLookupFn<'a> = &'a dyn Fn(&str, &str) -> Option<f64>;

/// エクスプレッション評価時の実行コンテキスト
pub struct ExpressionContext<'a> {
    pub time: f64,
    pub local_time: f64,
    pub duration: f64,
    pub fps: f64,
    pub value: f64,
    pub layer_index: usize,
    pub scene_width: f64,
    pub scene_height: f64,
    pub seed: u64,
    pub base_value: f32,
    pub keyframes: &'a [Keyframe],
    /// `[low, mid, high, rms]` (0.0..=1.5)
    pub audio_bands: [f32; 4],
    /// `layer(target).prop` の参照解決コールバック
    pub layer_lookup: Option<LayerLookupFn<'a>>,
}

impl<'a> ExpressionContext<'a> {
    pub fn simple(time: f64, value: f64) -> Self {
        Self {
            time,
            local_time: time,
            duration: 5.0,
            fps: 60.0,
            value,
            layer_index: 0,
            scene_width: 1920.0,
            scene_height: 1080.0,
            seed: 42,
            base_value: value as f32,
            keyframes: &[],
            audio_bands: [0.0; 4],
            layer_lookup: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Number(f64),
    Str(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
    Dot,
    Question,
    Colon,
    Bang,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    Eof,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    offset: usize,
}

fn tokenize(src: &str) -> Result<Vec<Token>, ExpressionError> {
    if src.len() > MAX_EXPR_LEN {
        return Err(ExpressionError::new(
            format!("式が長すぎます（上限 {MAX_EXPR_LEN} バイト）"),
            MAX_EXPR_LEN,
        ));
    }
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut tokens = Vec::new();

    while i < bytes.len() {
        let ch = src[i..].chars().next().unwrap();
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        let start = i;
        if ch.is_ascii_digit()
            || (ch == '.' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit()))
        {
            let mut has_dot = ch == '.';
            i += 1;
            while i < bytes.len() {
                let b = bytes[i];
                if b.is_ascii_digit() {
                    i += 1;
                } else if b == b'.' && !has_dot {
                    has_dot = true;
                    i += 1;
                } else if b == b'e' || b == b'E' {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            let num: f64 = src[start..i].parse().map_err(|_| {
                ExpressionError::new(format!("数値リテラルが不正です: {}", &src[start..i]), start)
            })?;
            tokens.push(Token {
                kind: TokenKind::Number(num),
                offset: start,
            });
            continue;
        }
        if ch == '"' || ch == '\'' {
            let quote = ch;
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < src.len() {
                let c = src[i..].chars().next().unwrap();
                i += c.len_utf8();
                if c == '\\' && i < src.len() {
                    let esc = src[i..].chars().next().unwrap();
                    i += esc.len_utf8();
                    match esc {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        _ => s.push(esc),
                    }
                } else if c == quote {
                    closed = true;
                    break;
                } else {
                    s.push(c);
                }
            }
            if !closed {
                return Err(ExpressionError::new(
                    "文字列リテラルが閉じられていません",
                    start,
                ));
            }
            tokens.push(Token {
                kind: TokenKind::Str(s),
                offset: start,
            });
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            i += ch.len_utf8();
            while i < src.len() {
                let c = src[i..].chars().next().unwrap();
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += c.len_utf8();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Ident(src[start..i].to_string()),
                offset: start,
            });
            continue;
        }

        // 2文字演算子
        if i + 1 < bytes.len() {
            let two = &bytes[i..i + 2];
            let kind = match two {
                b"==" => Some(TokenKind::EqEq),
                b"!=" => Some(TokenKind::BangEq),
                b"<=" => Some(TokenKind::LtEq),
                b">=" => Some(TokenKind::GtEq),
                b"&&" => Some(TokenKind::AndAnd),
                b"||" => Some(TokenKind::OrOr),
                _ => None,
            };
            if let Some(k) = kind {
                tokens.push(Token {
                    kind: k,
                    offset: start,
                });
                i += 2;
                continue;
            }
        }

        let kind = match ch {
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '^' => TokenKind::Caret,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            '?' => TokenKind::Question,
            ':' => TokenKind::Colon,
            '!' => TokenKind::Bang,
            '<' => TokenKind::Lt,
            '>' => TokenKind::Gt,
            _ => {
                return Err(ExpressionError::new(
                    format!("認識できない文字 '{ch}'"),
                    start,
                ));
            }
        };
        tokens.push(Token {
            kind,
            offset: start,
        });
        i += ch.len_utf8();
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        offset: src.len(),
    });
    Ok(tokens)
}

#[derive(Debug, Clone)]
enum Value {
    Num(f64),
    Str(String),
}

impl Value {
    fn as_num(&self) -> f64 {
        match self {
            Self::Num(n) => *n,
            Self::Str(s) => s.parse::<f64>().unwrap_or(0.0),
        }
    }

    fn as_truthy(&self) -> bool {
        match self {
            Self::Num(n) => *n != 0.0 && !n.is_nan(),
            Self::Str(s) => !s.is_empty(),
        }
    }

    fn as_str(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            Self::Num(n) => {
                if (n - n.round()).abs() < 1e-6 {
                    format!("{}", n.round() as i64)
                } else {
                    format!("{n}")
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
enum OpCode {
    PushNum(f64),
    PushStr(String),
    LoadVar(String),
    Neg,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    JumpIfFalse(usize),
    Jump(usize),
    Call { name: String, argc: usize },
    LayerProp { prop: String },
}

/// コンパイル済みバイトコード表現
#[derive(Debug, Clone)]
pub struct CompiledExpression {
    ops: Vec<OpCode>,
    uses_audio: bool,
}

impl CompiledExpression {
    pub fn uses_audio(&self) -> bool {
        self.uses_audio
    }

    pub fn evaluate(&self, ctx: &ExpressionContext<'_>) -> Result<f32, ExpressionError> {
        let mut stack: Vec<Value> = Vec::with_capacity(32);
        let mut pc = 0usize;
        let mut steps = 0usize;

        while pc < self.ops.len() {
            steps += 1;
            if steps > MAX_VM_STEPS {
                return Err(ExpressionError::new("実行ステップ上限を超過しました", 0));
            }
            if stack.len() > MAX_STACK_DEPTH {
                return Err(ExpressionError::new("評価スタック上限を超過しました", 0));
            }
            match &self.ops[pc] {
                OpCode::PushNum(n) => stack.push(Value::Num(*n)),
                OpCode::PushStr(s) => stack.push(Value::Str(s.clone())),
                OpCode::LoadVar(name) => {
                    let val = resolve_var(name, ctx)?;
                    stack.push(Value::Num(val));
                }
                OpCode::Neg => {
                    let v = pop_num(&mut stack)?;
                    stack.push(Value::Num(-v));
                }
                OpCode::Not => {
                    let v = pop_val(&mut stack)?;
                    stack.push(Value::Num(if v.as_truthy() { 0.0 } else { 1.0 }));
                }
                OpCode::Add => {
                    let b = pop_num(&mut stack)?;
                    let a = pop_num(&mut stack)?;
                    stack.push(Value::Num(a + b));
                }
                OpCode::Sub => {
                    let b = pop_num(&mut stack)?;
                    let a = pop_num(&mut stack)?;
                    stack.push(Value::Num(a - b));
                }
                OpCode::Mul => {
                    let b = pop_num(&mut stack)?;
                    let a = pop_num(&mut stack)?;
                    stack.push(Value::Num(a * b));
                }
                OpCode::Div => {
                    let b = pop_num(&mut stack)?;
                    let a = pop_num(&mut stack)?;
                    stack.push(Value::Num(if b.abs() < 1e-12 { 0.0 } else { a / b }));
                }
                OpCode::Rem => {
                    let b = pop_num(&mut stack)?;
                    let a = pop_num(&mut stack)?;
                    stack.push(Value::Num(if b.abs() < 1e-12 {
                        0.0
                    } else {
                        a.rem_euclid(b)
                    }));
                }
                OpCode::Pow => {
                    let b = pop_num(&mut stack)?;
                    let a = pop_num(&mut stack)?;
                    let r = a.powf(b);
                    stack.push(Value::Num(if r.is_finite() { r } else { 0.0 }));
                }
                OpCode::Eq => {
                    let b = pop_val(&mut stack)?;
                    let a = pop_val(&mut stack)?;
                    let eq = match (a, b) {
                        (Value::Str(sa), Value::Str(sb)) => sa == sb,
                        (va, vb) => (va.as_num() - vb.as_num()).abs() < 1e-6,
                    };
                    stack.push(Value::Num(if eq { 1.0 } else { 0.0 }));
                }
                OpCode::Ne => {
                    let b = pop_val(&mut stack)?;
                    let a = pop_val(&mut stack)?;
                    let ne = match (a, b) {
                        (Value::Str(sa), Value::Str(sb)) => sa != sb,
                        (va, vb) => (va.as_num() - vb.as_num()).abs() >= 1e-6,
                    };
                    stack.push(Value::Num(if ne { 1.0 } else { 0.0 }));
                }
                OpCode::Lt => cmp_op(&mut stack, |a, b| a < b)?,
                OpCode::Le => cmp_op(&mut stack, |a, b| a <= b)?,
                OpCode::Gt => cmp_op(&mut stack, |a, b| a > b)?,
                OpCode::Ge => cmp_op(&mut stack, |a, b| a >= b)?,
                OpCode::JumpIfFalse(target) => {
                    let cond = pop_val(&mut stack)?;
                    if !cond.as_truthy() {
                        pc = *target;
                        continue;
                    }
                }
                OpCode::Jump(target) => {
                    pc = *target;
                    continue;
                }
                OpCode::Call { name, argc } => {
                    if stack.len() < *argc {
                        return Err(ExpressionError::new("引数が不足しています", 0));
                    }
                    let split_at = stack.len() - *argc;
                    let args: Vec<Value> = stack.drain(split_at..).collect();
                    let res = call_builtin(name, &args, ctx)?;
                    stack.push(res);
                }
                OpCode::LayerProp { prop } => {
                    let layer_key = pop_val(&mut stack)?.as_str();
                    let resolved = ctx
                        .layer_lookup
                        .and_then(|f| f(&layer_key, prop))
                        .unwrap_or(0.0);
                    stack.push(Value::Num(resolved));
                }
            }
            pc += 1;
        }

        let out = stack.pop().map(|v| v.as_num()).unwrap_or(ctx.value);
        if out.is_finite() {
            Ok(out as f32)
        } else {
            Ok(ctx.value as f32)
        }
    }
}

fn pop_val(stack: &mut Vec<Value>) -> Result<Value, ExpressionError> {
    stack
        .pop()
        .ok_or_else(|| ExpressionError::new("スタックアンダーフロー", 0))
}

fn pop_num(stack: &mut Vec<Value>) -> Result<f64, ExpressionError> {
    Ok(pop_val(stack)?.as_num())
}

fn cmp_op(stack: &mut Vec<Value>, f: impl FnOnce(f64, f64) -> bool) -> Result<(), ExpressionError> {
    let b = pop_num(stack)?;
    let a = pop_num(stack)?;
    stack.push(Value::Num(if f(a, b) { 1.0 } else { 0.0 }));
    Ok(())
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    ops: Vec<OpCode>,
    uses_audio: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
            ops: Vec::new(),
            uses_audio: false,
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn enter(&mut self) -> Result<(), ExpressionError> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(ExpressionError::new(
                "式のネストが深すぎます",
                self.peek().offset,
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn parse_expr(&mut self) -> Result<(), ExpressionError> {
        self.enter()?;
        let res = self.parse_ternary();
        self.leave();
        res
    }

    fn parse_ternary(&mut self) -> Result<(), ExpressionError> {
        self.parse_or()?;
        if matches!(self.peek().kind, TokenKind::Question) {
            self.advance();
            let jmp_false = self.ops.len();
            self.ops.push(OpCode::JumpIfFalse(0));
            self.parse_expr()?;
            if !matches!(self.peek().kind, TokenKind::Colon) {
                return Err(ExpressionError::new(
                    "三項演算子に ':' が必要です",
                    self.peek().offset,
                ));
            }
            self.advance();
            let jmp_end = self.ops.len();
            self.ops.push(OpCode::Jump(0));
            let else_pos = self.ops.len();
            self.ops[jmp_false] = OpCode::JumpIfFalse(else_pos);
            self.parse_expr()?;
            let end_pos = self.ops.len();
            self.ops[jmp_end] = OpCode::Jump(end_pos);
        }
        Ok(())
    }

    fn parse_or(&mut self) -> Result<(), ExpressionError> {
        self.parse_and()?;
        while matches!(self.peek().kind, TokenKind::OrOr) {
            self.advance();
            // 短絡 OR: a ? 1 : (b ? 1 : 0)
            let jmp_rhs = self.ops.len();
            self.ops.push(OpCode::JumpIfFalse(0));
            self.ops.push(OpCode::PushNum(1.0));
            let jmp_end = self.ops.len();
            self.ops.push(OpCode::Jump(0));
            let rhs_pos = self.ops.len();
            self.ops[jmp_rhs] = OpCode::JumpIfFalse(rhs_pos);
            self.parse_and()?;
            self.ops.push(OpCode::Not);
            self.ops.push(OpCode::Not);
            let end_pos = self.ops.len();
            self.ops[jmp_end] = OpCode::Jump(end_pos);
        }
        Ok(())
    }

    fn parse_and(&mut self) -> Result<(), ExpressionError> {
        self.parse_equality()?;
        while matches!(self.peek().kind, TokenKind::AndAnd) {
            self.advance();
            let jmp_false = self.ops.len();
            self.ops.push(OpCode::JumpIfFalse(0));
            self.parse_equality()?;
            self.ops.push(OpCode::Not);
            self.ops.push(OpCode::Not);
            let jmp_end = self.ops.len();
            self.ops.push(OpCode::Jump(0));
            let false_pos = self.ops.len();
            self.ops[jmp_false] = OpCode::JumpIfFalse(false_pos);
            self.ops.push(OpCode::PushNum(0.0));
            let end_pos = self.ops.len();
            self.ops[jmp_end] = OpCode::Jump(end_pos);
        }
        Ok(())
    }

    fn parse_equality(&mut self) -> Result<(), ExpressionError> {
        self.parse_comparison()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::EqEq => OpCode::Eq,
                TokenKind::BangEq => OpCode::Ne,
                _ => break,
            };
            self.advance();
            self.parse_comparison()?;
            self.ops.push(op);
        }
        Ok(())
    }

    fn parse_comparison(&mut self) -> Result<(), ExpressionError> {
        self.parse_additive()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Lt => OpCode::Lt,
                TokenKind::LtEq => OpCode::Le,
                TokenKind::Gt => OpCode::Gt,
                TokenKind::GtEq => OpCode::Ge,
                _ => break,
            };
            self.advance();
            self.parse_additive()?;
            self.ops.push(op);
        }
        Ok(())
    }

    fn parse_additive(&mut self) -> Result<(), ExpressionError> {
        self.parse_multiplicative()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => OpCode::Add,
                TokenKind::Minus => OpCode::Sub,
                _ => break,
            };
            self.advance();
            self.parse_multiplicative()?;
            self.ops.push(op);
        }
        Ok(())
    }

    fn parse_multiplicative(&mut self) -> Result<(), ExpressionError> {
        self.parse_unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => OpCode::Mul,
                TokenKind::Slash => OpCode::Div,
                TokenKind::Percent => OpCode::Rem,
                _ => break,
            };
            self.advance();
            self.parse_unary()?;
            self.ops.push(op);
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> Result<(), ExpressionError> {
        match self.peek().kind {
            TokenKind::Minus => {
                self.advance();
                self.parse_unary()?;
                self.ops.push(OpCode::Neg);
                Ok(())
            }
            TokenKind::Plus => {
                self.advance();
                self.parse_unary()
            }
            TokenKind::Bang => {
                self.advance();
                self.parse_unary()?;
                self.ops.push(OpCode::Not);
                Ok(())
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<(), ExpressionError> {
        self.parse_postfix()?;
        if matches!(self.peek().kind, TokenKind::Caret) {
            self.advance();
            self.parse_unary()?;
            self.ops.push(OpCode::Pow);
        }
        Ok(())
    }

    fn parse_postfix(&mut self) -> Result<(), ExpressionError> {
        self.parse_primary()?;
        while matches!(self.peek().kind, TokenKind::Dot) {
            self.advance();
            let tok = self.advance();
            match tok.kind {
                TokenKind::Ident(prop) => {
                    self.ops.push(OpCode::LayerProp { prop });
                }
                _ => {
                    return Err(ExpressionError::new(
                        "'.' の後ろにはプロパティ名が必要です（例: layer(\"背景\").x）",
                        tok.offset,
                    ));
                }
            }
        }
        Ok(())
    }

    fn parse_primary(&mut self) -> Result<(), ExpressionError> {
        let tok = self.advance();
        match tok.kind {
            TokenKind::Number(n) => {
                self.ops.push(OpCode::PushNum(n));
                Ok(())
            }
            TokenKind::Str(s) => {
                self.ops.push(OpCode::PushStr(s));
                Ok(())
            }
            TokenKind::Ident(name) => {
                if matches!(self.peek().kind, TokenKind::LParen) {
                    self.advance();
                    let mut argc = 0usize;
                    if !matches!(self.peek().kind, TokenKind::RParen) {
                        loop {
                            self.parse_expr()?;
                            argc += 1;
                            if matches!(self.peek().kind, TokenKind::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    if !matches!(self.peek().kind, TokenKind::RParen) {
                        return Err(ExpressionError::new(
                            format!("関数 '{name}' の閉じ括弧 ')' がありません"),
                            self.peek().offset,
                        ));
                    }
                    self.advance();
                    validate_builtin_signature(&name, argc, tok.offset)?;
                    if name == "audio_band" {
                        self.uses_audio = true;
                    }
                    self.ops.push(OpCode::Call { name, argc });
                    Ok(())
                } else {
                    validate_var_name(&name, tok.offset)?;
                    self.ops.push(OpCode::LoadVar(name));
                    Ok(())
                }
            }
            TokenKind::LParen => {
                self.parse_expr()?;
                if !matches!(self.peek().kind, TokenKind::RParen) {
                    return Err(ExpressionError::new(
                        "閉じ括弧 ')' が不足しています",
                        self.peek().offset,
                    ));
                }
                self.advance();
                Ok(())
            }
            _ => Err(ExpressionError::new(
                "値・変数・関数呼び出しが必要です",
                tok.offset,
            )),
        }
    }
}

fn validate_var_name(name: &str, offset: usize) -> Result<(), ExpressionError> {
    match name {
        "time" | "local_time" | "t" | "duration" | "progress" | "fps" | "frame" | "value"
        | "index" | "width" | "height" | "PI" | "TAU" | "E" | "true" | "false" => Ok(()),
        _ => Err(ExpressionError::new(format!("未知の変数 '{name}'"), offset)),
    }
}

fn resolve_var(name: &str, ctx: &ExpressionContext<'_>) -> Result<f64, ExpressionError> {
    Ok(match name {
        "time" => ctx.time,
        "local_time" | "t" => ctx.local_time,
        "duration" => ctx.duration,
        "progress" => {
            if ctx.duration > 1e-6 {
                (ctx.local_time / ctx.duration).clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
        "fps" => ctx.fps,
        "frame" => (ctx.time * ctx.fps).floor(),
        "value" => ctx.value,
        "index" => ctx.layer_index as f64,
        "width" => ctx.scene_width,
        "height" => ctx.scene_height,
        "PI" => std::f64::consts::PI,
        "TAU" => std::f64::consts::TAU,
        "E" => std::f64::consts::E,
        "true" => 1.0,
        "false" => 0.0,
        _ => {
            return Err(ExpressionError::new(format!("未知の変数 '{name}'"), 0));
        }
    })
}

fn validate_builtin_signature(
    name: &str,
    argc: usize,
    offset: usize,
) -> Result<(), ExpressionError> {
    let valid = match name {
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "abs" | "floor" | "ceil" | "round"
        | "sqrt" | "exp" | "log" | "sign" | "fract" | "deg" | "rad" | "noise" | "random"
        | "layer" => argc == 1,
        "audio_band" => argc <= 1,
        "loopOut" => argc <= 1,
        "atan2" | "pow" | "min" | "max" | "mod" => argc == 2,
        "wiggle" => argc == 2 || argc == 3,
        "clamp" | "lerp" => argc == 3,
        "linear" | "ease" => argc == 3 || argc == 5,
        _ => {
            return Err(ExpressionError::new(format!("未知の関数 '{name}'"), offset));
        }
    };
    if !valid {
        return Err(ExpressionError::new(
            format!("関数 '{name}' の引数個数 ({argc}) が不正です"),
            offset,
        ));
    }
    Ok(())
}

fn call_builtin(
    name: &str,
    args: &[Value],
    ctx: &ExpressionContext<'_>,
) -> Result<Value, ExpressionError> {
    let num = |idx: usize| args.get(idx).map(Value::as_num).unwrap_or(0.0);
    let out = match name {
        "sin" => num(0).sin(),
        "cos" => num(0).cos(),
        "tan" => num(0).tan(),
        "asin" => num(0).clamp(-1.0, 1.0).asin(),
        "acos" => num(0).clamp(-1.0, 1.0).acos(),
        "atan" => num(0).atan(),
        "atan2" => num(0).atan2(num(1)),
        "abs" => num(0).abs(),
        "floor" => num(0).floor(),
        "ceil" => num(0).ceil(),
        "round" => num(0).round(),
        "sqrt" => num(0).max(0.0).sqrt(),
        "pow" => num(0).powf(num(1)),
        "exp" => num(0).clamp(-40.0, 40.0).exp(),
        "log" => num(0).max(1e-12).ln(),
        "min" => num(0).min(num(1)),
        "max" => num(0).max(num(1)),
        "clamp" => {
            let lo = num(1).min(num(2));
            let hi = num(1).max(num(2));
            num(0).clamp(lo, hi)
        }
        "sign" => {
            let v = num(0);
            if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "fract" => num(0).rem_euclid(1.0),
        "mod" => {
            let d = num(1);
            if d.abs() < 1e-12 {
                0.0
            } else {
                num(0).rem_euclid(d)
            }
        }
        "deg" => num(0).to_degrees(),
        "rad" => num(0).to_radians(),
        "lerp" => {
            let a = num(0);
            let b = num(1);
            let t = num(2);
            a + (b - a) * t
        }
        "linear" => {
            if args.len() == 3 {
                let u = num(0).clamp(0.0, 1.0);
                num(1) + (num(2) - num(1)) * u
            } else {
                let (t, t0, t1, v0, v1) = (num(0), num(1), num(2), num(3), num(4));
                let span = t1 - t0;
                let u = if span.abs() < 1e-9 {
                    0.0
                } else {
                    ((t - t0) / span).clamp(0.0, 1.0)
                };
                v0 + (v1 - v0) * u
            }
        }
        "ease" => {
            let (u, v0, v1) = if args.len() == 3 {
                (num(0).clamp(0.0, 1.0), num(1), num(2))
            } else {
                let (t, t0, t1, v0, v1) = (num(0), num(1), num(2), num(3), num(4));
                let span = t1 - t0;
                let u = if span.abs() < 1e-9 {
                    0.0
                } else {
                    ((t - t0) / span).clamp(0.0, 1.0)
                };
                (u, v0, v1)
            };
            let s = u * u * (3.0 - 2.0 * u);
            v0 + (v1 - v0) * s
        }
        "noise" => value_noise_1d(num(0), ctx.seed),
        "random" => hash_u64_to_unit(ctx.seed ^ (num(0).to_bits())),
        "wiggle" => {
            let freq = num(0).max(0.0);
            let amp = num(1);
            let octaves = if args.len() >= 3 {
                (num(2).round() as usize).clamp(1, 6)
            } else {
                2
            };
            eval_wiggle(ctx.time, freq, amp, octaves, ctx.seed)
        }
        "loopOut" => {
            let mode = args
                .first()
                .map(Value::as_str)
                .unwrap_or_else(|| "cycle".to_string());
            eval_loop_out(&mode, ctx)
        }
        "audio_band" => {
            let band = args
                .first()
                .map(Value::as_str)
                .unwrap_or_else(|| "rms".to_string());
            let v = match band.to_ascii_lowercase().as_str() {
                "low" | "bass" => ctx.audio_bands[0],
                "mid" => ctx.audio_bands[1],
                "high" | "treble" => ctx.audio_bands[2],
                _ => ctx.audio_bands[3],
            };
            f64::from(v)
        }
        "layer" => {
            // `layer("名前").x` 用に識別子文字列を返す
            return Ok(Value::Str(args[0].as_str()));
        }
        _ => {
            return Err(ExpressionError::new(format!("未知の関数 '{name}'"), 0));
        }
    };
    Ok(Value::Num(if out.is_finite() { out } else { 0.0 }))
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn hash_u64_to_unit(x: u64) -> f64 {
    let h = splitmix64(x);
    ((h >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

fn value_noise_1d(x: f64, seed: u64) -> f64 {
    let i0 = x.floor() as i64;
    let frac = x - (i0 as f64);
    // Quintic smoothstep で滑らかな2次連続性を確保
    let u = frac * frac * frac * (frac * (frac * 6.0 - 15.0) + 10.0);
    let n0 = hash_u64_to_unit(seed ^ (i0 as u64).wrapping_mul(0x517C_C1B7_2722_0A95)) * 2.0 - 1.0;
    let n1 =
        hash_u64_to_unit(seed ^ ((i0.wrapping_add(1)) as u64).wrapping_mul(0x517C_C1B7_2722_0A95))
            * 2.0
            - 1.0;
    n0 + (n1 - n0) * u
}

fn eval_wiggle(time: f64, freq: f64, amp: f64, octaves: usize, seed: u64) -> f64 {
    if freq <= 0.0 || amp == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut f = freq;
    let mut a = 1.0;
    for oct in 0..octaves {
        let oct_seed = splitmix64(seed.wrapping_add((oct as u64 + 1) * 0x9E37_79B9));
        sum += value_noise_1d(time * f, oct_seed) * a;
        norm += a;
        f *= 2.0;
        a *= 0.5;
    }
    if norm > 0.0 { (sum / norm) * amp } else { 0.0 }
}

fn eval_loop_out(mode: &str, ctx: &ExpressionContext<'_>) -> f64 {
    let kfs = ctx.keyframes;
    if kfs.len() < 2 {
        return ctx.value;
    }
    let first = &kfs[0];
    let last = &kfs[kfs.len() - 1];
    let t0 = first.time.as_seconds();
    let t1 = last.time.as_seconds();
    let span = t1 - t0;
    if span <= 1e-6 || ctx.local_time <= t1 {
        return ctx.value;
    }

    let elapsed = ctx.local_time - t0;
    match mode.to_ascii_lowercase().as_str() {
        "pingpong" => {
            let cycle = (elapsed / span).floor() as i64;
            let rem = elapsed - (cycle as f64) * span;
            let mapped = if cycle % 2 == 0 { t0 + rem } else { t1 - rem };
            f64::from(evaluate_track(
                ctx.base_value,
                kfs,
                RationalTime::from_seconds(mapped),
            ))
        }
        "offset" => {
            let cycles = (elapsed / span).floor();
            let rem = elapsed - cycles * span;
            let base = f64::from(evaluate_track(
                ctx.base_value,
                kfs,
                RationalTime::from_seconds(t0 + rem),
            ));
            let delta = f64::from(last.value - first.value);
            base + cycles * delta
        }
        "continue" => {
            let prev = &kfs[kfs.len() - 2];
            let dt = (last.time - prev.time).as_seconds();
            let slope = if dt.abs() > 1e-6 {
                f64::from(last.value - prev.value) / dt
            } else {
                0.0
            };
            f64::from(last.value) + slope * (ctx.local_time - t1)
        }
        _ => {
            // "cycle"
            let rem = elapsed.rem_euclid(span);
            f64::from(evaluate_track(
                ctx.base_value,
                kfs,
                RationalTime::from_seconds(t0 + rem),
            ))
        }
    }
}

/// 式文字列をバイトコードへコンパイル（構文検証も兼ねる）
pub fn compile_expression(src: &str) -> Result<CompiledExpression, ExpressionError> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return Err(ExpressionError::new("式が空です", 0));
    }
    let tokens = tokenize(trimmed)?;
    let mut parser = Parser::new(tokens);
    parser.parse_expr()?;
    if !matches!(parser.peek().kind, TokenKind::Eof) {
        return Err(ExpressionError::new(
            "式の末尾に余分なトークンがあります",
            parser.peek().offset,
        ));
    }
    Ok(CompiledExpression {
        ops: parser.ops,
        uses_audio: parser.uses_audio,
    })
}

/// 式文字列を 1 回でコンパイル＆評価するヘルパー
pub fn evaluate_expression(src: &str, ctx: &ExpressionContext<'_>) -> Result<f32, ExpressionError> {
    let prog = compile_expression(src)?;
    prog.evaluate(ctx)
}

/// エクスプレッション対象の標準プロパティ名一覧
pub const EXPRESSION_PROPERTIES: &[&str] = &[
    "x",
    "y",
    "scale_x",
    "scale_y",
    "rotation_deg",
    "opacity",
    "z",
    "rotation_x_deg",
    "rotation_y_deg",
    "trim_start",
    "trim_end",
    "trim_offset",
];

/// エクスプレッションのプリセットスニペット `(名称, 推奨プロパティ, 式)`
pub fn expression_presets() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("カメラ手ブレ (Wiggle)", "x", "value + wiggle(2.5, 18)"),
        ("上下ふわふわ浮遊", "y", "value + sin(time * PI * 1.5) * 20"),
        (
            "鼓動バウンス拡大",
            "scale_x",
            "value + abs(sin(time * PI * 2)) * 0.12",
        ),
        ("等速回転 (36°/秒)", "rotation_deg", "value + time * 36"),
        ("無限ピンポンループ", "x", "loopOut(\"pingpong\")"),
        ("無限サイクルループ", "x", "loopOut(\"cycle\")"),
        (
            "低音ビート拡大",
            "scale_x",
            "value + audio_band(\"low\") * 0.35",
        ),
        (
            "スムーズフェードイン",
            "opacity",
            "ease(local_time, 0, 0.5, 0, 1)",
        ),
        ("他レイヤー追従 (+120px)", "x", "layer(\"L0\").x + 120"),
    ]
}

fn hash_seed(item_id: uuid::Uuid, property: &str) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325_u64;
    for b in item_id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    for b in property.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    h
}

/// シーン内の他レイヤーまたはアイテム名からプロパティ値を解決する（再帰ループ防止付き）
fn lookup_scene_layer_property(
    scene: &Scene,
    global_time: RationalTime,
    target: &str,
    prop: &str,
) -> Option<f64> {
    let trimmed = target.trim();
    // "0" または "L0" / "l0" 形式のレイヤー番号指定を優先解決
    let layer_idx_opt = trimmed
        .strip_prefix('L')
        .or_else(|| trimmed.strip_prefix('l'))
        .unwrap_or(trimmed)
        .parse::<usize>()
        .ok();

    let found_item = if let Some(l_idx) = layer_idx_opt
        && let Some(layer) = scene.layers.get(l_idx)
    {
        layer
            .items
            .iter()
            .find(|i| i.contains(global_time))
            .or_else(|| layer.items.first())
    } else {
        // アイテム名またはレイヤー名で一致検索
        scene.layers.iter().find_map(|layer| {
            if layer.name.eq_ignore_ascii_case(trimmed) {
                layer
                    .items
                    .iter()
                    .find(|i| i.contains(global_time))
                    .or_else(|| layer.items.first())
            } else {
                layer
                    .items
                    .iter()
                    .find(|i| i.name.eq_ignore_ascii_case(trimmed) && i.contains(global_time))
                    .or_else(|| {
                        layer
                            .items
                            .iter()
                            .find(|i| i.name.eq_ignore_ascii_case(trimmed))
                    })
            }
        })
    }?;

    // 循環参照を防ぐため参照先の基本/キーフレーム値（式評価前）を返す
    let tf2 = found_item.eval_transform(global_time);
    let tf3 = found_item.eval_transform_3d(global_time);
    let v = match prop {
        "x" => tf2.x,
        "y" => tf2.y,
        "scale_x" => tf2.scale_x,
        "scale_y" => tf2.scale_y,
        "rotation_deg" | "rotation" => tf2.rotation_deg,
        "opacity" => tf2.opacity,
        "z" => tf3.z,
        "rotation_x_deg" => tf3.rotation_x_deg,
        "rotation_y_deg" => tf3.rotation_y_deg,
        _ => return None,
    };
    Some(f64::from(v))
}

/// 指定アイテムの 1 プロパティをキーフレーム＋エクスプレッション込みで評価する
pub fn evaluate_item_property(
    scene: &Scene,
    layer_index: usize,
    item: &TimelineItem,
    property: &str,
    global_time: RationalTime,
    audio_bands: [f32; 4],
) -> f32 {
    let base = item_base_property(item, property);
    let local = global_time - item.start;
    let kfs = item
        .keyframes
        .get(property)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let kf_val = if kfs.is_empty() {
        base
    } else {
        evaluate_track(base, kfs, local)
    };

    let Some(expr) = item.expressions.get(property) else {
        return kf_val;
    };
    if expr.trim().is_empty() {
        return kf_val;
    }

    let lookup =
        |target: &str, prop: &str| lookup_scene_layer_property(scene, global_time, target, prop);
    let ctx = ExpressionContext {
        time: global_time.as_seconds(),
        local_time: local.as_seconds(),
        duration: item.duration.as_seconds(),
        fps: scene.fps.as_f64(),
        value: f64::from(kf_val),
        layer_index,
        scene_width: f64::from(scene.width),
        scene_height: f64::from(scene.height),
        seed: hash_seed(item.id, property),
        base_value: base,
        keyframes: kfs,
        audio_bands,
        layer_lookup: Some(&lookup),
    };
    evaluate_expression(expr, &ctx).unwrap_or(kf_val)
}

/// シーン文脈でアイテムの `Transform2D` / `Transform3D` をエクスプレッション込みで評価する
pub fn evaluate_item_transform(
    scene: &Scene,
    layer_index: usize,
    item: &TimelineItem,
    global_time: RationalTime,
    audio_bands: [f32; 4],
) -> (Transform2D, Transform3D) {
    let mut tf2 = item.eval_transform(global_time);
    let mut tf3 = item.eval_transform_3d(global_time);
    if item.expressions.is_empty() {
        return (tf2, tf3);
    }

    let eval_if = |prop: &str, current: f32| -> f32 {
        if item.expressions.contains_key(prop) {
            evaluate_item_property(scene, layer_index, item, prop, global_time, audio_bands)
        } else {
            current
        }
    };

    tf2.x = eval_if("x", tf2.x);
    tf2.y = eval_if("y", tf2.y);
    tf2.scale_x = eval_if("scale_x", tf2.scale_x).clamp(0.01, 20.0);
    tf2.scale_y = eval_if("scale_y", tf2.scale_y).clamp(0.01, 20.0);
    tf2.rotation_deg = eval_if("rotation_deg", tf2.rotation_deg);
    tf2.opacity = eval_if("opacity", tf2.opacity).clamp(0.0, 1.0);

    tf3.z = eval_if("z", tf3.z);
    tf3.rotation_x_deg = eval_if("rotation_x_deg", tf3.rotation_x_deg);
    tf3.rotation_y_deg = eval_if("rotation_y_deg", tf3.rotation_y_deg);

    (tf2, tf3)
}

/// アイテムのエクスプレッションに `audio_band(...)` が含まれるか高速判定
pub fn item_expressions_use_audio(item: &TimelineItem) -> bool {
    item.expressions.values().any(|s| s.contains("audio_band"))
}

/// 指定プロパティのベース値（キーフレーム無し時の値）を取得する
pub fn item_base_property(item: &TimelineItem, property: &str) -> f32 {
    match property {
        "x" => item.transform.x,
        "y" => item.transform.y,
        "scale_x" => item.transform.scale_x,
        "scale_y" => item.transform.scale_y,
        "rotation_deg" => item.transform.rotation_deg,
        "opacity" => item.transform.opacity,
        "z" => item.transform_3d.z,
        "rotation_x_deg" => item.transform_3d.rotation_x_deg,
        "rotation_y_deg" => item.transform_3d.rotation_y_deg,
        "trim_start" => match &item.kind {
            ItemKind::VectorShape { config } => config.trim_start,
            _ => 0.0,
        },
        "trim_end" => match &item.kind {
            ItemKind::VectorShape { config } => config.trim_end,
            _ => 1.0,
        },
        "trim_offset" => match &item.kind {
            ItemKind::VectorShape { config } => config.trim_offset,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

/// エクスプレッションの軌跡を標準キーフレームへ焼き込み（Bake）、式を解除する 1 ステップ Undo コマンドを生成する
pub fn build_bake_expression_command(
    scene: &Scene,
    layer_index: usize,
    item: &TimelineItem,
    property: &str,
    step_frames: u32,
    audio_bands_fn: impl Fn(RationalTime) -> [f32; 4],
) -> Result<EditCommand, String> {
    let expr = item
        .expressions
        .get(property)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            format!("プロパティ '{property}' にエクスプレッションが設定されていません")
        })?;
    compile_expression(expr).map_err(|e| e.to_string())?;

    let step = step_frames.clamp(1, 60) as i64;
    let total_frames = item.duration.to_frame_floor(scene.fps).max(1);
    let max_samples = 240usize;
    let effective_step = step.max((total_frames / max_samples as i64).max(1));

    let mut commands = Vec::new();
    let mut f = 0i64;
    while f <= total_frames {
        let local_t = RationalTime::from_frame(f, scene.fps).min(item.duration);
        let global_t = item.start + local_t;
        let bands = audio_bands_fn(global_t);
        let val = evaluate_item_property(scene, layer_index, item, property, global_t, bands);
        commands.push(EditCommand::SetKeyframe {
            item_id: item.id,
            property: property.to_string(),
            keyframe: Keyframe {
                time: local_t,
                value: val,
                interpolation: Interpolation::Linear,
            },
        });
        if f == total_frames {
            break;
        }
        f = (f + effective_step).min(total_frames);
    }

    commands.push(EditCommand::SetExpression {
        item_id: item.id,
        property: property.to_string(),
        expression: None,
    });

    Ok(EditCommand::Batch {
        label: format!("bake_expression:{property}"),
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Layer, Project, ProjectSettings, RgbaColor, ShapeKind};
    use crate::time::FrameRate;

    #[test]
    fn evaluates_math_ternary_interpolation_and_wiggle_deterministically() {
        let ctx = ExpressionContext::simple(1.25, 100.0);
        let v =
            evaluate_expression("value + sin(PI / 2) * 50 + (time > 1 ? 10 : -10)", &ctx).unwrap();
        assert!((v - 160.0).abs() < 1e-3);

        let eased = evaluate_expression("ease(0.5, 0, 1, 0, 100)", &ctx).unwrap();
        assert!((eased - 50.0).abs() < 1e-3);

        // wiggle は同一時刻・同一シードで完全一致する決定論的結果
        let w1 = evaluate_expression("wiggle(3, 25, 3)", &ctx).unwrap();
        let w2 = evaluate_expression("wiggle(3, 25, 3)", &ctx).unwrap();
        assert_eq!(w1, w2);
        assert!(w1.abs() <= 25.001);
    }

    #[test]
    fn loop_out_cycle_pingpong_and_offset_work_past_last_keyframe() {
        let kfs = vec![
            Keyframe {
                time: RationalTime::ZERO,
                value: 0.0,
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: RationalTime::from_seconds(1.0),
                value: 100.0,
                interpolation: Interpolation::Linear,
            },
        ];
        let mut ctx = ExpressionContext::simple(1.5, 100.0);
        ctx.local_time = 1.5;
        ctx.keyframes = &kfs;

        let cyc = evaluate_expression("loopOut(\"cycle\")", &ctx).unwrap();
        assert!((cyc - 50.0).abs() < 1e-2, "cycle expected 50, got {cyc}");

        let pp = evaluate_expression("loopOut(\"pingpong\")", &ctx).unwrap();
        assert!((pp - 50.0).abs() < 1e-2, "pingpong expected 50, got {pp}");

        ctx.local_time = 1.25;
        let pp2 = evaluate_expression("loopOut(\"pingpong\")", &ctx).unwrap();
        assert!(
            (pp2 - 75.0).abs() < 1e-2,
            "pingpong at 1.25 expected 75, got {pp2}"
        );

        let off = evaluate_expression("loopOut(\"offset\")", &ctx).unwrap();
        assert!(
            (off - 125.0).abs() < 1e-2,
            "offset at 1.25 expected 125, got {off}"
        );
    }

    #[test]
    fn audio_band_layer_reference_and_bake_to_keyframes_work() {
        let mut project = Project::new("expr_test", ProjectSettings::default());
        let scene = project.active_scene_mut().unwrap();
        scene.fps = FrameRate::FPS_30;
        let mut leader = TimelineItem::new(
            "Leader",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: 100.0,
                height: 100.0,
                color: RgbaColor::WHITE,
            },
        );
        leader.transform.x = 200.0;
        let mut follower = TimelineItem::new(
            "Follower",
            RationalTime::ZERO,
            RationalTime::from_seconds(2.0),
            ItemKind::Shape {
                shape: ShapeKind::Ellipse,
                width: 80.0,
                height: 80.0,
                color: RgbaColor::WHITE,
            },
        );
        follower.expressions.insert(
            "x".into(),
            "layer(\"Leader\").x + audio_band(\"low\") * 100".into(),
        );
        let follower_id = follower.id;
        scene.layers = vec![
            Layer {
                items: vec![leader],
                ..Layer::new("L0")
            },
            Layer {
                items: vec![follower],
                ..Layer::new("L1")
            },
        ];

        let scene_ref = project.active_scene().unwrap();
        let f_item = &scene_ref.layers[1].items[0];
        let (tf2, _) = evaluate_item_transform(
            scene_ref,
            1,
            f_item,
            RationalTime::from_seconds(0.5),
            [0.5, 0.0, 0.0, 0.3],
        );
        assert!((tf2.x - 250.0).abs() < 1e-2);

        let bake_cmd =
            build_bake_expression_command(scene_ref, 1, f_item, "x", 15, |_| [0.5, 0.0, 0.0, 0.3])
                .unwrap();
        let mut bus = crate::command::CommandBus::default();
        bus.execute(
            &mut project,
            bake_cmd,
            crate::command::CommandSource::Gui,
            None,
        )
        .unwrap();
        let baked = project.active_scene().unwrap().layers[1]
            .items
            .iter()
            .find(|i| i.id == follower_id)
            .unwrap();
        assert!(!baked.expressions.contains_key("x"));
        assert!(baked.keyframes.get("x").map(Vec::len).unwrap_or(0) >= 3);
    }

    #[test]
    fn rejects_invalid_syntax_and_unknown_identifiers() {
        assert!(compile_expression("1 + * 2").is_err());
        assert!(compile_expression("unknown_var + 1").is_err());
        assert!(compile_expression("evil_fn(1)").is_err());
        assert!(compile_expression("\"unterminated").is_err());
    }
}
