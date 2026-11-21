use crate::{
    EffectPlugin, PLUGIN_API_VERSION, PluginCapability, PluginManifest, PluginParamDef,
    PluginRegistry,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

const WASM_MAGIC: [u8; 4] = *b"\0asm";
const WASM_VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];
const WASM_PAGE_SIZE: usize = 65_536;
const MAX_MEMORY_PAGES: usize = 64; // 4 MiB sandbox cap
const MAX_CALL_DEPTH: usize = 32;
const DEFAULT_FUEL_PER_CALL: u64 = 25_000_000;
const PIXEL_BUFFER_PTR: usize = 4096;
const PIXEL_CHUNK_LEN: usize = 2048; // 2048 * 16B = 32 KiB (fits inside 1 page with room to spare)

#[derive(Debug, Error)]
pub enum WasmError {
    #[error("Invalid WebAssembly magic or version header")]
    InvalidHeader,
    #[error("Unexpected end of WebAssembly binary")]
    UnexpectedEof,
    #[error("Malformed LEB128 or section payload: {0}")]
    Malformed(String),
    #[error("Host imports are forbidden in sandboxed WASM plugins")]
    ImportsForbidden,
    #[error("Unsupported WebAssembly opcode: 0x{0:02X}")]
    UnsupportedOpcode(u8),
    #[error("Missing required export: {0}")]
    MissingExport(String),
    #[error("Incompatible plugin API version: {found} (expected {expected})")]
    IncompatibleApiVersion { found: i32, expected: i32 },
    #[error("WASM execution trapped: unreachable")]
    Unreachable,
    #[error("WASM execution trapped: memory out of bounds")]
    MemoryOutOfBounds,
    #[error("WASM execution trapped: division by zero")]
    DivisionByZero,
    #[error("WASM execution trapped: stack underflow or type mismatch")]
    StackError,
    #[error("WASM execution trapped: call stack overflow")]
    CallStackOverflow,
    #[error("WASM execution trapped: instruction fuel limit exceeded (infinite loop protection)")]
    FuelExhausted,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValType {
    I32,
    I64,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WasmVal {
    I32(i32),
    F32(f32),
}

impl WasmVal {
    fn as_i32(self) -> Result<i32, WasmError> {
        match self {
            Self::I32(v) => Ok(v),
            _ => Err(WasmError::StackError),
        }
    }

    fn as_f32(self) -> Result<f32, WasmError> {
        match self {
            Self::F32(v) => Ok(v),
            _ => Err(WasmError::StackError),
        }
    }
}

#[derive(Debug, Clone)]
struct FuncType {
    params: Vec<ValType>,
    results: Vec<ValType>,
}

#[derive(Debug, Clone)]
struct WasmFunc {
    type_idx: u32,
    locals: Vec<ValType>,
    code: Vec<Instr>,
}

#[derive(Debug, Clone)]
struct DataSegment {
    offset: usize,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
enum Instr {
    Unreachable,
    Nop,
    Block {
        end_pc: usize,
    },
    Loop {
        end_pc: usize,
    },
    If {
        else_or_end_pc: usize,
        end_pc: usize,
    },
    Else {
        end_pc: usize,
    },
    End,
    Br(u32),
    BrIf(u32),
    Return,
    Call(u32),
    Drop,
    Select,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    I32Load(u32),
    F32Load(u32),
    I32Load8U(u32),
    I32Store(u32),
    F32Store(u32),
    I32Store8(u32),
    MemorySize,
    MemoryGrow,
    I32Const(i32),
    F32Const(f32),
    I32Eqz,
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtS,
    I32GtU,
    I32LeS,
    I32LeU,
    I32GeS,
    I32GeU,
    F32Eq,
    F32Ne,
    F32Lt,
    F32Gt,
    F32Le,
    F32Ge,
    I32Add,
    I32Sub,
    I32Mul,
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrS,
    I32ShrU,
    F32Abs,
    F32Neg,
    F32Ceil,
    F32Floor,
    F32Trunc,
    F32Nearest,
    F32Sqrt,
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32Min,
    F32Max,
    I32TruncF32S,
    F32ConvertI32S,
    F32ConvertI32U,
    I32ReinterpretF32,
    F32ReinterpretI32,
}

/// Pure-Rust ゼロ `unsafe` WebAssembly 1.0 モジュール（パース済み不変表現）
#[derive(Debug, Clone)]
pub struct WasmModule {
    types: Vec<FuncType>,
    funcs: Vec<WasmFunc>,
    initial_mem_pages: usize,
    max_mem_pages: usize,
    exports_func: BTreeMap<String, usize>,
    has_exported_memory: bool,
    data_segments: Vec<DataSegment>,
}

impl WasmModule {
    pub fn parse(bytes: &[u8]) -> Result<Self, WasmError> {
        if bytes.len() < 8 || bytes[0..4] != WASM_MAGIC || bytes[4..8] != WASM_VERSION {
            return Err(WasmError::InvalidHeader);
        }
        let mut r = BinaryReader::new(&bytes[8..]);
        let mut types = Vec::new();
        let mut func_type_indices = Vec::new();
        let mut initial_mem_pages = 1usize;
        let mut max_mem_pages = MAX_MEMORY_PAGES;
        let mut exports_func = BTreeMap::new();
        let mut has_exported_memory = false;
        let mut funcs = Vec::new();
        let mut data_segments = Vec::new();

        while !r.is_eof() {
            let section_id = r.read_u8()?;
            let section_len = r.read_leb_u32()? as usize;
            let section_bytes = r.read_bytes(section_len)?;
            let mut sr = BinaryReader::new(section_bytes);

            match section_id {
                0 => {
                    // Custom section (name / debug info) — safely ignored
                }
                1 => {
                    // Type section
                    let count = sr.read_leb_u32()? as usize;
                    for _ in 0..count {
                        let tag = sr.read_u8()?;
                        if tag != 0x60 {
                            return Err(WasmError::Malformed(format!(
                                "expected 0x60 func type, got 0x{tag:02X}"
                            )));
                        }
                        let p_cnt = sr.read_leb_u32()? as usize;
                        let mut params = Vec::with_capacity(p_cnt);
                        for _ in 0..p_cnt {
                            params.push(decode_val_type(sr.read_u8()?)?);
                        }
                        let r_cnt = sr.read_leb_u32()? as usize;
                        let mut results = Vec::with_capacity(r_cnt);
                        for _ in 0..r_cnt {
                            results.push(decode_val_type(sr.read_u8()?)?);
                        }
                        types.push(FuncType { params, results });
                    }
                }
                2 => {
                    // Import section — strictly forbidden in sandbox
                    let count = sr.read_leb_u32()?;
                    if count > 0 {
                        return Err(WasmError::ImportsForbidden);
                    }
                }
                3 => {
                    // Function section
                    let count = sr.read_leb_u32()? as usize;
                    for _ in 0..count {
                        func_type_indices.push(sr.read_leb_u32()?);
                    }
                }
                5 => {
                    // Memory section
                    let count = sr.read_leb_u32()? as usize;
                    if count > 0 {
                        let flags = sr.read_u8()?;
                        let min_p = sr.read_leb_u32()? as usize;
                        let max_p = if flags & 1 != 0 {
                            (sr.read_leb_u32()? as usize).min(MAX_MEMORY_PAGES)
                        } else {
                            MAX_MEMORY_PAGES
                        };
                        if min_p > MAX_MEMORY_PAGES {
                            return Err(WasmError::Malformed(format!(
                                "memory initial pages {min_p} exceeds sandbox cap {MAX_MEMORY_PAGES}"
                            )));
                        }
                        initial_mem_pages = min_p.max(1);
                        max_mem_pages = max_p.max(initial_mem_pages);
                    }
                }
                7 => {
                    // Export section
                    let count = sr.read_leb_u32()? as usize;
                    for _ in 0..count {
                        let name_len = sr.read_leb_u32()? as usize;
                        let name_bytes = sr.read_bytes(name_len)?;
                        let name = std::str::from_utf8(name_bytes)
                            .map_err(|_| WasmError::Malformed("non-UTF8 export name".into()))?
                            .to_string();
                        let kind = sr.read_u8()?;
                        let index = sr.read_leb_u32()? as usize;
                        match kind {
                            0x00 => {
                                exports_func.insert(name, index);
                            }
                            0x02 => {
                                has_exported_memory = true;
                            }
                            _ => {}
                        }
                    }
                }
                10 => {
                    // Code section
                    let count = sr.read_leb_u32()? as usize;
                    if count != func_type_indices.len() {
                        return Err(WasmError::Malformed(
                            "code count does not match function section count".into(),
                        ));
                    }
                    for &type_idx in &func_type_indices {
                        if (type_idx as usize) >= types.len() {
                            return Err(WasmError::Malformed("invalid function type index".into()));
                        }
                        let body_size = sr.read_leb_u32()? as usize;
                        let body_bytes = sr.read_bytes(body_size)?;
                        let mut br = BinaryReader::new(body_bytes);
                        let local_decl_cnt = br.read_leb_u32()? as usize;
                        let mut locals = Vec::new();
                        for _ in 0..local_decl_cnt {
                            let n = br.read_leb_u32()? as usize;
                            let vt = decode_val_type(br.read_u8()?)?;
                            if locals.len().saturating_add(n) > 1024 {
                                return Err(WasmError::Malformed("too many locals".into()));
                            }
                            for _ in 0..n {
                                locals.push(vt);
                            }
                        }
                        let code = decode_instructions(&mut br)?;
                        funcs.push(WasmFunc {
                            type_idx,
                            locals,
                            code,
                        });
                    }
                }
                11 => {
                    // Data section (active segments)
                    let count = sr.read_leb_u32()? as usize;
                    for _ in 0..count {
                        let mode = sr.read_leb_u32()?;
                        if mode == 0 {
                            let op = sr.read_u8()?;
                            if op != 0x41 {
                                return Err(WasmError::Malformed(
                                    "data segment offset must begin with i32.const".into(),
                                ));
                            }
                            let offset = sr.read_leb_i32()?.max(0) as usize;
                            let end_op = sr.read_u8()?;
                            if end_op != 0x0B {
                                return Err(WasmError::Malformed(
                                    "data segment init expr missing end".into(),
                                ));
                            }
                            let dlen = sr.read_leb_u32()? as usize;
                            let dbytes = sr.read_bytes(dlen)?.to_vec();
                            data_segments.push(DataSegment {
                                offset,
                                bytes: dbytes,
                            });
                        }
                    }
                }
                _ => {
                    // Other non-essential sections (Table/Global/Element) skipped safely
                }
            }
        }

        Ok(Self {
            types,
            funcs,
            initial_mem_pages,
            max_mem_pages,
            exports_func,
            has_exported_memory,
            data_segments,
        })
    }

    pub fn export_param_count(&self, name: &str) -> Option<usize> {
        let &fidx = self.exports_func.get(name)?;
        let f = self.funcs.get(fidx)?;
        let ty = self.types.get(f.type_idx as usize)?;
        Some(ty.params.len())
    }

    pub fn instantiate(&self) -> Result<WasmInstance<'_>, WasmError> {
        let mut memory = vec![0u8; self.initial_mem_pages * WASM_PAGE_SIZE];
        for seg in &self.data_segments {
            let end = seg
                .offset
                .checked_add(seg.bytes.len())
                .ok_or(WasmError::MemoryOutOfBounds)?;
            if end > memory.len() {
                return Err(WasmError::MemoryOutOfBounds);
            }
            memory[seg.offset..end].copy_from_slice(&seg.bytes);
        }
        Ok(WasmInstance {
            module: self,
            memory,
            fuel: DEFAULT_FUEL_PER_CALL,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct LabelFrame {
    is_loop: bool,
    start_pc: usize,
    end_pc: usize,
    stack_height: usize,
}

/// 実体化されたサンドボックス WASM VM インスタンス
pub struct WasmInstance<'a> {
    module: &'a WasmModule,
    pub memory: Vec<u8>,
    pub fuel: u64,
}

impl<'a> WasmInstance<'a> {
    pub fn call_export(&mut self, name: &str, args: &[WasmVal]) -> Result<Vec<WasmVal>, WasmError> {
        let &fidx = self
            .module
            .exports_func
            .get(name)
            .ok_or_else(|| WasmError::MissingExport(name.to_string()))?;
        self.call_func(fidx, args, 0)
    }

    fn call_func(
        &mut self,
        fidx: usize,
        args: &[WasmVal],
        depth: usize,
    ) -> Result<Vec<WasmVal>, WasmError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(WasmError::CallStackOverflow);
        }
        let func = self
            .module
            .funcs
            .get(fidx)
            .ok_or_else(|| WasmError::Malformed(format!("invalid func index {fidx}")))?;
        let ftype = &self.module.types[func.type_idx as usize];
        if args.len() != ftype.params.len() {
            return Err(WasmError::StackError);
        }

        let mut locals = Vec::with_capacity(args.len() + func.locals.len());
        locals.extend_from_slice(args);
        for &lt in &func.locals {
            locals.push(match lt {
                ValType::F32 | ValType::F64 => WasmVal::F32(0.0),
                ValType::I32 | ValType::I64 => WasmVal::I32(0),
            });
        }

        let mut stack: Vec<WasmVal> = Vec::with_capacity(32);
        let mut labels: Vec<LabelFrame> = Vec::with_capacity(16);
        let code = &func.code;
        let mut pc = 0usize;

        while pc < code.len() {
            if self.fuel == 0 {
                return Err(WasmError::FuelExhausted);
            }
            self.fuel -= 1;

            match code[pc] {
                Instr::Unreachable => return Err(WasmError::Unreachable),
                Instr::Nop => pc += 1,
                Instr::Block { end_pc } => {
                    labels.push(LabelFrame {
                        is_loop: false,
                        start_pc: pc + 1,
                        end_pc,
                        stack_height: stack.len(),
                    });
                    pc += 1;
                }
                Instr::Loop { end_pc } => {
                    labels.push(LabelFrame {
                        is_loop: true,
                        start_pc: pc + 1,
                        end_pc,
                        stack_height: stack.len(),
                    });
                    pc += 1;
                }
                Instr::If {
                    else_or_end_pc,
                    end_pc,
                } => {
                    let cond = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    if cond != 0 {
                        labels.push(LabelFrame {
                            is_loop: false,
                            start_pc: pc + 1,
                            end_pc,
                            stack_height: stack.len(),
                        });
                        pc += 1;
                    } else if else_or_end_pc < end_pc {
                        labels.push(LabelFrame {
                            is_loop: false,
                            start_pc: else_or_end_pc + 1,
                            end_pc,
                            stack_height: stack.len(),
                        });
                        pc = else_or_end_pc + 1;
                    } else {
                        pc = end_pc + 1;
                    }
                }
                Instr::Else { end_pc } => {
                    let _ = labels.pop();
                    pc = end_pc + 1;
                }
                Instr::End => {
                    if labels.pop().is_none() {
                        break;
                    }
                    pc += 1;
                }
                Instr::Br(rel_depth) => {
                    pc = branch_to(&mut labels, &mut stack, rel_depth, code.len())?;
                }
                Instr::BrIf(rel_depth) => {
                    let cond = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    if cond != 0 {
                        pc = branch_to(&mut labels, &mut stack, rel_depth, code.len())?;
                    } else {
                        pc += 1;
                    }
                }
                Instr::Return => break,
                Instr::Call(callee_idx) => {
                    let callee = self
                        .module
                        .funcs
                        .get(callee_idx as usize)
                        .ok_or_else(|| WasmError::Malformed("bad call index".into()))?;
                    let pcount = self.module.types[callee.type_idx as usize].params.len();
                    if stack.len() < pcount {
                        return Err(WasmError::StackError);
                    }
                    let call_args: Vec<WasmVal> = stack.split_off(stack.len() - pcount);
                    let res = self.call_func(callee_idx as usize, &call_args, depth + 1)?;
                    stack.extend(res);
                    pc += 1;
                }
                Instr::Drop => {
                    stack.pop().ok_or(WasmError::StackError)?;
                    pc += 1;
                }
                Instr::Select => {
                    let c = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    let v2 = stack.pop().ok_or(WasmError::StackError)?;
                    let v1 = stack.pop().ok_or(WasmError::StackError)?;
                    stack.push(if c != 0 { v1 } else { v2 });
                    pc += 1;
                }
                Instr::LocalGet(idx) => {
                    let v = *locals.get(idx as usize).ok_or(WasmError::StackError)?;
                    stack.push(v);
                    pc += 1;
                }
                Instr::LocalSet(idx) => {
                    let v = stack.pop().ok_or(WasmError::StackError)?;
                    let slot = locals.get_mut(idx as usize).ok_or(WasmError::StackError)?;
                    *slot = v;
                    pc += 1;
                }
                Instr::LocalTee(idx) => {
                    let v = *stack.last().ok_or(WasmError::StackError)?;
                    let slot = locals.get_mut(idx as usize).ok_or(WasmError::StackError)?;
                    *slot = v;
                    pc += 1;
                }
                Instr::I32Load(offset) => {
                    let base = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let addr = checked_mem_addr(base, offset, 4, self.memory.len())?;
                    let val = i32::from_le_bytes([
                        self.memory[addr],
                        self.memory[addr + 1],
                        self.memory[addr + 2],
                        self.memory[addr + 3],
                    ]);
                    stack.push(WasmVal::I32(val));
                    pc += 1;
                }
                Instr::F32Load(offset) => {
                    let base = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let addr = checked_mem_addr(base, offset, 4, self.memory.len())?;
                    let val = f32::from_le_bytes([
                        self.memory[addr],
                        self.memory[addr + 1],
                        self.memory[addr + 2],
                        self.memory[addr + 3],
                    ]);
                    stack.push(WasmVal::F32(val));
                    pc += 1;
                }
                Instr::I32Load8U(offset) => {
                    let base = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let addr = checked_mem_addr(base, offset, 1, self.memory.len())?;
                    stack.push(WasmVal::I32(i32::from(self.memory[addr])));
                    pc += 1;
                }
                Instr::I32Store(offset) => {
                    let val = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    let base = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let addr = checked_mem_addr(base, offset, 4, self.memory.len())?;
                    self.memory[addr..addr + 4].copy_from_slice(&val.to_le_bytes());
                    pc += 1;
                }
                Instr::F32Store(offset) => {
                    let val = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
                    let base = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let addr = checked_mem_addr(base, offset, 4, self.memory.len())?;
                    self.memory[addr..addr + 4].copy_from_slice(&val.to_le_bytes());
                    pc += 1;
                }
                Instr::I32Store8(offset) => {
                    let val = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    let base = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let addr = checked_mem_addr(base, offset, 1, self.memory.len())?;
                    self.memory[addr] = val as u8;
                    pc += 1;
                }
                Instr::MemorySize => {
                    stack.push(WasmVal::I32((self.memory.len() / WASM_PAGE_SIZE) as i32));
                    pc += 1;
                }
                Instr::MemoryGrow => {
                    let delta = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32 as usize;
                    let cur_pages = self.memory.len() / WASM_PAGE_SIZE;
                    if cur_pages.saturating_add(delta) <= self.module.max_mem_pages {
                        self.memory
                            .resize((cur_pages + delta) * WASM_PAGE_SIZE, 0u8);
                        stack.push(WasmVal::I32(cur_pages as i32));
                    } else {
                        stack.push(WasmVal::I32(-1));
                    }
                    pc += 1;
                }
                Instr::I32Const(v) => {
                    stack.push(WasmVal::I32(v));
                    pc += 1;
                }
                Instr::F32Const(v) => {
                    stack.push(WasmVal::F32(v));
                    pc += 1;
                }
                Instr::I32Eqz => {
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    stack.push(WasmVal::I32(i32::from(a == 0)));
                    pc += 1;
                }
                Instr::I32Eq => bin_i32_cmp(&mut stack, &mut pc, |a, b| a == b)?,
                Instr::I32Ne => bin_i32_cmp(&mut stack, &mut pc, |a, b| a != b)?,
                Instr::I32LtS => bin_i32_cmp(&mut stack, &mut pc, |a, b| a < b)?,
                Instr::I32LtU => bin_i32_cmp(&mut stack, &mut pc, |a, b| (a as u32) < (b as u32))?,
                Instr::I32GtS => bin_i32_cmp(&mut stack, &mut pc, |a, b| a > b)?,
                Instr::I32GtU => bin_i32_cmp(&mut stack, &mut pc, |a, b| (a as u32) > (b as u32))?,
                Instr::I32LeS => bin_i32_cmp(&mut stack, &mut pc, |a, b| a <= b)?,
                Instr::I32LeU => bin_i32_cmp(&mut stack, &mut pc, |a, b| (a as u32) <= (b as u32))?,
                Instr::I32GeS => bin_i32_cmp(&mut stack, &mut pc, |a, b| a >= b)?,
                Instr::I32GeU => bin_i32_cmp(&mut stack, &mut pc, |a, b| (a as u32) >= (b as u32))?,
                Instr::F32Eq => bin_f32_cmp(&mut stack, &mut pc, |a, b| a == b)?,
                Instr::F32Ne => bin_f32_cmp(&mut stack, &mut pc, |a, b| a != b)?,
                Instr::F32Lt => bin_f32_cmp(&mut stack, &mut pc, |a, b| a < b)?,
                Instr::F32Gt => bin_f32_cmp(&mut stack, &mut pc, |a, b| a > b)?,
                Instr::F32Le => bin_f32_cmp(&mut stack, &mut pc, |a, b| a <= b)?,
                Instr::F32Ge => bin_f32_cmp(&mut stack, &mut pc, |a, b| a >= b)?,
                Instr::I32Add => bin_i32(&mut stack, &mut pc, i32::wrapping_add)?,
                Instr::I32Sub => bin_i32(&mut stack, &mut pc, i32::wrapping_sub)?,
                Instr::I32Mul => bin_i32(&mut stack, &mut pc, i32::wrapping_mul)?,
                Instr::I32DivS => {
                    let b = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    if b == 0 || (a == i32::MIN && b == -1) {
                        return Err(WasmError::DivisionByZero);
                    }
                    stack.push(WasmVal::I32(a / b));
                    pc += 1;
                }
                Instr::I32DivU => {
                    let b = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    if b == 0 {
                        return Err(WasmError::DivisionByZero);
                    }
                    stack.push(WasmVal::I32((a / b) as i32));
                    pc += 1;
                }
                Instr::I32RemS => {
                    let b = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    if b == 0 {
                        return Err(WasmError::DivisionByZero);
                    }
                    stack.push(WasmVal::I32(a.wrapping_rem(b)));
                    pc += 1;
                }
                Instr::I32RemU => {
                    let b = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    if b == 0 {
                        return Err(WasmError::DivisionByZero);
                    }
                    stack.push(WasmVal::I32((a % b) as i32));
                    pc += 1;
                }
                Instr::I32And => bin_i32(&mut stack, &mut pc, |a, b| a & b)?,
                Instr::I32Or => bin_i32(&mut stack, &mut pc, |a, b| a | b)?,
                Instr::I32Xor => bin_i32(&mut stack, &mut pc, |a, b| a ^ b)?,
                Instr::I32Shl => bin_i32(&mut stack, &mut pc, |a, b| a.wrapping_shl(b as u32))?,
                Instr::I32ShrS => bin_i32(&mut stack, &mut pc, |a, b| a.wrapping_shr(b as u32))?,
                Instr::I32ShrU => bin_i32(&mut stack, &mut pc, |a, b| {
                    ((a as u32).wrapping_shr(b as u32)) as i32
                })?,
                Instr::F32Abs => un_f32(&mut stack, &mut pc, f32::abs)?,
                Instr::F32Neg => un_f32(&mut stack, &mut pc, |v| -v)?,
                Instr::F32Ceil => un_f32(&mut stack, &mut pc, f32::ceil)?,
                Instr::F32Floor => un_f32(&mut stack, &mut pc, f32::floor)?,
                Instr::F32Trunc => un_f32(&mut stack, &mut pc, f32::trunc)?,
                Instr::F32Nearest => un_f32(&mut stack, &mut pc, f32::round_ties_even)?,
                Instr::F32Sqrt => un_f32(&mut stack, &mut pc, f32::sqrt)?,
                Instr::F32Add => bin_f32(&mut stack, &mut pc, |a, b| a + b)?,
                Instr::F32Sub => bin_f32(&mut stack, &mut pc, |a, b| a - b)?,
                Instr::F32Mul => bin_f32(&mut stack, &mut pc, |a, b| a * b)?,
                Instr::F32Div => bin_f32(&mut stack, &mut pc, |a, b| a / b)?,
                Instr::F32Min => bin_f32(&mut stack, &mut pc, f32::min)?,
                Instr::F32Max => bin_f32(&mut stack, &mut pc, f32::max)?,
                Instr::I32TruncF32S => {
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
                    stack.push(WasmVal::I32(a as i32));
                    pc += 1;
                }
                Instr::F32ConvertI32S => {
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    stack.push(WasmVal::F32(a as f32));
                    pc += 1;
                }
                Instr::F32ConvertI32U => {
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()? as u32;
                    stack.push(WasmVal::F32(a as f32));
                    pc += 1;
                }
                Instr::I32ReinterpretF32 => {
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
                    stack.push(WasmVal::I32(a.to_bits() as i32));
                    pc += 1;
                }
                Instr::F32ReinterpretI32 => {
                    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
                    stack.push(WasmVal::F32(f32::from_bits(a as u32)));
                    pc += 1;
                }
            }
        }

        let rcount = ftype.results.len();
        if stack.len() < rcount {
            return Err(WasmError::StackError);
        }
        Ok(stack.split_off(stack.len() - rcount))
    }
}

fn branch_to(
    labels: &mut Vec<LabelFrame>,
    stack: &mut Vec<WasmVal>,
    rel_depth: u32,
    code_len: usize,
) -> Result<usize, WasmError> {
    let d = rel_depth as usize;
    if d >= labels.len() {
        return Ok(code_len);
    }
    let target_idx = labels.len() - 1 - d;
    let frame = labels[target_idx];
    if frame.is_loop {
        labels.truncate(target_idx + 1);
        stack.truncate(frame.stack_height);
        Ok(frame.start_pc)
    } else {
        labels.truncate(target_idx);
        stack.truncate(frame.stack_height);
        Ok(frame.end_pc + 1)
    }
}

fn checked_mem_addr(
    base: u32,
    offset: u32,
    size: usize,
    mem_len: usize,
) -> Result<usize, WasmError> {
    let addr = (base as usize)
        .checked_add(offset as usize)
        .ok_or(WasmError::MemoryOutOfBounds)?;
    let end = addr.checked_add(size).ok_or(WasmError::MemoryOutOfBounds)?;
    if end > mem_len {
        return Err(WasmError::MemoryOutOfBounds);
    }
    Ok(addr)
}

fn bin_i32(
    stack: &mut Vec<WasmVal>,
    pc: &mut usize,
    op: fn(i32, i32) -> i32,
) -> Result<(), WasmError> {
    let b = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
    stack.push(WasmVal::I32(op(a, b)));
    *pc += 1;
    Ok(())
}

fn bin_i32_cmp(
    stack: &mut Vec<WasmVal>,
    pc: &mut usize,
    op: fn(i32, i32) -> bool,
) -> Result<(), WasmError> {
    let b = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
    let a = stack.pop().ok_or(WasmError::StackError)?.as_i32()?;
    stack.push(WasmVal::I32(i32::from(op(a, b))));
    *pc += 1;
    Ok(())
}

fn bin_f32(
    stack: &mut Vec<WasmVal>,
    pc: &mut usize,
    op: fn(f32, f32) -> f32,
) -> Result<(), WasmError> {
    let b = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
    let a = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
    stack.push(WasmVal::F32(op(a, b)));
    *pc += 1;
    Ok(())
}

fn bin_f32_cmp(
    stack: &mut Vec<WasmVal>,
    pc: &mut usize,
    op: fn(f32, f32) -> bool,
) -> Result<(), WasmError> {
    let b = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
    let a = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
    stack.push(WasmVal::I32(i32::from(op(a, b))));
    *pc += 1;
    Ok(())
}

fn un_f32(stack: &mut Vec<WasmVal>, pc: &mut usize, op: fn(f32) -> f32) -> Result<(), WasmError> {
    let a = stack.pop().ok_or(WasmError::StackError)?.as_f32()?;
    stack.push(WasmVal::F32(op(a)));
    *pc += 1;
    Ok(())
}

/// v0.7 #9: サンドボックス実行される WebAssembly 映像エフェクトプラグイン
pub struct WasmEffectPlugin {
    module: WasmModule,
    manifest: PluginManifest,
    param_count: usize,
}

impl WasmEffectPlugin {
    pub fn from_bytes(
        id: impl Into<String>,
        name: impl Into<String>,
        bytes: &[u8],
        custom_manifest: Option<PluginManifest>,
    ) -> Result<Self, WasmError> {
        let module = WasmModule::parse(bytes)?;
        if !module.has_exported_memory {
            return Err(WasmError::MissingExport("memory".into()));
        }
        let param_count = module
            .export_param_count("process_rgba")
            .ok_or_else(|| WasmError::MissingExport("process_rgba".into()))?;
        if param_count != 4 && param_count != 6 {
            return Err(WasmError::Malformed(format!(
                "process_rgba must accept 4 or 6 params, got {param_count}"
            )));
        }

        // 任意の `ve_api_version() -> i32` エクスポートがあればバージョン検証
        if module.exports_func.contains_key("ve_api_version") {
            let mut inst = module.instantiate()?;
            let res = inst.call_export("ve_api_version", &[])?;
            let ver = res
                .first()
                .copied()
                .ok_or(WasmError::StackError)?
                .as_i32()?;
            if ver != PLUGIN_API_VERSION as i32 {
                return Err(WasmError::IncompatibleApiVersion {
                    found: ver,
                    expected: PLUGIN_API_VERSION as i32,
                });
            }
        }

        let manifest = custom_manifest.unwrap_or_else(|| PluginManifest {
            api_version: PLUGIN_API_VERSION,
            id: id.into(),
            name: name.into(),
            version: "1.0.0".into(),
            author: "WASM Plugin".into(),
            description: "Sandboxed WebAssembly (.wasm) video effect plugin".into(),
            capabilities: vec![PluginCapability::VideoEffect],
            params: vec![PluginParamDef {
                key: "strength".into(),
                label: "Strength".into(),
                default_value: 1.0,
                min_value: 0.0,
                max_value: 2.0,
            }],
        });

        Ok(Self {
            module,
            manifest,
            param_count,
        })
    }

    pub fn from_file(path: &Path) -> Result<Self, WasmError> {
        let bytes = std::fs::read(path)?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin");
        let sanitized: String = stem
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        let id = format!(
            "wasm.{}",
            if sanitized.is_empty() {
                "plugin"
            } else {
                &sanitized
            }
        );

        let sidecar = path.with_extension("json");
        let custom_manifest = if sidecar.exists() {
            std::fs::read_to_string(&sidecar)
                .ok()
                .and_then(|s| serde_json::from_str::<PluginManifest>(&s).ok())
        } else {
            None
        };

        Self::from_bytes(id, format!("WASM: {stem}"), &bytes, custom_manifest)
    }
}

impl EffectPlugin for WasmEffectPlugin {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }

    fn process_linear_premul_rgba(
        &self,
        pixels: &mut [[f32; 4]],
        width: u32,
        height: u32,
        time_sec: f64,
        params: &BTreeMap<String, f32>,
    ) {
        if pixels.is_empty() {
            return;
        }
        let strength = params
            .get("strength")
            .copied()
            .unwrap_or(1.0)
            .clamp(0.0, 2.0);
        let Ok(mut inst) = self.module.instantiate() else {
            return;
        };

        // 1ページ (64 KiB) に収まるチャンク単位でサンドボックス実行（4K解像度でも低メモリ＆安全）
        let needed_bytes = PIXEL_BUFFER_PTR + PIXEL_CHUNK_LEN * 16;
        if inst.memory.len() < needed_bytes {
            let pages = needed_bytes.div_ceil(WASM_PAGE_SIZE).min(MAX_MEMORY_PAGES);
            inst.memory.resize(pages * WASM_PAGE_SIZE, 0);
        }

        for chunk in pixels.chunks_mut(PIXEL_CHUNK_LEN) {
            let count = chunk.len();
            let byte_len = count * 16;
            if PIXEL_BUFFER_PTR + byte_len > inst.memory.len() {
                break;
            }
            for (i, px) in chunk.iter().enumerate() {
                let off = PIXEL_BUFFER_PTR + i * 16;
                inst.memory[off..off + 4].copy_from_slice(&px[0].to_le_bytes());
                inst.memory[off + 4..off + 8].copy_from_slice(&px[1].to_le_bytes());
                inst.memory[off + 8..off + 12].copy_from_slice(&px[2].to_le_bytes());
                inst.memory[off + 12..off + 16].copy_from_slice(&px[3].to_le_bytes());
            }

            inst.fuel = DEFAULT_FUEL_PER_CALL;
            let call_res = if self.param_count == 6 {
                inst.call_export(
                    "process_rgba",
                    &[
                        WasmVal::I32(PIXEL_BUFFER_PTR as i32),
                        WasmVal::I32(count as i32),
                        WasmVal::I32(width as i32),
                        WasmVal::I32(height as i32),
                        WasmVal::F32(time_sec as f32),
                        WasmVal::F32(strength),
                    ],
                )
            } else {
                inst.call_export(
                    "process_rgba",
                    &[
                        WasmVal::I32(PIXEL_BUFFER_PTR as i32),
                        WasmVal::I32(count as i32),
                        WasmVal::F32(time_sec as f32),
                        WasmVal::F32(strength),
                    ],
                )
            };

            // トラップ時は元ピクセルを保持して安全に中断（ホストを絶対にクラッシュさせない）
            if call_res.is_err() {
                break;
            }

            for (i, px) in chunk.iter_mut().enumerate() {
                let off = PIXEL_BUFFER_PTR + i * 16;
                let read_f32 = |o: usize| {
                    let v = f32::from_le_bytes([
                        inst.memory[o],
                        inst.memory[o + 1],
                        inst.memory[o + 2],
                        inst.memory[o + 3],
                    ]);
                    if v.is_finite() {
                        v.clamp(0.0, 4.0)
                    } else {
                        0.0
                    }
                };
                px[0] = read_f32(off);
                px[1] = read_f32(off + 4);
                px[2] = read_f32(off + 8);
                px[3] = read_f32(off + 12).clamp(0.0, 1.0);
            }
        }
    }
}

/// ホットリロード監視対象 WASM プラグインの状態
#[derive(Debug, Clone)]
pub struct WasmPluginStatus {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub last_error: Option<String>,
}

struct TrackedWasm {
    id: String,
    name: String,
    modified: Option<SystemTime>,
    file_len: u64,
    last_error: Option<String>,
}

/// v0.7 #9: `plugins/*.wasm` 動的ロード＆ホットリロード（ロールバック保護付き）マネージャー
#[derive(Default)]
pub struct WasmPluginManager {
    plugin_dir: Option<PathBuf>,
    tracked: BTreeMap<PathBuf, TrackedWasm>,
}

impl WasmPluginManager {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            plugin_dir: Some(dir.into()),
            tracked: BTreeMap::new(),
        }
    }

