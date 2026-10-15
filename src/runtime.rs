//! Minimal Rust host for LINE Chrome 3.7.2's unmodified Emscripten/embind WASM.
//! Binding registration and emval conventions informed by beeper/line (MIT).
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::{collections::BTreeMap, fs::File, io::Read};
use wasmi::{Caller, Engine, ExternType, Func, Instance, Linker, Memory, Module, Store, Val};

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
    context: u32,
    args: usize,
    types: Vec<u32>,
}
#[derive(Clone, Debug, Default)]
struct Class {
    name: String,
    base: u32,
    upcast: u32,
    destructor: u32,
    methods: BTreeMap<String, Binding>,
    statics: BTreeMap<String, Binding>,
    constructors: Vec<Binding>,
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
}
impl Default for Host {
    fn default() -> Self {
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
        }
    }
}
impl Host {
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
fn host_call(name: &str, mut c: Caller<'_, Host>, input: &[Val], output: &mut [Val]) -> Result<()> {
    let p: Vec<u32> = input
        .iter()
        .map(|v| v.i32().map(|x| x as u32).unwrap_or(0))
        .collect();
    let mut result = 0;
    match name {
        "a" => c.data_mut().dec(p[0])?,
        "j" => {
            if p[0] >= 5 {
                c.data_mut()
                    .handles
                    .get_mut(p[0] as usize)
                    .context("invalid incref")?
                    .refs += 1;
            }
        }
        "b" | "g" => {
            let method = cstr(&c, p[1])?;
            let types = (1..p[2])
                .map(|i| u32_at(&c, p[3] + i * 4))
                .collect::<Result<Vec<_>>>()?;
            let binding = Binding {
                invoker: p[5],
                context: p[6],
                args: p[2] as usize - 1,
                types,
            };
            let class = c.data_mut().classes.entry(p[0]).or_default();
            if name == "b" {
                class.methods.insert(method, binding);
            } else {
                class.statics.insert(method, binding);
            }
        }
        "i" => {
            let class = Class {
                name: cstr(&c, p[10])?,
                base: p[3],
                upcast: p[7],
                destructor: p[12],
                ..c.data_mut().classes.remove(&p[0]).unwrap_or_default()
            };
            c.data_mut().classes.insert(p[0], class);
            for id in &p[..3] {
                c.data_mut().types.insert(
                    *id,
                    Type {
                        kind: "class",
                        size: 4,
                    },
                );
            }
        }
        "k" => {
            let types = (1..p[1])
                .map(|i| u32_at(&c, p[2] + i * 4))
                .collect::<Result<Vec<_>>>()?;
            let binding = Binding {
                invoker: p[4],
                context: p[5],
                args: p[1] as usize - 1,
                types,
            };
            c.data_mut()
                .classes
                .entry(p[0])
                .or_default()
                .constructors
                .push(binding);
        }
        "d" | "h" | "n" | "t" | "u" | "v" | "I" | "J" | "K" => {
            let (kind, size) = match name {
                "d" => ("memory_view", 0),
                "h" => ("int", p[2]),
                "n" => ("wstring", p[1]),
                "t" => ("string", 4),
                "u" => ("float", p[2]),
                "v" => ("bigint", p[2]),
                "I" => ("emval", 4),
                "J" => ("bool", p[2]),
                "K" => ("void", 0),
                _ => unreachable!(),
            };
            c.data_mut().types.insert(p[0], Type { kind, size });
        }
        "c" => {} // Numeric configuration constants are not used by this PoC.
        "N" => {
            let name = cstr(&c, p[0])?;
            let types = (1..p[1])
                .map(|i| u32_at(&c, p[2] + i * 4))
                .collect::<Result<Vec<_>>>()?;
            c.data_mut().globals.insert(
                name,
                Binding {
                    invoker: p[4],
                    context: p[5],
                    args: p[1] as usize - 1,
                    types,
                },
            );
        }
        "f" | "M" => {
            let s = cstr(&c, p[0])?;
            result = c.data_mut().put(Value::String(s));
        }
        "l" => {
            let v = take_value(&c, p[0], p[1])?;
            result = c.data_mut().put(v);
        }
        "e" => {
            let constructor = c.data().get(p[0])?;
            ensure!(
                matches!(&constructor, Value::String(s) if s == "Uint8Array" || s == "Int8Array"),
                "unsupported emval constructor {constructor:?}"
            );
            ensure!(p[1] == 1, "unsupported typed array arity");
            let v = take_value(&c, u32_at(&c, p[2])?, p[3])?;
            let bytes = match v {
                Value::Bytes(b) => b,
                Value::Number(n) => vec![0; usize::try_from(n)?],
                _ => bail!("typed array argument"),
            };
            result = c.data_mut().put(Value::Bytes(bytes));
        }
        "x" => result = c.data_mut().put(Value::Bytes(Vec::new())),
        "A" => {
            let obj = c.data().get(p[0])?;
            let key = c.data().get(p[1])?;
            let val = match (obj, key) {
                (Value::Bytes(b), Value::String(s)) if s == "length" || s == "byteLength" => {
                    Value::Number(b.len() as u64)
                }
                (Value::Bytes(b), Value::Number(i)) => {
                    Value::Number(*b.get(i as usize).context("array index")? as u64)
                }
                other => bail!("unsupported emval get_property {other:?}"),
            };
            result = c.data_mut().put(val);
        }
        "L" => {
            let key = c.data().get(p[1])?;
            let value = c.data().get(p[2])?;
            if let (Value::Number(i), Value::Number(v)) = (key, value) {
                let h = c
                    .data_mut()
                    .handles
                    .get_mut(p[0] as usize)
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
        }
        "z" => {
            let val = c.data().get(p[0])?;
            let ty = c
                .data()
                .types
                .get(&p[1])
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
                ("emval", _) => p[0] as f64,
                other => bail!("unsupported emval conversion {other:?}"),
            };
            let h = c.data_mut().put(Value::Destructors(destructors));
            write(&mut c, p[2], &h.to_le_bytes())?;
            output[0] = Val::F64(value.into());
            return Ok(());
        }
        "y" => {
            let v = c.data().get(p[0])?;
            let Value::Destructors(ptrs) = v else {
                bail!("invalid destructor list")
            };
            for ptr in ptrs {
                free(&mut c, ptr)?;
            }
            c.data_mut().dec(p[0])?;
        }
        "w" => {
            let origin = b"chrome-extension://ophjlpahpchlmihnnnihgmmeilfjmjjc\0";
            result = malloc(&mut c, origin.len())?;
            write(&mut c, result, origin)?;
        }
        "o" => {
            // Emscripten embeds three JS callbacks, identified by offsets in this release.
            // No JavaScript is executed: each callback is implemented here in Rust.
            match p[0] {
                1_456_932 => bail!(
                    "LTSM {}: {}",
                    cstr(&c, u32_at(&c, p[2])?)?,
                    cstr(&c, u32_at(&c, p[2] + 4)?)?
                ),
                1_456_985 => {}
                1_457_725 => {
                    let ptr = u32_at(&c, p[2])?;
                    let n = u32_at(&c, p[2] + 4)?;
                    write(&mut c, ptr, &random_bytes(n as usize)?)?;
                }
                id => bail!("unsupported EM_JS callback {id}"),
            }
        }
        "p" => result = malloc(&mut c, p[0] as usize)?,
        "B" => bail!("LTSM C++ exception (type={}, ptr={})", p[1], p[0]),
        "m" => bail!("LTSM abort"),
        "C" => {
            // emscripten_resize_heap
            let mem = memory(&c)?;
            let current = mem.data(&c).len();
            let wanted = p[0] as usize;
            if wanted > current {
                let pages = (wanted - current).div_ceil(65536) as u64;
                result = u32::from(mem.grow(&mut c, pages).is_ok());
            } else {
                result = 1;
            }
        }
        "G" => {
            let path = cstr(&c, p[1])?;
            ensure!(
                path == "/dev/urandom" || path == "/dev/random",
                "unsupported open path {path}"
            );
            result = c.data().urandom_fd;
        }
        "E" => {
            ensure!(p[0] == c.data().urandom_fd, "unsupported fd_read");
            let mut total = 0;
            for i in 0..p[2] {
                let ptr = u32_at(&c, p[1] + i * 8)?;
                let n = u32_at(&c, p[1] + i * 8 + 4)?;
                write(&mut c, ptr, &random_bytes(n as usize)?)?;
                total += n;
            }
            write(&mut c, p[3], &total.to_le_bytes())?;
        }
        "r" => {
            let mut total: u32 = 0;
            for i in 0..p[2] {
                total += u32_at(&c, p[1] + i * 8 + 4)?;
            }
            write(&mut c, p[3], &total.to_le_bytes())?;
        }
        "q" => {
            ensure!(p[0] == c.data().urandom_fd, "unsupported fd_close");
        }
        "s" | "F" => result = 0, // fcntl/ioctl metadata; no host file access.
        "D" => bail!("fd_seek not supported"),
        "H" => {
            let b = read(&c, p[1], p[2] as usize)?;
            write(&mut c, p[0], &b)?;
        }
        other => bail!("unsupported import {other}"),
    }
    if let Some(out) = output.first_mut() {
        *out = Val::I32(result as i32);
    }
    Ok(())
}

pub struct Runtime {
    store: Store<Host>,
    instance: Instance,
    poisoned: bool,
}
impl Runtime {
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes)?;
        let mut linker = Linker::new(&engine);
        for import in module.imports() {
            ensure!(import.module() == "a", "unexpected import namespace");
            let ExternType::Func(ty) = import.ty() else {
                bail!("unexpected non-function import")
            };
            let name = import.name().to_owned();
            let dispatch = name.clone();
            linker.func_new("a", &name, ty.clone(), move |c, args, out| {
                host_call(&dispatch, c, args, out)
                    .map_err(|e| wasmi::Error::new(format!("import {dispatch}: {e:#}")))
            })?;
        }
        let mut store = Store::new(&engine, Host::default());
        let instance = linker.instantiate(&mut store, &module)?.start(&mut store)?;
        let mut rt = Self {
            store,
            instance,
            poisoned: false,
        };
        rt.export("P", &[])?; // C++ constructors
        rt.export("T", &[])?; // embind type and class registrations
        let binding = rt
            .store
            .data()
            .globals
            .get("isAvailable")
            .cloned()
            .context("isAvailable binding")?;
        ensure!(
            rt.invoke(binding, &[])? != 0,
            "LTSM not available for this origin"
        );
        Ok(rt)
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
            return Err(e.into());
        }
        Ok(results)
    }
    fn indirect(&mut self, index: u32, args: &[u64]) -> Result<u64> {
        let table = self
            .instance
            .get_table(&self.store, "Q")
            .context("missing function table")?;
        let val = table
            .get(&self.store, index as u64)
            .context("function table index")?;
        let f = *val
            .funcref()
            .and_then(|r| r.func())
            .context("null function pointer")?;
        let ty = f.ty(&self.store);
        ensure!(
            ty.params().len() == args.len(),
            "invoker argument count: expected {}, got {}",
            ty.params().len(),
            args.len()
        );
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
    fn class(&self, name: &str) -> Result<Class> {
        self.store
            .data()
            .classes
            .values()
            .find(|c| c.name == name)
            .cloned()
            .with_context(|| format!("class {name}"))
    }
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
    pub fn static_call(&mut self, class: &str, name: &str, args: &[u64]) -> Result<u32> {
        let b = self
            .class(class)?
            .statics
            .get(name)
            .cloned()
            .with_context(|| format!("static {class}.{name}"))?;
        self.invoke(b, args)
    }
    pub fn construct(&mut self, class: &str, args: &[u64]) -> Result<u32> {
        let b = self
            .class(class)?
            .constructors
            .iter()
            .find(|b| b.args == args.len())
            .cloned()
            .context("matching constructor")?;
        self.invoke(b, args)
    }
    pub fn method(&mut self, class: &str, name: &str, this: u32, args: &[u64]) -> Result<u32> {
        let mut c = self.class(class)?;
        let mut this = this;
        let b = loop {
            if let Some(b) = c.methods.get(name) {
                break b.clone();
            }
            ensure!(c.base != 0, "method {class}.{name}");
            this = self.indirect(c.upcast, &[this as u64])? as u32;
            c = self
                .store
                .data()
                .classes
                .get(&c.base)
                .cloned()
                .with_context(|| format!("method {class}.{name}"))?;
        };
        let mut a = vec![this as u64];
        a.extend_from_slice(args);
        self.invoke(b, &a)
    }
    fn invoke(&mut self, b: Binding, args: &[u64]) -> Result<u32> {
        ensure!(args.len() == b.args, "binding argument count");
        // emval arguments transfer one reference to the C++ val wrapper.
        // Retain our caller-owned reference until release() is called.
        for (t, a) in b.types.iter().zip(args) {
            if self
                .store
                .data()
                .types
                .get(t)
                .is_some_and(|t| t.kind == "emval")
                && *a >= 5
            {
                let h = self
                    .store
                    .data_mut()
                    .handles
                    .get_mut(*a as usize)
                    .context("invalid argument handle")?;
                ensure!(h.refs > 0, "released argument handle");
                h.refs += 1;
            }
        }
        let mut a = vec![b.context as u64];
        a.extend_from_slice(args);
        Ok(self.indirect(b.invoker, &a)? as u32)
    }
    pub fn destroy(&mut self, class: &str, ptr: u32) -> Result<()> {
        let i = self.class(class)?.destructor;
        self.indirect(i, &[ptr as u64])?;
        Ok(())
    }
    /// Read immutable module data for the synthetic opaque-ID test helper.
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
