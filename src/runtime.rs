//! Rust host for the installed LINE Chrome extension's Emscripten/embind WASM.
//! Binding registration and emval conventions informed by beeper/line (MIT).
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::{collections::BTreeMap, fs::File, io::Read};
use wasmi::{Caller, Engine, ExternType, Func, Instance, Linker, Memory, Module, Store, Val};

// Public callback source embedded in the extension, matched as interface data.
const CALLBACK_THROW: &str = "{ throw UTF8ToString($0) + ': ' + UTF8ToString($1) }";
const CALLBACK_RANDOM_INIT: &str = "{ if (undefined === Module.getRandomValues) { try { var window_ = 'object' === typeof(window) ? window : self; var crypto_ = 'undefined' !== typeof(window_.crypto) ? window_.crypto : window_.msCrypto; if('function' !== typeof(crypto_.getRandomValues)) { throw 'Crypto.getRandomValues is not a function'; } Module.getRandomValues = function(data) { crypto_.getRandomValues(data); }; } catch (e) { } } if (undefined === Module.getRandomValues) { try { var crypto_ = require('crypto'); if('function' !== typeof(crypto_['randomFillSync'])) { throw 'crypto.randomFillSync is not a function'; } Module.getRandomValues = function(data) { crypto_['randomFillSync'](data); }; } catch (e) { } } return undefined === Module.getRandomValues ? 1 : 0; }";
const CALLBACK_RANDOM_FILL: &str = "{ var ret = 1; var data = HEAPU8.subarray($0, $0 + $1); try { Module.getRandomValues(data); ret = 0; } catch (e) { ret = 1; } return ret; }";

#[derive(Clone, Debug)]
enum Value {
    Undefined,
    Number(u64),
    String(String),
    Bytes(Vec<u8>),
    Destructors(Vec<u32>),
}
#[derive(Clone, Debug)]
struct Type {
    kind: &'static str,
    size: u32,
}
#[derive(Clone, Debug)]
struct Binding {
    invoker: u32,
    return_type: u32,
    context: u32,
    args: usize,
    types: Vec<u32>,
}
#[derive(Debug, Default)]
struct Class {
    name: String,
    base: u32,
    upcast: u32,
    destructor: u32,
    destructor_signature: String,
    upcast_signature: String,
    methods: BTreeMap<String, Binding>,
    statics: BTreeMap<String, Binding>,
    constructors: BTreeMap<usize, Binding>,
}
struct Handle {
    value: Value,
    refs: u32,
}
struct Host {
    types: BTreeMap<u32, Type>,
    classes: BTreeMap<u32, Class>,
    handles: Vec<Handle>,
    free_handles: Vec<u32>,
    urandom_fd: u32,
    globals: BTreeMap<String, Binding>,
    version: String,
}
impl Host {
    fn new(version: &str) -> Self {
        Self {
            types: BTreeMap::new(),
            classes: BTreeMap::new(),
            // Emscripten's reserved handles: undefined, null, true, false.
            handles: vec![
                Handle {
                    value: Value::Undefined,
                    refs: 1,
                },
                Handle {
                    value: Value::Undefined,
                    refs: 1,
                },
                Handle {
                    value: Value::Undefined,
                    refs: 1,
                },
                Handle {
                    value: Value::Number(1),
                    refs: 1,
                },
                Handle {
                    value: Value::Number(0),
                    refs: 1,
                },
            ],
            free_handles: Vec::new(),
            urandom_fd: 42,
            globals: BTreeMap::new(),
            version: version.into(),
        }
    }
    fn put(&mut self, value: Value) -> u32 {
        let h = Handle { value, refs: 1 };
        if let Some(i) = self.free_handles.pop() {
            self.handles[i as usize] = h;
            i
        } else {
            let i = self.handles.len() as u32;
            self.handles.push(h);
            i
        }
    }
    fn get(&self, i: u32) -> Result<Value> {
        let h = self
            .handles
            .get(i as usize)
            .context("invalid emval handle")?;
        ensure!(h.refs > 0, "released emval handle");
        Ok(h.value.clone())
    }
    fn dec(&mut self, i: u32) -> Result<()> {
        if i < 5 {
            return Ok(());
        }
        let h = self
            .handles
            .get_mut(i as usize)
            .context("invalid emval handle")?;
        ensure!(h.refs > 0, "emval refcount underflow");
        h.refs -= 1;
        if h.refs == 0 {
            h.value = Value::Undefined;
            self.free_handles.push(i);
        }
        Ok(())
    }
}
fn memory(c: &Caller<'_, Host>) -> Result<Memory> {
    c.get_export("O")
        .and_then(|x| x.into_memory())
        .context("missing WASM memory O")
}
fn read(c: &Caller<'_, Host>, p: u32, n: usize) -> Result<Vec<u8>> {
    let m = memory(c)?;
    let end = (p as usize)
        .checked_add(n)
        .context("memory range overflow")?;
    Ok(m.data(c)
        .get(p as usize..end)
        .context("out-of-bounds WASM read")?
        .to_vec())
}
fn write(c: &mut Caller<'_, Host>, p: u32, bytes: &[u8]) -> Result<()> {
    memory(c)?
        .write(c, p as usize, bytes)
        .map_err(|e| anyhow!("WASM write: {e}"))
}
fn u32_at(c: &Caller<'_, Host>, p: u32) -> Result<u32> {
    Ok(u32::from_le_bytes(read(c, p, 4)?.try_into().unwrap()))
}
fn cstr(c: &Caller<'_, Host>, p: u32) -> Result<String> {
    let m = memory(c)?;
    let s = m
        .data(c)
        .get(p as usize..)
        .context("invalid string pointer")?;
    let n = s
        .iter()
        .position(|b| *b == 0)
        .context("unterminated C string")?;
    Ok(std::str::from_utf8(&s[..n])?.to_owned())
}
fn call_export(c: &mut Caller<'_, Host>, name: &str, args: &[Val]) -> Result<Vec<Val>> {
    let f = c
        .get_export(name)
        .and_then(|e| e.into_func())
        .context("missing export")?;
    let mut out: Vec<_> = f
        .ty(&*c)
        .results()
        .iter()
        .map(|t| Val::default(*t))
        .collect();
    f.call(c, args, &mut out)?;
    Ok(out)
}
fn malloc(c: &mut Caller<'_, Host>, n: usize) -> Result<u32> {
    let n = i32::try_from(n).context("allocation too large")?;
    let out = call_export(c, "V", &[Val::I32(n)])?;
    let p = out[0].i32().context("malloc result")? as u32;
    ensure!(p != 0, "WASM allocation failed");
    Ok(p)
}
fn free(c: &mut Caller<'_, Host>, p: u32) -> Result<()> {
    call_export(c, "R", &[Val::I32(p as i32)])?;
    Ok(())
}
fn std_string(c: &mut Caller<'_, Host>, bytes: &[u8]) -> Result<u32> {
    let p = malloc(c, bytes.len() + 5)?;
    write(c, p, &(bytes.len() as u32).to_le_bytes())?;
    write(c, p + 4, bytes)?;
    write(c, p + 4 + bytes.len() as u32, &[0])?;
    Ok(p)
}
fn random_bytes(n: usize) -> Result<Vec<u8>> {
    let mut out = vec![0; n];
    File::open("/dev/urandom")?.read_exact(&mut out)?;
    Ok(out)
}
fn take_value(c: &Caller<'_, Host>, t: u32, p: u32) -> Result<Value> {
    let ty = c.data().types.get(&t).context("unregistered emval type")?;
    Ok(match ty.kind {
        "memory_view" => Value::Bytes(read(c, u32_at(c, p + 4)?, u32_at(c, p)? as usize)?),
        "int" | "bigint" => {
            let bytes = read(c, p, ty.size as usize)?;
            let mut raw = [0; 8];
            raw[..bytes.len()].copy_from_slice(&bytes);
            Value::Number(u64::from_le_bytes(raw))
        }
        "class" => Value::Number(u32_at(c, p)? as u64),
        other => bail!("unsupported take_value type {other}"),
    })
}
fn emval_decref(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    c.data_mut().dec(p(0))?;
    Ok(())
}