    pub fn plugin_dir(&self) -> Option<&Path> {
        self.plugin_dir.as_deref()
    }

    pub fn set_plugin_dir(&mut self, dir: PathBuf) {
        self.plugin_dir = Some(dir);
    }

    /// `plugins/*.wasm` を走査し、新規・更新された WASM プラグインをホットリロードして `PluginRegistry` へ同期する。
    /// 破損した `.wasm` に差し替えられた場合はロールバックして直前の正常プラグインを保持する。
    pub fn poll_and_sync(&mut self, registry: &mut PluginRegistry) -> Vec<String> {
        let mut events = Vec::new();
        let Some(dir) = self.plugin_dir.clone() else {
            return events;
        };
        if !dir.exists() {
            return events;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return events;
        };

        let mut seen = HashSet::new();
        for entry in rd.flatten() {
            let path = entry.path();
            let is_wasm = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("wasm"));
            if !is_wasm {
                continue;
            }
            seen.insert(path.clone());
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta.modified().ok();
            let file_len = meta.len();

            let unchanged = self
                .tracked
                .get(&path)
                .is_some_and(|t| t.modified == modified && t.file_len == file_len);
            if unchanged {
                continue;
            }

            match WasmEffectPlugin::from_file(&path) {
                Ok(plugin) => {
                    let m = plugin.manifest();
                    match registry.register_or_replace_effect(Arc::new(plugin)) {
                        Ok(()) => {
                            let msg = format!("WASM 読込完了: {} ({})", m.name, m.id);
                            events.push(msg);
                            self.tracked.insert(
                                path,
                                TrackedWasm {
                                    id: m.id,
                                    name: m.name,
                                    modified,
                                    file_len,
                                    last_error: None,
                                },
                            );
                        }
                        Err(e) => {
                            let err_s = e.to_string();
                            events.push(format!("WASM 登録失敗 ({}): {err_s}", path.display()));
                            self.record_error(path, modified, file_len, err_s);
                        }
                    }
                }
                Err(e) => {
                    let err_s = e.to_string();
                    events.push(format!(
                        "WASM 検証失敗・直前版を保持 ({}): {err_s}",
                        path.display()
                    ));
                    self.record_error(path, modified, file_len, err_s);
                }
            }
        }

        // 削除された .wasm ファイルをレジストリから除去
        let removed_paths: Vec<PathBuf> = self
            .tracked
            .keys()
            .filter(|p| !seen.contains(*p))
            .cloned()
            .collect();
        for rp in removed_paths {
            if let Some(old) = self.tracked.remove(&rp) {
                if !old.id.is_empty() {
                    registry.unregister_effect(&old.id);
                }
                events.push(format!("WASM アンロード: {}", old.id));
            }
        }

        events
    }

    fn record_error(
        &mut self,
        path: PathBuf,
        modified: Option<SystemTime>,
        file_len: u64,
        err: String,
    ) {
        self.tracked
            .entry(path)
            .and_modify(|t| {
                t.modified = modified;
                t.file_len = file_len;
                t.last_error = Some(err.clone());
            })
            .or_insert(TrackedWasm {
                id: String::new(),
                name: String::new(),
                modified,
                file_len,
                last_error: Some(err),
            });
    }

    pub fn list_status(&self) -> Vec<WasmPluginStatus> {
        self.tracked
            .iter()
            .map(|(path, t)| WasmPluginStatus {
                id: t.id.clone(),
                name: t.name.clone(),
                path: path.clone(),
                last_error: t.last_error.clone(),
            })
            .collect()
    }
}