fn emval_incref(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    if p(0) >= 5 {
        c.data_mut()
            .handles
            .get_mut(p(0) as usize)
            .context("invalid incref")?
            .refs += 1;
    }
    Ok(())
}

fn register_method(
    kind: BindingKind,
    mut c: Caller<'_, Host>,
    input: &[Val],
    _output: &mut [Val],
) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let method = cstr(&c, p(1))?;
    let types = (1..p(2))
        .map(|i| u32_at(&c, p(3) + i * 4))
        .collect::<Result<Vec<_>>>()?;
    let binding = Binding {
        invoker: p(5),
        return_type: u32_at(&c, p(3))?,
        context: p(6),
        args: p(2) as usize - 1,
        types,
    };
    let class = c.data_mut().classes.entry(p(0)).or_default();
    if kind == BindingKind::Method {
        class.methods.insert(method, binding);
    } else {
        class.statics.insert(method, binding);
    }
    Ok(())
}

fn register_class(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let class = Class {
        name: cstr(&c, p(10))?,
        base: p(3),
        upcast: p(7),
        destructor: p(12),
        destructor_signature: cstr(&c, p(11))?,
        upcast_signature: if p(7) != 0 {
            cstr(&c, p(6))?
        } else {
            String::new()
        },
        ..c.data_mut().classes.remove(&p(0)).unwrap_or_default()
    };
    c.data_mut().classes.insert(p(0), class);
    for index in 0..3 {
        let id = p(index);
        c.data_mut().types.insert(
            id,
            Type {
                kind: "class",
                size: 4,
            },
        );
    }
    Ok(())
}

fn register_constructor(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let types = (1..p(1))
        .map(|i| u32_at(&c, p(2) + i * 4))
        .collect::<Result<Vec<_>>>()?;
    let binding = Binding {
        invoker: p(4),
        return_type: u32_at(&c, p(2))?,
        context: p(5),
        args: p(1) as usize - 1,
        types,
    };
    c.data_mut()
        .classes
        .entry(p(0))
        .or_default()
        .constructors
        .insert(binding.args, binding);
    Ok(())
}

fn register_type(mut c: Caller<'_, Host>, id: u32, kind: &'static str, size: u32) -> Result<()> {
    c.data_mut().types.insert(id, Type { kind, size });
    Ok(())
}

fn register_constant(_c: Caller<'_, Host>, _input: &[Val], _output: &mut [Val]) -> Result<()> {
    // Numeric configuration constants are not used by this host.
    Ok(())
}

fn register_function(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let name = cstr(&c, p(0))?;
    let types = (1..p(1))
        .map(|i| u32_at(&c, p(2) + i * 4))
        .collect::<Result<Vec<_>>>()?;
    let return_type = u32_at(&c, p(2))?;
    c.data_mut().globals.insert(
        name,
        Binding {
            invoker: p(4),
            return_type,
            context: p(5),
            args: p(1) as usize - 1,
            types,
        },
    );
    Ok(())
}