/// 実行可能なサンプル `.wasm` プラグイン（色反転・ウォームティント・モノクロ）を指定フォルダへ生成する
pub fn emit_sample_wasm_plugins(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    std::fs::create_dir_all(dir)?;
    let samples = [
        ("wasm_invert.wasm", build_sample_wasm_invert()),
        ("wasm_warm_tint.wasm", build_sample_wasm_warm_tint()),
        ("wasm_grayscale.wasm", build_sample_wasm_grayscale()),
    ];
    let mut created = Vec::new();
    for (fname, bytes) in samples {
        let p = dir.join(fname);
        std::fs::write(&p, bytes)?;
        created.push(p);
    }
    Ok(created)
}

/// 正規の WebAssembly 1.0 バイナリ生成: `process_rgba(ptr, count, time, strength)` で Premultiplied RGB を色反転する
pub fn build_sample_wasm_invert() -> Vec<u8> {
    // locals: 0=ptr(i32), 1=count(i32), 2=time(f32), 3=strength(f32), 4=i(i32), 5=off(i32), 6=a(f32)
    let mut body = Vec::new();
    // 2 local groups: 2 i32 (i, off), 1 f32 (a)
    write_leb_u32(&mut body, 2);
    write_leb_u32(&mut body, 2);
    body.push(0x7F); // i32
    write_leb_u32(&mut body, 1);
    body.push(0x7D); // f32

    // if count <= 0 return
    body.extend([0x20, 0x01, 0x41, 0x00, 0x4C, 0x04, 0x40, 0x0F, 0x0B]);
    // loop
    body.extend([0x03, 0x40]);
    // off = ptr + (i << 4)
    body.extend([0x20, 0x00, 0x20, 0x04, 0x41, 0x04, 0x74, 0x6A, 0x21, 0x05]);
    // a = f32.load(off + 12)
    body.extend([0x20, 0x05, 0x2A, 0x02, 0x0C, 0x21, 0x06]);
    // for ch offset in [0, 4, 8]: c' = c + ((a - c) - c) * strength = c + (a - 2*c)*strength
    for ch_off in [0u8, 4u8, 8u8] {
        // addr
        body.extend([0x20, 0x05]);
        // c = load(off + ch_off)
        body.extend([0x20, 0x05, 0x2A, 0x02, ch_off]);
        // (a - c) - c
        body.extend([0x20, 0x06]);
        body.extend([0x20, 0x05, 0x2A, 0x02, ch_off]);
        body.push(0x93); // f32.sub -> (a - c)
        body.extend([0x20, 0x05, 0x2A, 0x02, ch_off]);
        body.push(0x93); // f32.sub -> (a - c) - c
        // * strength
        body.extend([0x20, 0x03, 0x94]);
        // + c
        body.push(0x92);
        // f32.store(off + ch_off)
        body.extend([0x38, 0x02, ch_off]);
    }
    // i = i + 1; br_if 0 (i < count)
    body.extend([
        0x20, 0x04, 0x41, 0x01, 0x6A, 0x22, 0x04, 0x20, 0x01, 0x49, 0x0D, 0x00,
    ]);
    // end loop, end func
    body.extend([0x0B, 0x0B]);

    assemble_effect_wasm(&body)
}

/// 正規の WebAssembly 1.0 バイナリ生成: `process_rgba` で暖色ティント（R強調・B減衰）を適用する
pub fn build_sample_wasm_warm_tint() -> Vec<u8> {
    let mut body = Vec::new();
    write_leb_u32(&mut body, 1);
    write_leb_u32(&mut body, 2);
    body.push(0x7F); // i(4), off(5)

    // if count <= 0 return
    body.extend([0x20, 0x01, 0x41, 0x00, 0x4C, 0x04, 0x40, 0x0F, 0x0B]);
    // loop
    body.extend([0x03, 0x40]);
    // off = ptr + (i << 4)
    body.extend([0x20, 0x00, 0x20, 0x04, 0x41, 0x04, 0x74, 0x6A, 0x21, 0x05]);

    // R = R * (1.0 + 0.35 * strength)
    body.extend([0x20, 0x05, 0x20, 0x05, 0x2A, 0x02, 0x00]);
    body.push(0x43);
    body.extend(1.0f32.to_le_bytes());
    body.push(0x43);
    body.extend(0.35f32.to_le_bytes());
    body.extend([0x20, 0x03, 0x94, 0x92, 0x94, 0x38, 0x02, 0x00]);

    // B = B * max(0.0, 1.0 - 0.35 * strength)
    body.extend([0x20, 0x05, 0x20, 0x05, 0x2A, 0x02, 0x08]);
    body.push(0x43);
    body.extend(0.0f32.to_le_bytes());
    body.push(0x43);
    body.extend(1.0f32.to_le_bytes());
    body.push(0x43);
    body.extend(0.35f32.to_le_bytes());
    body.extend([0x20, 0x03, 0x94, 0x93, 0x97, 0x94, 0x38, 0x02, 0x08]);

    // i = i + 1; br_if 0 (i < count)
    body.extend([
        0x20, 0x04, 0x41, 0x01, 0x6A, 0x22, 0x04, 0x20, 0x01, 0x49, 0x0D, 0x00,
    ]);
    body.extend([0x0B, 0x0B]);

    assemble_effect_wasm(&body)
}