fn emval_string(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let s = cstr(&c, p(0))?;
    let result = c.data_mut().put(Value::String(s));
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn emval_take_value(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let v = take_value(&c, p(0), p(1))?;
    let result = c.data_mut().put(v);
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn emval_new(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let constructor = c.data().get(p(0))?;
    ensure!(
        matches!(&constructor, Value::String(s) if s == "Uint8Array" || s == "Int8Array"),
        "unsupported emval constructor {constructor:?}"
    );
    ensure!(p(1) == 1, "unsupported typed array arity");
    let v = take_value(&c, u32_at(&c, p(2))?, p(3))?;
    let bytes = match v {
        Value::Bytes(b) => b,
        Value::Number(n) => vec![0; usize::try_from(n)?],
        _ => bail!("typed array argument"),
    };
    let result = c.data_mut().put(Value::Bytes(bytes));
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn emval_array(mut c: Caller<'_, Host>, _input: &[Val], output: &mut [Val]) -> Result<()> {
    let result = c.data_mut().put(Value::Bytes(Vec::new()));
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn emval_get_property(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let obj = c.data().get(p(0))?;
    let key = c.data().get(p(1))?;
    let val = match (obj, key) {
        (Value::Bytes(b), Value::String(s)) if s == "length" || s == "byteLength" => {
            Value::Number(b.len() as u64)
        }
        (Value::Bytes(b), Value::Number(i)) => {
            Value::Number(*b.get(i as usize).context("array index")? as u64)
        }
        other => bail!("unsupported emval get_property {other:?}"),
    };
    let result = c.data_mut().put(val);
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn emval_set_property(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let key = c.data().get(p(1))?;
    let value = c.data().get(p(2))?;
    if let (Value::Number(i), Value::Number(v)) = (key, value) {
        let h = c
            .data_mut()
            .handles
            .get_mut(p(0) as usize)
            .context("array handle")?;
        let Value::Bytes(b) = &mut h.value else {
            bail!("set_property requires byte array")
        };
        let i = usize::try_from(i)?;
        ensure!(i < 16 * 1024 * 1024, "array too large");
        if i >= b.len() {
            b.resize(i + 1, 0);
        }
        b[i] = v as u8;
    } else {
        bail!("unsupported emval set_property")
    }
    Ok(())
}

fn emval_as(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let val = c.data().get(p(0))?;
    let ty = c
        .data()
        .types
        .get(&p(1))
        .context("unregistered conversion type")?
        .clone();
    let mut destructors = Vec::new();
    let value = match (ty.kind, val) {
        ("string", Value::Bytes(b)) => {
            let ptr = std_string(&mut c, &b)?;
            destructors.push(ptr);
            ptr as f64
        }
        ("string", Value::String(s)) => {
            let ptr = std_string(&mut c, s.as_bytes())?;
            destructors.push(ptr);
            ptr as f64
        }
        ("memory_view", Value::Bytes(b)) => {
            let ptr = malloc(&mut c, b.len().max(1))?;
            write(&mut c, ptr, &b)?;
            let desc = malloc(&mut c, 8)?;
            write(&mut c, desc, &(b.len() as u32).to_le_bytes())?;
            write(&mut c, desc + 4, &ptr.to_le_bytes())?;
            destructors.extend([ptr, desc]);
            desc as f64
        }
        ("int" | "bigint" | "float", Value::Number(n)) => n as f64,
        ("int" | "bigint", Value::Bytes(b)) => b.len() as f64,
        ("emval", _) => p(0) as f64,
        other => bail!("unsupported emval conversion {other:?}"),
    };
    let h = c.data_mut().put(Value::Destructors(destructors));
    write(&mut c, p(2), &h.to_le_bytes())?;
    output[0] = Val::F64(value.into());
    Ok(())
}

fn emval_run_destructors(
    mut c: Caller<'_, Host>,
    input: &[Val],
    _output: &mut [Val],
) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let v = c.data().get(p(0))?;
    let Value::Destructors(ptrs) = v else {
        bail!("invalid destructor list")
    };
    for ptr in ptrs {
        free(&mut c, ptr)?;
    }
    c.data_mut().dec(p(0))?;
    Ok(())
}

fn origin(mut c: Caller<'_, Host>, _input: &[Val], output: &mut [Val]) -> Result<()> {
    let origin = b"chrome-extension://ophjlpahpchlmihnnnihgmmeilfjmjjc\0";
    let result = malloc(&mut c, origin.len())?;
    write(&mut c, result, origin)?;
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn embedded_callback(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    // Identify known embedded callback source at the supplied pointer,
    // so data relocation across extension builds needs no fixed offsets.
    // No JavaScript is executed; these callbacks are implemented in Rust.
    let source = cstr(&c, p(0))?;
    match source.as_str() {
        CALLBACK_THROW => bail!(
            "LTSM {}: {}",
            cstr(&c, u32_at(&c, p(2))?)?,
            cstr(&c, u32_at(&c, p(2) + 4)?)?
        ),
        CALLBACK_RANDOM_INIT => {}
        CALLBACK_RANDOM_FILL => {
            let ptr = u32_at(&c, p(2))?;
            let n = u32_at(&c, p(2) + 4)?;
            write(&mut c, ptr, &random_bytes(n as usize)?)?;
        }
        _ => return Err(crypto_changed(&c.data().version, "EM_JS callback")),
    }
    output[0] = Val::I32(0);
    Ok(())
}

fn allocate(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let result = malloc(&mut c, p(0) as usize)?;
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn throw_exception(_c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    bail!("LTSM C++ exception (type={}, ptr={})", p(1), p(0))
}

fn abort(_c: Caller<'_, Host>, _input: &[Val], _output: &mut [Val]) -> Result<()> {
    bail!("LTSM abort")
}

fn resize_heap(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    // emscripten_resize_heap
    let mem = memory(&c)?;
    let current = mem.data(&c).len();
    let wanted = p(0) as usize;
    let result = if wanted > current {
        let pages = (wanted - current).div_ceil(65536) as u64;
        u32::from(mem.grow(&mut c, pages).is_ok())
    } else {
        1
    };
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn open_random(c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let path = cstr(&c, p(1))?;
    ensure!(
        path == "/dev/urandom" || path == "/dev/random",
        "unsupported open path {path}"
    );
    let result = c.data().urandom_fd;
    output[0] = Val::I32(result as i32);
    Ok(())
}

fn fd_read(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    ensure!(p(0) == c.data().urandom_fd, "unsupported fd_read");
    let mut total = 0;
    for i in 0..p(2) {
        let ptr = u32_at(&c, p(1) + i * 8)?;
        let n = u32_at(&c, p(1) + i * 8 + 4)?;
        write(&mut c, ptr, &random_bytes(n as usize)?)?;
        total += n;
    }
    write(&mut c, p(3), &total.to_le_bytes())?;
    output[0] = Val::I32(0);
    Ok(())
}

fn fd_write(mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let mut total: u32 = 0;
    for i in 0..p(2) {
        total += u32_at(&c, p(1) + i * 8 + 4)?;
    }
    write(&mut c, p(3), &total.to_le_bytes())?;
    output[0] = Val::I32(0);
    Ok(())
}

fn fd_close(c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    ensure!(p(0) == c.data().urandom_fd, "unsupported fd_close");
    output[0] = Val::I32(0);
    Ok(())
}

fn fd_metadata(_c: Caller<'_, Host>, _input: &[Val], output: &mut [Val]) -> Result<()> {
    // fcntl/ioctl metadata; no host file access.
    output[0] = Val::I32(0);
    Ok(())
}

fn fd_seek(_c: Caller<'_, Host>, _input: &[Val], _output: &mut [Val]) -> Result<()> {
    bail!("fd_seek not supported")
}

fn copy_memory(mut c: Caller<'_, Host>, input: &[Val], _output: &mut [Val]) -> Result<()> {
    let p = |index: usize| input[index].i32().unwrap() as u32;
    let b = read(&c, p(1), p(2) as usize)?;
    write(&mut c, p(0), &b)?;
    Ok(())
}

type Handler = fn(Caller<'_, Host>, &[Val], &mut [Val]) -> Result<()>;
struct Function {
    name: &'static str,
    params: &'static [wasmi::core::ValType],
    results: &'static [wasmi::core::ValType],
    handler: Option<Handler>,
}
use wasmi::core::ValType::{F64, I32, I64};
macro_rules! host_function {
    ($name:literal, $params:expr, $results:expr, $handler:expr) => {
        Function {
            name: $name,
            params: $params,
            results: $results,
            handler: Some($handler),
        }
    };
}
macro_rules! export_function {
    ($name:literal, $params:expr, $results:expr) => {
        Function {
            name: $name,
            params: $params,
            results: $results,
            handler: None,
        }
    };
}
const FUNCTIONS: &[Function] = &[
    host_function!("a", &[I32], &[], emval_decref),
    host_function!(
        "b",
        &[I32, I32, I32, I32, I32, I32, I32, I32],
        &[],
        |c, p, r| register_method(Method, c, p, r)
    ),
    host_function!("c", &[I32, I32, F64], &[], register_constant),
    host_function!("d", &[I32, I32, I32], &[], |c, p, _| register_type(
        c,
        p[0].i32().unwrap() as u32,
        "memory_view",
        0
    )),
    host_function!("e", &[I32, I32, I32, I32], &[I32], emval_new),
    host_function!("f", &[I32], &[I32], emval_string),
    host_function!("g", &[I32, I32, I32, I32, I32, I32, I32], &[], |c, p, r| {
        register_method(Static, c, p, r)
    }),
    host_function!("h", &[I32, I32, I32, I32, I32], &[], |c, p, _| {
        register_type(
            c,
            p[0].i32().unwrap() as u32,
            "int",
            p[2].i32().unwrap() as u32,
        )
    }),
    host_function!(
        "i",
        &[
            I32, I32, I32, I32, I32, I32, I32, I32, I32, I32, I32, I32, I32,
        ],
        &[],
        register_class
    ),
    host_function!("j", &[I32], &[], emval_incref),
    host_function!(
        "k",
        &[I32, I32, I32, I32, I32, I32],
        &[],
        register_constructor
    ),
    host_function!("l", &[I32, I32], &[I32], emval_take_value),
    host_function!("m", &[], &[], abort),
    host_function!("n", &[I32, I32, I32], &[], |c, p, _| register_type(
        c,
        p[0].i32().unwrap() as u32,
        "wstring",
        p[1].i32().unwrap() as u32
    )),
    host_function!("o", &[I32, I32, I32], &[I32], embedded_callback),
    host_function!("p", &[I32], &[I32], allocate),
    host_function!("q", &[I32], &[I32], fd_close),
    host_function!("r", &[I32, I32, I32, I32], &[I32], fd_write),
    host_function!("s", &[I32, I32, I32], &[I32], fd_metadata),
    host_function!("t", &[I32, I32], &[], |c, p, _| register_type(
        c,
        p[0].i32().unwrap() as u32,
        "string",
        4
    )),
    host_function!("u", &[I32, I32, I32], &[], |c, p, _| register_type(
        c,
        p[0].i32().unwrap() as u32,
        "float",
        p[2].i32().unwrap() as u32
    )),
    host_function!("v", &[I32, I32, I32, I64, I64], &[], |c, p, _| {
        register_type(
            c,
            p[0].i32().unwrap() as u32,
            "bigint",
            p[2].i32().unwrap() as u32,
        )
    }),
    host_function!("w", &[], &[I32], origin),
    host_function!("x", &[], &[I32], emval_array),
    host_function!("y", &[I32], &[], emval_run_destructors),
    host_function!("z", &[I32, I32, I32], &[F64], emval_as),
    host_function!("A", &[I32, I32], &[I32], emval_get_property),
    host_function!("B", &[I32, I32, I32], &[], throw_exception),
    host_function!("C", &[I32], &[I32], resize_heap),
    host_function!("D", &[I32, I64, I32, I32], &[I32], fd_seek),
    host_function!("E", &[I32, I32, I32, I32], &[I32], fd_read),
    host_function!("F", &[I32, I32, I32], &[I32], fd_metadata),
    host_function!("G", &[I32, I32, I32, I32], &[I32], open_random),
    host_function!("H", &[I32, I32, I32], &[], copy_memory),
    host_function!("I", &[I32, I32], &[], |c, p, _| register_type(
        c,
        p[0].i32().unwrap() as u32,
        "emval",
        4
    )),
    host_function!("J", &[I32, I32, I32, I32, I32], &[], |c, p, _| {
        register_type(
            c,
            p[0].i32().unwrap() as u32,
            "bool",
            p[2].i32().unwrap() as u32,
        )
    }),
    host_function!("K", &[I32, I32], &[], |c, p, _| register_type(
        c,
        p[0].i32().unwrap() as u32,
        "void",
        0
    )),
    host_function!("L", &[I32, I32, I32], &[], emval_set_property),
    host_function!("M", &[I32], &[I32], emval_string),
    host_function!("N", &[I32, I32, I32, I32, I32, I32], &[], register_function),
    export_function!("P", &[], &[]),
    export_function!("T", &[], &[]),
    export_function!("V", &[I32], &[I32]),
    export_function!("R", &[I32], &[]),
];

/// Owned and called directly by the account net worker for live decode, history and signing.
struct ResolvedBinding {
    invoker: Func,
    context: u32,
    emval_args: Vec<usize>,
    upcasts: Vec<Func>,
}
pub struct Runtime {
    store: Store<Host>,
    instance: Instance,
    poisoned: bool,
    class_ids: BTreeMap<String, u32>,
    bindings: Vec<ResolvedBinding>,
    destructors: BTreeMap<String, Func>,
}
impl Runtime {
    pub fn new(bytes: &[u8], version: &str) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes)?;
        let mut linker = Linker::new(&engine);
        for import in module.imports() {
            let spec = FUNCTIONS
                .iter()
                .find(|s| s.name == import.name() && s.handler.is_some())
                .filter(|_| import.module() == "a")
                .ok_or_else(|| crypto_changed(version, import.name()))?;
            let ExternType::Func(ty) = import.ty() else {
                return Err(crypto_changed(version, import.name()));
            };
            if ty.params() != spec.params || ty.results() != spec.results {
                return Err(crypto_changed(version, import.name()));
            }
            let handler = spec.handler.expect("host handler");
            linker.func_new("a", spec.name, ty.clone(), move |c, args, out| {
                handler(c, args, out).map_err(|e| match e.downcast::<crate::client::Error>() {
                    Ok(e) => wasmi::Error::host(e),
                    Err(e) => wasmi::Error::new(format!("import {}: {e:#}", spec.name)),
                })
            })?;
        }
        for spec in FUNCTIONS.iter().filter(|s| s.handler.is_none()) {
            let valid = module.exports().find(|e| e.name() == spec.name).is_some_and(|e| {
                matches!(e.ty(), ExternType::Func(ty) if ty.params() == spec.params && ty.results() == spec.results)
            });
            if !valid {
                return Err(crypto_changed(version, spec.name));
            }
        }
        for (name, memory) in [("O", true), ("Q", false)] {
            let valid = module
                .exports()
                .find(|e| e.name() == name)
                .is_some_and(|e| match (e.ty(), memory) {
                    (ExternType::Memory(ty), true) => !ty.is_64(),
                    (ExternType::Table(ty), false) => {
                        !ty.is_64() && ty.element() == wasmi::core::ValType::FuncRef
                    }
                    _ => false,
                });
            if !valid {
                return Err(crypto_changed(version, name));
            }
        }
        let mut store = Store::new(&engine, Host::new(version));
        let instance = linker
            .instantiate(&mut store, &module)?
            .start(&mut store)
            .map_err(runtime_error)?;
        let mut rt = Self {
            store,
            instance,
            poisoned: false,
            class_ids: BTreeMap::new(),
            bindings: Vec::new(),
            destructors: BTreeMap::new(),
        };
        rt.export("P", &[])?; // C++ constructors
        rt.export("T", &[])?; // embind type and class registrations
        rt.class_ids = rt
            .store
            .data()
            .classes
            .iter()
            .map(|(id, c)| (c.name.clone(), *id))
            .collect();
        let (invoker, context) = rt.validate_interface()?;
        if rt.indirect(invoker, &[context as u64])? == 0 {
            return Err(crypto_changed(version, "isAvailable"));
        }
        Ok(rt)
    }
    pub fn reinitialize(&mut self, bytes: &[u8], version: &str) -> Result<()> {
        *self = Self::new(bytes, version)?;
        Ok(())
    }
    fn binding(&self, call: Call) -> Result<(Binding, Vec<u32>)> {
        let (class, name, arity, kind) = call.binding();
        let missing = || {
            crypto_changed(
                &self.store.data().version,
                &format!("{class}.{name}/{arity}"),
            )
        };
        let mut c = self.class(class).map_err(|_| missing())?;
        let mut upcasts = Vec::new();
        loop {
            let binding = match kind {
                BindingKind::Static => c.statics.get(name),
                BindingKind::Method => c.methods.get(name),
                BindingKind::Constructor => c.constructors.get(&arity),
            };
            if let Some(b) = binding {
                if b.args != arity {
                    return Err(missing());
                }
                return Ok((b.clone(), upcasts));
            }
            if kind != BindingKind::Method {
                return Err(missing());
            }
            if c.upcast_signature != "ii" {
                return Err(missing());
            }
            upcasts.push(c.upcast);
            c = self.store.data().classes.get(&c.base).ok_or_else(missing)?;
        }
    }
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    fn validate_indirect(
        &self,
        index: u32,
        params: &[wasmi::core::ValType],
        results: &[wasmi::core::ValType],
    ) -> Result<Func> {
        let missing = || {
            crypto_changed(
                &self.store.data().version,
                &format!("function table {index}"),
            )
        };
        let f = self
            .instance
            .get_table(&self.store, "Q")
            .and_then(|t| t.get(&self.store, index as u64))
            .and_then(|v| v.funcref().and_then(|r| r.func()).copied())
            .ok_or_else(missing)?;
        let ty = f.ty(&self.store);
        if ty.params() != params || ty.results() != results {
            return Err(crypto_changed(
                &self.store.data().version,
                &format!("function table {index}: expected {params:?} -> {results:?}, got {ty:?}"),
            ));
        }
        Ok(f)
    }
    fn validate_invoker(&self, binding: &Binding) -> Result<Func> {
        let changed = || crypto_changed(&self.store.data().version, "invoker types");
        let wire_type = |id| {
            let ty = self.store.data().types.get(&id).ok_or_else(changed)?;
            match ty.kind {
                "bigint" => Ok(I64),
                "int" | "bool" | "class" | "emval" | "string" | "memory_view" => Ok(I32),
                _ => Err(changed()),
            }
        };
        let mut params = vec![I32]; // Invoker context precedes the registered arguments.
        for id in &binding.types {
            params.push(wire_type(*id)?);
        }
        let results = if self
            .store
            .data()
            .types
            .get(&binding.return_type)
            .is_some_and(|t| t.kind == "void")
        {
            vec![]
        } else {
            vec![wire_type(binding.return_type)?]
        };
        self.validate_indirect(binding.invoker, &params, &results)
    }
    fn validate_interface(&mut self) -> Result<(Func, u32)> {
        for &call in INTERFACE {
            let (binding, upcasts) = self.binding(call)?;
            let invoker = self.validate_invoker(&binding)?;
            let upcasts = upcasts
                .into_iter()
                .map(|i| self.validate_indirect(i, &[I32], &[I32]))
                .collect::<Result<_>>()?;
            let emval_args = binding
                .types
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    self.store
                        .data()
                        .types
                        .get(t)
                        .filter(|t| t.kind == "emval")
                        .map(|_| i)
                })
                .collect();
            self.bindings.push(ResolvedBinding {
                invoker,
                context: binding.context,
                emval_args,
                upcasts,
            });
        }
        for (name, id) in &self.class_ids {
            let class = &self.store.data().classes[id];
            if class.destructor_signature != "vi" {
                return Err(crypto_changed(
                    &self.store.data().version,
                    &format!("{name} destructor"),
                ));
            }
            let f = self.validate_indirect(class.destructor, &[I32], &[])?;
            self.destructors.insert(name.clone(), f);
        }
        let available = self
            .store
            .data()
            .globals
            .get("isAvailable")
            .filter(|b| b.args == 0)
            .ok_or_else(|| crypto_changed(&self.store.data().version, "isAvailable/0"))?;
        Ok((self.validate_invoker(available)?, available.context))
    }
    fn export(&mut self, name: &str, args: &[Val]) -> Result<Vec<Val>> {
        let f = self
            .instance
            .get_func(&self.store, name)
            .context("missing export")?;
        self.call(f, args)
    }
    fn call(&mut self, f: Func, args: &[Val]) -> Result<Vec<Val>> {
        ensure!(
            !self.poisoned,
            "runtime must be discarded after an exception"
        );
        let mut results: Vec<_> = f
            .ty(&self.store)
            .results()
            .iter()
            .map(|t| Val::default(*t))
            .collect();
        if let Err(e) = f.call(&mut self.store, args, &mut results) {
            self.poisoned = true;
            return Err(runtime_error(e));
        }
        Ok(results)
    }
    fn indirect(&mut self, f: Func, args: &[u64]) -> Result<u64> {
        let ty = f.ty(&self.store);
        let values: Result<Vec<Val>> = ty
            .params()
            .iter()
            .zip(args)
            .map(|(t, v)| {
                Ok(match t {
                    wasmi::core::ValType::I32 => Val::I32(*v as i32),
                    wasmi::core::ValType::I64 => Val::I64(*v as i64),
                    _ => bail!("unsupported invoker value type"),
                })
            })
            .collect();
        let out = self.call(f, &values?)?;
        Ok(out
            .first()
            .map(|v| {
                v.i32()
                    .map(|n| n as u32 as u64)
                    .or_else(|| v.i64().map(|n| n as u64))
                    .unwrap_or(0)
            })
            .unwrap_or(0))
    }
    fn class(&self, name: &str) -> Result<&Class> {
        let id = self
            .class_ids
            .get(name)
            .with_context(|| format!("class {name}"))?;
        Ok(&self.store.data().classes[id])
    }
    #[cfg(any(test, feature = "harness"))]
    pub fn bindings(&self) -> Vec<(String, Vec<String>)> {
        self.store
            .data()
            .classes
            .values()
            .map(|c| {
                (
                    c.name.clone(),
                    c.methods.keys().chain(c.statics.keys()).cloned().collect(),
                )
            })
            .collect()
    }
    pub fn static_call(&mut self, call: Call, args: &[u64]) -> Result<u32> {
        assert!(
            call.binding().3 == BindingKind::Static,
            "static call kind {call:?}"
        );
        self.bound_call(call, None, args)
    }
    pub fn construct(&mut self, call: Call, args: &[u64]) -> Result<u32> {
        assert!(
            call.binding().3 == BindingKind::Constructor,
            "constructor call kind {call:?}"
        );
        self.bound_call(call, None, args)
    }
    pub fn method(&mut self, call: Call, this: u32, args: &[u64]) -> Result<u32> {
        self.bound_call(call, Some(this), args)
    }
    fn bound_call(&mut self, call: Call, this: Option<u32>, args: &[u64]) -> Result<u32> {
        let (_, _, arity, kind) = call.binding();
        assert_eq!(
            this.is_some(),
            kind == BindingKind::Method,
            "call kind {call:?}"
        );
        assert_eq!(
            args.len() + usize::from(this.is_some()),
            arity,
            "call arity {call:?}"
        );
        (|| {
            let index = call as usize;
            let mut values = Vec::with_capacity(arity + 1);
            values.push(self.bindings[index].context as u64);
            if let Some(mut ptr) = this {
                for i in 0..self.bindings[index].upcasts.len() {
                    let f = self.bindings[index].upcasts[i];
                    ptr = self.indirect(f, &[ptr as u64])? as u32;
                }
                values.push(ptr as u64);
            }
            values.extend_from_slice(args);
            // The C++ val wrapper takes one reference; retain the caller's.
            for &i in &self.bindings[index].emval_args {
                let handle = values[i + 1];
                if handle >= 5 {
                    let h = self
                        .store
                        .data_mut()
                        .handles
                        .get_mut(handle as usize)
                        .context("invalid argument handle")?;
                    ensure!(h.refs > 0, "released argument handle");
                    h.refs += 1;
                }
            }
            self.indirect(self.bindings[index].invoker, &values)
                .map(|v| v as u32)
        })()
        .map_err(|error| {
            if error.downcast_ref::<crate::client::Error>().is_some() {
                error
            } else {
                error.context(crate::client::Error::Crypto)
            }
        })
    }
    pub fn destroy(&mut self, class: &str, ptr: u32) -> Result<()> {
        let f = *self.destructors.get(class).context("class destructor")?;
        self.indirect(f, &[ptr as u64])?;
        Ok(())
    }
    /// Read immutable module data for the synthetic opaque-ID test helper.
    #[cfg(any(test, feature = "harness"))]
    pub fn read_memory(&self, address: usize, len: usize) -> Result<Vec<u8>> {
        let mem = self
            .instance
            .get_memory(&self.store, "O")
            .context("missing memory")?;
        let end = address.checked_add(len).context("memory range overflow")?;
        Ok(mem
            .data(&self.store)
            .get(address..end)
            .context("memory read bounds")?
            .to_vec())
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> u64 {
        self.store.data_mut().put(Value::Bytes(bytes.to_vec())) as u64
    }
    pub fn release(&mut self, handle: u64) -> Result<()> {
        self.store.data_mut().dec(handle as u32)
    }
    /// Extract a class pointer returned through an emval handle (e.g. keychain.get).
    pub fn object(&mut self, handle: u32) -> Result<u32> {
        let value = self.store.data().get(handle)?;
        self.release(handle as u64)?;
        let Value::Number(ptr) = value else {
            bail!("expected object-pointer result")
        };
        let ptr = u32::try_from(ptr)?;
        ensure!(ptr != 0, "null object-pointer result");
        Ok(ptr)
    }
    pub fn output(&mut self, handle: u32) -> Result<Vec<u8>> {
        let value = self.store.data().get(handle)?;
        self.store.data_mut().dec(handle)?;
        let Value::Bytes(b) = value else {
            bail!("expected byte-array result")
        };
        Ok(b)
    }
    pub fn string(&mut self, s: &str) -> Result<u64> {
        let p = self.export("V", &[Val::I32((s.len() + 5) as i32)])?[0]
            .i32()
            .context("malloc")? as u32;
        ensure!(p != 0, "string allocation failed");
        let m = self
            .instance
            .get_memory(&self.store, "O")
            .context("memory")?;
        let mut b = (s.len() as u32).to_le_bytes().to_vec();
        b.extend_from_slice(s.as_bytes());
        b.push(0);
        m.write(&mut self.store, p as usize, &b)
            .map_err(|e| anyhow!("string write {e}"))?;
        Ok(p as u64)
    }
    pub fn free_string(&mut self, p: u64) -> Result<()> {
        self.export("R", &[Val::I32(p as i32)])?;
        Ok(())
    }
}

fn crypto_changed(version: &str, missing: &str) -> anyhow::Error {
    crate::client::Error::SignIn(crate::client::error::SignInFailure::CryptoChanged {
        version: version.into(),
        missing: missing.into(),
    })
    .into()
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingKind {
    Method,
    Static,
    Constructor,
}
use BindingKind::{Constructor, Method, Static};
// The protocol interface is defined once: validation and calls use these IDs.
macro_rules! interface {
    ($($id:ident => ($class:literal, $method:literal, $arity:literal, $kind:ident)),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Call { $($id),* }
        impl Call {
            fn binding(self) -> (&'static str, &'static str, usize, BindingKind) {
                match self { $(Self::$id => ($class, $method, $arity, $kind)),* }
            }
        }
        const INTERFACE: &[Call] = &[$(Call::$id),*];
    };
}
interface! {
    SecureKeyLoadToken => ("SecureKey", "loadToken", 1, Static),
    SecureKeyLoadKey => ("SecureKey", "loadKey", 1, Static),
    SecureKeyDeriveKey => ("SecureKey", "deriveKey", 3, Method),
    SecureKeyExportKey => ("SecureKey", "exportKey", 1, Method),
    HmacNew => ("Hmac", "constructor", 1, Constructor),
    HmacDigest => ("Hmac", "digest", 2, Method),
    Curve25519KeyNew => ("Curve25519Key", "constructor", 1, Constructor),
    Curve25519KeyLoadKey => ("Curve25519Key", "loadKey", 1, Static),
    Curve25519KeyGenerate => ("Curve25519Key", "generate", 1, Static),
    Curve25519KeyCreateChannel => ("Curve25519Key", "createChannel", 2, Method),
    Curve25519KeyGetPublicKey => ("Curve25519Key", "getPublicKey", 1, Method),
    Curve25519KeyExportKey => ("Curve25519Key", "exportKey", 1, Method),
    E2EEKeyLoadKey => ("E2EEKey", "loadKey", 1, Static),
    E2EEKeyCreateChannel => ("E2EEKey", "createChannel", 2, Method),
    E2EEKeyGetPublicKey => ("E2EEKey", "getPublicKey", 1, Method),
    E2EEKeyExportKey => ("E2EEKey", "exportKey", 1, Method),
    E2EEKeyGetKeyId => ("E2EEKey", "getKeyId", 1, Method),
    E2EEKeyGetVersion => ("E2EEKey", "getVersion", 1, Method),
    E2EEChannelNew => ("E2EEChannel", "constructor", 1, Constructor),
    E2EEChannelUnwrapE2EEKeyChain => ("E2EEChannel", "unwrapE2EEKeyChain", 2, Method),
    E2EEChannelUnwrapGroupSharedKey => ("E2EEChannel", "unwrapGroupSharedKey", 2, Method),
    E2EEChannelEncryptV1 => ("E2EEChannel", "encryptV1", 2, Method),
    E2EEChannelDecryptV1 => ("E2EEChannel", "decryptV1", 2, Method),
    E2EEChannelEncryptV2 => ("E2EEChannel", "encryptV2", 8, Method),
    E2EEChannelDecryptV2 => ("E2EEChannel", "decryptV2", 7, Method),
    E2EEKeychainSize => ("E2EEKeychain", "size", 1, Method),
    E2EEKeychainGet => ("E2EEKeychain", "get", 2, Method),
    AesKeyNew => ("AesKey", "constructor", 1, Constructor),
    AesKeyGenerate => ("AesKey", "generate", 2, Static),
    AesKeyEncrypt => ("AesKey", "encrypt", 2, Method),
    AesKeyDecrypt => ("AesKey", "decrypt", 2, Method),
}

fn runtime_error(error: wasmi::Error) -> anyhow::Error {
    match error.downcast_ref::<crate::client::Error>() {
        Some(error) => error.clone().into(),
        None => error.into(),
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    fn imported(name: u8, params: &[u8]) -> Vec<u8> {
        // A minimal module with one function import, no code or account data.
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        let mut types = vec![1, 0x60, params.len() as u8];
        types.extend_from_slice(params);
        types.push(0);
        wasm.extend([1, types.len() as u8]);
        wasm.extend(types);
        wasm.extend([2, 7, 1, 1, b'a', 1, name, 0, 0]);
        wasm
    }
    #[test]
    fn changed_registered_invoker_and_destructor_types_are_rejected() -> Result<()> {
        let wasm = crate::test_support::read_fixture(&crate::test_support::fixture_path())?;
        let mut rt = Runtime::new(&wasm, crate::test_support::FIXTURE_VERSION)?;
        let id = rt.class_ids["SecureKey"];
        let class = &mut rt.store.data_mut().classes.get_mut(&id).unwrap();
        let binding = class.statics.get_mut("loadToken").unwrap();
        let invoker = binding.invoker;
        binding.invoker = class.destructor;
        let error = rt.validate_interface().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::client::Error>(),
            Some(crate::client::Error::SignIn(
                crate::client::error::SignInFailure::CryptoChanged { .. }
            ))
        ));
        let class = rt.store.data_mut().classes.get_mut(&id).unwrap();
        class.statics.get_mut("loadToken").unwrap().invoker = invoker;
        class.destructor = invoker;
        let error = rt.validate_interface().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::client::Error>(),
            Some(crate::client::Error::SignIn(
                crate::client::error::SignInFailure::CryptoChanged { .. }
            ))
        ));
        Ok(())
    }
    #[test]
    fn wrong_export_signature_is_a_crypto_change() {
        let wasm = b"\0asm\x01\0\0\0\x01\x05\x01\x60\x01\x7f\0\x03\x02\x01\0\x07\x05\x01\x01P\0\0\x0a\x04\x01\x02\0\x0b";
        let error = Runtime::new(wasm, "synthetic")
            .err()
            .expect("bad export accepted");
        assert!(
            matches!(error.downcast_ref::<crate::client::Error>(), Some(crate::client::Error::SignIn(crate::client::error::SignInFailure::CryptoChanged { missing, .. })) if missing == "P")
        );
    }
    #[test]
    fn unknown_imports_and_wrong_signatures_fail_before_linking() {
        for (name, params) in [
            (b'?', &[][..]),
            (b'a', &[][..]),
            (b'c', &[0x7f, 0x7f, 0x7f][..]),
        ] {
            let error = Runtime::new(&imported(name, params), "synthetic")
                .err()
                .expect("invalid interface accepted");
            assert!(matches!(
                error.downcast_ref::<crate::client::Error>(),
                Some(crate::client::Error::SignIn(
                    crate::client::error::SignInFailure::CryptoChanged { .. }
                ))
            ));
        }
    }
}