/// 正規の WebAssembly 1.0 バイナリ生成: `process_rgba` で線形輝度モノクロ（グレースケール）化
pub fn build_sample_wasm_grayscale() -> Vec<u8> {
    let mut body = Vec::new();
    write_leb_u32(&mut body, 2);
    write_leb_u32(&mut body, 2);
    body.push(0x7F); // i(4), off(5)
    write_leb_u32(&mut body, 1);
    body.push(0x7D); // luma(6)

    // if count <= 0 return
    body.extend([0x20, 0x01, 0x41, 0x00, 0x4C, 0x04, 0x40, 0x0F, 0x0B]);
    // loop
    body.extend([0x03, 0x40]);
    // off = ptr + (i << 4)
    body.extend([0x20, 0x00, 0x20, 0x04, 0x41, 0x04, 0x74, 0x6A, 0x21, 0x05]);
    // luma = 0.2126*R + 0.7152*G + 0.0722*B
    body.extend([0x20, 0x05, 0x2A, 0x02, 0x00, 0x43]);
    body.extend(0.2126f32.to_le_bytes());
    body.push(0x94);
    body.extend([0x20, 0x05, 0x2A, 0x02, 0x04, 0x43]);
    body.extend(0.7152f32.to_le_bytes());
    body.extend([0x94, 0x92]);
    body.extend([0x20, 0x05, 0x2A, 0x02, 0x08, 0x43]);
    body.extend(0.0722f32.to_le_bytes());
    body.extend([0x94, 0x92, 0x21, 0x06]);

    for ch_off in [0u8, 4u8, 8u8] {
        // c' = c + (luma - c) * strength
        body.extend([0x20, 0x05]);
        body.extend([0x20, 0x05, 0x2A, 0x02, ch_off]);
        body.extend([0x20, 0x06, 0x20, 0x05, 0x2A, 0x02, ch_off, 0x93]);
        body.extend([0x20, 0x03, 0x94, 0x92, 0x38, 0x02, ch_off]);
    }
    body.extend([
        0x20, 0x04, 0x41, 0x01, 0x6A, 0x22, 0x04, 0x20, 0x01, 0x49, 0x0D, 0x00,
    ]);
    body.extend([0x0B, 0x0B]);

    assemble_effect_wasm(&body)
}

fn assemble_effect_wasm(process_body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&WASM_MAGIC);
    out.extend_from_slice(&WASM_VERSION);

    // Section 1: Type (2 types: [0] () -> i32, [1] (i32, i32, f32, f32) -> ())
    let mut sec1 = Vec::new();
    write_leb_u32(&mut sec1, 2);
    sec1.extend([0x60, 0x00, 0x01, 0x7F]);
    sec1.extend([0x60, 0x04, 0x7F, 0x7F, 0x7D, 0x7D, 0x00]);
    write_section(&mut out, 1, &sec1);

    // Section 3: Function (func 0 -> type 0, func 1 -> type 1)
    write_section(&mut out, 3, &[2, 0, 1]);

    // Section 5: Memory (min 1 page, max 4 pages)
    write_section(&mut out, 5, &[1, 0x01, 1, 4]);

    // Section 7: Export ("memory", "ve_api_version", "process_rgba")
    let mut sec7 = Vec::new();
    write_leb_u32(&mut sec7, 3);
    write_export(&mut sec7, "memory", 0x02, 0);
    write_export(&mut sec7, "ve_api_version", 0x00, 0);
    write_export(&mut sec7, "process_rgba", 0x00, 1);
    write_section(&mut out, 7, &sec7);

    // Section 10: Code (func 0: i32.const 1; end, func 1: process_body)
    let mut sec10 = Vec::new();
    write_leb_u32(&mut sec10, 2);
    let ver_body = [0x00, 0x41, PLUGIN_API_VERSION as u8, 0x0B];
    write_leb_u32(&mut sec10, ver_body.len() as u32);
    sec10.extend_from_slice(&ver_body);
    write_leb_u32(&mut sec10, process_body.len() as u32);
    sec10.extend_from_slice(process_body);
    write_section(&mut out, 10, &sec10);

    out
}

fn write_section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    write_leb_u32(out, payload.len() as u32);
    out.extend_from_slice(payload);
}

fn write_export(out: &mut Vec<u8>, name: &str, kind: u8, idx: u32) {
    write_leb_u32(out, name.len() as u32);
    out.extend_from_slice(name.as_bytes());
    out.push(kind);
    write_leb_u32(out, idx);
}

fn write_leb_u32(out: &mut Vec<u8>, mut val: u32) {
    loop {
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        if val != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if val == 0 {
            break;
        }
    }
}

fn decode_val_type(b: u8) -> Result<ValType, WasmError> {
    match b {
        0x7F => Ok(ValType::I32),
        0x7E => Ok(ValType::I64),
        0x7D => Ok(ValType::F32),
        0x7C => Ok(ValType::F64),
        other => Err(WasmError::Malformed(format!(
            "unknown valtype 0x{other:02X}"
        ))),
    }
}

fn decode_instructions(r: &mut BinaryReader<'_>) -> Result<Vec<Instr>, WasmError> {
    let mut code = Vec::new();
    // Structured control block stack: (open_pc, is_if, else_pc)
    let mut block_stack: Vec<(usize, bool, Option<usize>)> = Vec::new();

    while !r.is_eof() {
        let op = r.read_u8()?;
        let instr = match op {
            0x00 => Instr::Unreachable,
            0x01 => Instr::Nop,
            0x02 => {
                let _bt = r.read_u8()?;
                let open_pc = code.len();
                block_stack.push((open_pc, false, None));
                Instr::Block { end_pc: 0 }
            }
            0x03 => {
                let _bt = r.read_u8()?;
                let open_pc = code.len();
                block_stack.push((open_pc, false, None));
                Instr::Loop { end_pc: 0 }
            }
            0x04 => {
                let _bt = r.read_u8()?;
                let open_pc = code.len();
                block_stack.push((open_pc, true, None));
                Instr::If {
                    else_or_end_pc: 0,
                    end_pc: 0,
                }
            }
            0x05 => {
                let else_pc = code.len();
                if let Some(top) = block_stack.last_mut()
                    && top.1
                {
                    top.2 = Some(else_pc);
                } else {
                    return Err(WasmError::Malformed("else without matching if".into()));
                }
                Instr::Else { end_pc: 0 }
            }
            0x0B => {
                let end_pc = code.len();
                if let Some((open_pc, is_if, else_pc)) = block_stack.pop() {
                    if is_if {
                        code[open_pc] = Instr::If {
                            else_or_end_pc: else_pc.unwrap_or(end_pc),
                            end_pc,
                        };
                        if let Some(ep) = else_pc {
                            code[ep] = Instr::Else { end_pc };
                        }
                    } else {
                        match &mut code[open_pc] {
                            Instr::Block { end_pc: ep } | Instr::Loop { end_pc: ep } => {
                                *ep = end_pc;
                            }
                            _ => {}
                        }
                    }
                }
                Instr::End
            }
            0x0C => Instr::Br(r.read_leb_u32()?),
            0x0D => Instr::BrIf(r.read_leb_u32()?),
            0x0F => Instr::Return,
            0x10 => Instr::Call(r.read_leb_u32()?),
            0x1A => Instr::Drop,
            0x1B => Instr::Select,
            0x20 => Instr::LocalGet(r.read_leb_u32()?),
            0x21 => Instr::LocalSet(r.read_leb_u32()?),
            0x22 => Instr::LocalTee(r.read_leb_u32()?),
            0x28 => {
                let _align = r.read_leb_u32()?;
                Instr::I32Load(r.read_leb_u32()?)
            }
            0x2A => {
                let _align = r.read_leb_u32()?;
                Instr::F32Load(r.read_leb_u32()?)
            }
            0x2D => {
                let _align = r.read_leb_u32()?;
                Instr::I32Load8U(r.read_leb_u32()?)
            }
            0x36 => {
                let _align = r.read_leb_u32()?;
                Instr::I32Store(r.read_leb_u32()?)
            }
            0x38 => {
                let _align = r.read_leb_u32()?;
                Instr::F32Store(r.read_leb_u32()?)
            }
            0x3A => {
                let _align = r.read_leb_u32()?;
                Instr::I32Store8(r.read_leb_u32()?)
            }
            0x3F => {
                let _zero = r.read_u8()?;
                Instr::MemorySize
            }
            0x40 => {
                let _zero = r.read_u8()?;
                Instr::MemoryGrow
            }
            0x41 => Instr::I32Const(r.read_leb_i32()?),
            0x43 => {
                let b = r.read_bytes(4)?;
                Instr::F32Const(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            0x45 => Instr::I32Eqz,
            0x46 => Instr::I32Eq,
            0x47 => Instr::I32Ne,
            0x48 => Instr::I32LtS,
            0x49 => Instr::I32LtU,
            0x4A => Instr::I32GtS,
            0x4B => Instr::I32GtU,
            0x4C => Instr::I32LeS,
            0x4D => Instr::I32LeU,
            0x4E => Instr::I32GeS,
            0x4F => Instr::I32GeU,
            0x5B => Instr::F32Eq,
            0x5C => Instr::F32Ne,
            0x5D => Instr::F32Lt,
            0x5E => Instr::F32Gt,
            0x5F => Instr::F32Le,
            0x60 => Instr::F32Ge,
            0x6A => Instr::I32Add,
            0x6B => Instr::I32Sub,
            0x6C => Instr::I32Mul,
            0x6D => Instr::I32DivS,
            0x6E => Instr::I32DivU,
            0x6F => Instr::I32RemS,
            0x70 => Instr::I32RemU,
            0x71 => Instr::I32And,
            0x72 => Instr::I32Or,
            0x73 => Instr::I32Xor,
            0x74 => Instr::I32Shl,
            0x75 => Instr::I32ShrS,
            0x76 => Instr::I32ShrU,
            0x8B => Instr::F32Abs,
            0x8C => Instr::F32Neg,
            0x8D => Instr::F32Ceil,
            0x8E => Instr::F32Floor,
            0x8F => Instr::F32Trunc,
            0x90 => Instr::F32Nearest,
            0x91 => Instr::F32Sqrt,
            0x92 => Instr::F32Add,
            0x93 => Instr::F32Sub,
            0x94 => Instr::F32Mul,
            0x95 => Instr::F32Div,
            0x96 => Instr::F32Min,
            0x97 => Instr::F32Max,
            0xA8 => Instr::I32TruncF32S,
            0xB2 => Instr::F32ConvertI32S,
            0xB3 => Instr::F32ConvertI32U,
            0xBC => Instr::I32ReinterpretF32,
            0xBE => Instr::F32ReinterpretI32,
            other => return Err(WasmError::UnsupportedOpcode(other)),
        };
        code.push(instr);
    }

    if !block_stack.is_empty() {
        return Err(WasmError::Malformed(
            "unclosed structured control block".into(),
        ));
    }
    Ok(code)
}

struct BinaryReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_u8(&mut self) -> Result<u8, WasmError> {
        if self.pos >= self.data.len() {
            return Err(WasmError::UnexpectedEof);
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], WasmError> {
        let end = self.pos.checked_add(len).ok_or(WasmError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(WasmError::UnexpectedEof);
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn read_leb_u32(&mut self) -> Result<u32, WasmError> {
        let mut result = 0u32;
        let mut shift = 0u32;
        for _ in 0..5 {
            let byte = self.read_u8()?;
            result |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
        Err(WasmError::Malformed("u32 LEB128 overflow".into()))
    }

    fn read_leb_i32(&mut self) -> Result<i32, WasmError> {
        let mut result = 0i32;
        let mut shift = 0u32;
        for _ in 0..5 {
            let byte = self.read_u8()?;
            result |= i32::from(byte & 0x7F) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 32 && (byte & 0x40) != 0 {
                    result |= !0i32 << shift;
                }
                return Ok(result);
            }
        }
        Err(WasmError::Malformed("i32 LEB128 overflow".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_vm_executes_real_wasm_binary_invert_and_tint_effects() {
        let invert_bytes = build_sample_wasm_invert();
        let plugin =
            WasmEffectPlugin::from_bytes("wasm.invert", "WASM Invert", &invert_bytes, None)
                .unwrap();
        let mut reg = PluginRegistry::with_builtins();
        reg.register_effect(Arc::new(plugin)).unwrap();

        let mut pixels = vec![[0.2, 0.7, 0.9, 1.0], [0.4, 0.1, 0.0, 0.5]];
        let mut params = BTreeMap::new();
        params.insert("strength".into(), 1.0);
        reg.apply_effect("wasm.invert", &mut pixels, 2, 1, 0.0, &params)
            .unwrap();

        assert!((pixels[0][0] - 0.8).abs() < 1e-4, "px0={:?}", pixels[0]);
        assert!((pixels[0][1] - 0.3).abs() < 1e-4, "px0={:?}", pixels[0]);
        assert!((pixels[0][2] - 0.1).abs() < 1e-4, "px0={:?}", pixels[0]);
        // Premultiplied alpha=0.5: inverted r = 0.5 - 0.4 = 0.1
        assert!((pixels[1][0] - 0.1).abs() < 1e-4, "px1={:?}", pixels[1]);
        assert!((pixels[1][3] - 0.5).abs() < 1e-4, "px1={:?}", pixels[1]);

        let warm_bytes = build_sample_wasm_warm_tint();
        let warm =
            WasmEffectPlugin::from_bytes("wasm.warm", "WASM Warm", &warm_bytes, None).unwrap();
        let mut wpx = vec![[0.5, 0.5, 0.5, 1.0]];
        warm.process_linear_premul_rgba(&mut wpx, 1, 1, 0.0, &params);
        assert!(wpx[0][0] > 0.65 && wpx[0][2] < 0.35, "wpx={:?}", wpx[0]);
    }

    #[test]
    fn wasm_vm_traps_safely_on_infinite_loop_and_oob_memory() {
        // Infinite loop: (loop (br 0))
        let loop_body = [0x00, 0x03, 0x40, 0x0C, 0x00, 0x0B, 0x0B];
        let loop_wasm = assemble_effect_wasm(&loop_body);
        let module = WasmModule::parse(&loop_wasm).unwrap();
        let mut inst = module.instantiate().unwrap();
        inst.fuel = 500;
        let err = inst
            .call_export(
                "process_rgba",
                &[
                    WasmVal::I32(4096),
                    WasmVal::I32(1),
                    WasmVal::F32(0.0),
                    WasmVal::F32(1.0),
                ],
            )
            .unwrap_err();
        assert!(matches!(err, WasmError::FuelExhausted));

        // OOB memory store at 0x70000000
        let mut oob_body = vec![0x00, 0x41];
        write_leb_u32(&mut oob_body, 0x0FFF_FFFF);
        oob_body.extend([0x43, 0, 0, 0, 0, 0x38, 0x02, 0x00, 0x0B]);
        let oob_wasm = assemble_effect_wasm(&oob_body);
        let oob_mod = WasmModule::parse(&oob_wasm).unwrap();
        let mut oob_inst = oob_mod.instantiate().unwrap();
        let oob_err = oob_inst
            .call_export(
                "process_rgba",
                &[
                    WasmVal::I32(4096),
                    WasmVal::I32(1),
                    WasmVal::F32(0.0),
                    WasmVal::F32(1.0),
                ],
            )
            .unwrap_err();
        assert!(matches!(oob_err, WasmError::MemoryOutOfBounds));
    }

    #[test]
    fn wasm_hot_reloader_loads_reloads_and_rolls_back_on_corrupt_wasm() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reg = PluginRegistry::with_builtins();
        let mut mgr = WasmPluginManager::new(tmp.path());

        emit_sample_wasm_plugins(tmp.path()).unwrap();
        let ev1 = mgr.poll_and_sync(&mut reg);
        assert_eq!(ev1.len(), 3, "ev1={ev1:?}");
        assert!(reg.get_effect("wasm.wasm_invert").is_some());
        assert!(reg.get_effect("wasm.wasm_warm_tint").is_some());
        assert!(reg.get_effect("wasm.wasm_grayscale").is_some());

        // Overwrite with corrupt bytes -> rollback must keep previous valid plugin working!
        std::fs::write(tmp.path().join("wasm_invert.wasm"), b"NOT_A_WASM").unwrap();
        let ev2 = mgr.poll_and_sync(&mut reg);
        assert!(ev2.iter().any(|m| m.contains("検証失敗")));
        assert!(reg.get_effect("wasm.wasm_invert").is_some());

        let mut px = vec![[0.2, 0.2, 0.2, 1.0]];
        reg.apply_effect("wasm.wasm_invert", &mut px, 1, 1, 0.0, &BTreeMap::new())
            .unwrap();
        assert!((px[0][0] - 0.8).abs() < 1e-4);
    }
}
