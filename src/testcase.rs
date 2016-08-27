use std::str;
use std::collections::HashMap;

use sexpr::Sexpr;
use module::{AsBytes, Module, MemoryInfo, FunctionBuilder, Export, FunctionIndex};
use types::{Type, Dynamic, IntType, FloatType};
use ops::{NormalOp, IntBinOp, IntUnOp, IntCmpOp};
use interp::{Instance, InterpResult};

macro_rules! vec_form {
    ($val:expr => () => $code:expr) => {{
        if $val.len() == 0 {
            Some($code)
        } else {
            None
        }
    }};
    ($val:expr => (*$rest:ident) => $code:expr) => {{
        let $rest = &$val[..];
        Some($code)
    }};
    ($val:expr => (ident:&$ident:ident $($parts:tt)*) => $code:expr) => {{
        if $val.len() > 0 {
            if let &Sexpr::Identifier(ref $ident) = &$val[0] {
                vec_form!($val[1..] => ($($parts)*) => $code)
            } else {
                None
            }
        } else {
            None
        }
    }};
    ($val:expr => (str:&$ident:ident $($parts:tt)*) => $code:expr) => {{
        if $val.len() > 0 {
            if let &Sexpr::String(ref $ident) = &$val[0] {
                vec_form!($val[1..] => ($($parts)*) => $code)
            } else {
                None
            }
        } else {
            None
        }
    }};
    ($val:expr => (&$ident:ident $($parts:tt)*) => $code:expr) => {{
        if $val.len() > 0 {
            let $ident = &$val[0];

            vec_form!($val[1..] => ($($parts)*) => $code)
        } else {
            None
        }
    }};
    ($val:expr => ($ident:ident $($parts:tt)*) => $code:expr) => {{
        if $val.len() > 0 {
            if let &Sexpr::Identifier(ref name) = &$val[0] {
                if name == stringify!($ident) {
                    vec_form!($val[1..] => ($($parts)*) => $code)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }};
}

macro_rules! sexpr_match {
    ($val:expr;) => {{ None }};
    ($val:expr; _ => $code:expr) => {{ Some($code) }};
    ($val:expr; $sexpr:tt => $code:expr; $($sexpr_rest:tt => $code_rest:expr);*) => {{
        let val = $val;
        let res = if let &Sexpr::List(ref items) = val {
            vec_form!(items => $sexpr => $code)
        } else {
            None
        };
        if let None = res {
            sexpr_match!(val; $($sexpr_rest => $code_rest);*)
        } else {
            res
        }
    }};
}

pub struct Invoke {
    function_name: String,
    arguments: Vec<Dynamic>,
}

impl Invoke {
    fn run<'a, B: AsBytes>(&self, instance: &mut Instance<'a, B>) -> InterpResult {
        let func = instance.module.find(self.function_name.as_bytes()).unwrap();
        instance.invoke(func, &self.arguments)
    }
}

pub enum Assert {
    Return(Invoke, Dynamic),
    Trap(Invoke)
}

impl Assert {
    fn run<'a, B: AsBytes>(&self, instance: &mut Instance<'a, B>) {
        match self {
            &Assert::Return(ref invoke, result) => {
                assert_eq!(invoke.run(instance), InterpResult::Value(Some(result)));
            }
            &Assert::Trap(ref invoke) => {
                assert_eq!(invoke.run(instance), InterpResult::Trap);
            }
        }
    }
}

pub struct TestCase {
    module: Module<Vec<u8>>,
    asserts: Vec<Assert>
}

fn parse_type(text: &str) -> Type {
    match text {
        "i32" => Type::Int32,
        "i64" => Type::Int64,
        "f32" => Type::Float32,
        "f64" => Type::Float64,
        _ => panic!()
    }
}

fn parse_invoke(s: &Sexpr) -> Invoke {
    sexpr_match!(s;
        (invoke str:&name *args) => {
            let args = args.iter().map(parse_const).collect::<Vec<_>>();
            return Invoke {
                function_name: name.clone(),
                arguments: args
            };
        };
        _ => panic!()
    );
    panic!();
}

fn parse_const(s: &Sexpr) -> Dynamic {
    sexpr_match!(s;
        (ident:&ty &value) => {
            return match ty.as_str() {
                "i32.const" => parse_int(value, IntType::Int32),
                "i64.const" => parse_int(value, IntType::Int64),
                // &Sexpr::Identifier("f32.const") => {
                //     Dynamic::from_f32(parse_int(it[1]))
                // }
                // &Sexpr::Identifier("f64.const") => {
                //     Dynamic::from_f64(parse_int(it[1]))
                // }
                _ => panic!()
            };
        };
        _ => panic!()
    );
    panic!();
}

impl TestCase {
    pub fn parse(bytes: &[u8]) -> TestCase {
        let text = str::from_utf8(bytes).unwrap();
        let exprs = Sexpr::parse(text);

        let mut asserts = Vec::new();
        let mut module = None;

        for s in &exprs {
            sexpr_match!(s;
                (module *it) => {
                    let mut m = Module::<Vec<u8>>::new();

                    let mut function_names = HashMap::new();

                    for s in it {
                        sexpr_match!(s;
                            (func *it) => {
                                let mut it = it.iter();

                                let name = if let Some(&Sexpr::Variable(ref v)) = it.next() {
                                    Some(v)
                                } else {
                                    None
                                };

                                let mut func = FunctionBuilder::new();

                                let mut local_names = HashMap::new();

                                while let Some(s) = it.next() {
                                    sexpr_match!(s;
                                        (param &id &ty) => {
                                            if let &Sexpr::Variable(ref v) = id {
                                                local_names.insert(v.as_str(), func.ty.param_types.len());
                                            } else {
                                                panic!();
                                            }
                                            if let &Sexpr::Identifier(ref v) = ty {
                                                func.ty.param_types.push(parse_type(v.as_str()).to_u8());
                                            } else {
                                                panic!();
                                            }
                                        };
                                        (result &ty) => {
                                            if let &Sexpr::Identifier(ref v) = ty {
                                                func.ty.return_type = Some(parse_type(v.as_str()));
                                            } else {
                                                panic!();
                                            }
                                        };
                                        (local &id &ty) => {
                                            if let &Sexpr::Variable(ref v) = id {
                                                local_names.insert(v.as_str(), func.ty.param_types.len() + func.local_types.len());
                                            } else {
                                                panic!();
                                            }
                                            if let &Sexpr::Identifier(ref v) = ty {
                                                func.local_types.push(parse_type(v.as_str()));
                                            } else {
                                                panic!();
                                            }
                                        };
                                        _ => {
                                            parse_op(s, &mut func.ops, &local_names);
                                        }
                                    );
                                }

                                if let Some(name) = name {
                                    function_names.insert(name.as_str(), m.functions.len());
                                }

                                m.functions.push(func.ty.clone());
                                m.code.push(func.build());
                            };
                            (export &name &id) => {
                                match id {
                                    &Sexpr::Variable(ref id) => {
                                        match name {
                                            &Sexpr::String(ref name) => {
                                                m.exports.push(Export {
                                                    function_index: FunctionIndex(*function_names.get(id.as_str()).unwrap()),
                                                    function_name: Vec::from(name.as_bytes())
                                                });
                                            }
                                            _ => panic!()
                                        }
                                    }
                                    _ => panic!()
                                }
                            };
                            (import &module &name &ty) => {
                                // println!("found import!");
                            };
                            (import &id &module &name &ty) => {
                                // println!("found import!");
                            };
                            (type &id &ty) => {
                                // println!("found type!");
                            };
                            (type &ty) => {
                                // println!("found type!");
                            };
                            (memory *args) => {
                                // m.memory_info.initial_64k_pages = parse_int(initial);
                                // m.memory_info.maximum_64k_pages = parse_int(max);
                                //
                                // assert!(m.memory_info.maximum_64k_pages >= m.memory_info.initial_64k_pages);
                                //
                                // for s in segments {
                                //     sexpr_match!(s;
                                //         (segment &offset &data) => {
                                //             m.memory_chunks.push(MemoryChunk {
                                //                 offset: parse_int(offset),
                                //                 data: parse_bin_string(data),
                                //             })
                                //         };
                                //         _ => panic!("a")
                                //     );
                                // }
                            };
                            (table *items) => {
                                // println!("found table!");
                            };
                            (start &id) => {
                                // println!("found start!");
                            };
                            _ => {
                                panic!("unhandled inner: {}", s);
                            }
                        );
                    }
                    module = Some(m)
                };
                (assert_invalid &module &text) => {
                    panic!();
                };
                (assert_return &invoke &result) => {
                    asserts.push(Assert::Return(parse_invoke(invoke), parse_const(result)));
                };
                (assert_return_nan &invoke) => {
                    panic!();
                };
                (assert_trap &invoke &text) => {
                    asserts.push(Assert::Trap(parse_invoke(invoke)));
                };
                (invoke &ident *args) => {
                    panic!();
                };
                _ => {
                    panic!("unhandled: {}", s);
                }
            );
        }

        TestCase {
            module: module.unwrap(),
            asserts: asserts
        }
    }

    pub fn run_all(&self) {
        let mut instance = Instance::new(&self.module);
        for assert in &self.asserts {
            assert.run(&mut instance);
        }
    }
}

fn read_local(exprs: &[Sexpr], local_names: &HashMap<&str, usize>) -> usize {
    assert!(exprs.len() == 1);
    match &exprs[0] {
        &Sexpr::Variable(ref name) => *local_names.get(name.as_str()).unwrap(),
        _ => panic!()
    }
}

fn parse_ops(exprs: &[Sexpr], ops: &mut Vec<NormalOp>, local_names: &HashMap<&str, usize>) -> usize {
    let mut num = 0;
    for s in exprs {
        parse_op(s, ops, local_names);
        num += 1;
    }
    num
}

fn parse_op(s: &Sexpr, ops: &mut Vec<NormalOp>, local_names: &HashMap<&str, usize>) {
    sexpr_match!(s;
        (ident:&op *args) => {
            match op.as_str() {
                "nop" => {ops.push(NormalOp::Nop);},
                // "block" => NormalOp::Nop,
                // "loop" => NormalOp::Nop,
                // "if" => NormalOp::Nop,
                // "else" => NormalOp::Nop,
                // "select" => NormalOp::Nop,
                // "br" => NormalOp::Nop,
                // "brif" => NormalOp::Nop,
                // "brtable" => NormalOp::Nop,
                "return" => {
                    let num = parse_ops(args, ops, local_names);
                    assert!(num == 0 || num == 1);
                    ops.push(NormalOp::Return{has_arg: num == 1});
                }
                "unreachable" => { ops.push(NormalOp::Nop); }
                "drop" => { ops.push(NormalOp::Nop); }
                "end" => { ops.push(NormalOp::Nop); }
                "i32.const" => { ops.push(NormalOp::Nop); }
                "i64.const" => { ops.push(NormalOp::Nop); }
                "f64.const" => { ops.push(NormalOp::Nop); }
                "f32.const" => { ops.push(NormalOp::Nop); }
                "get_local" => {
                    ops.push(NormalOp::GetLocal(read_local(args, local_names)));
                }
                "set_local" => {
                    ops.push(NormalOp::SetLocal(read_local(args, local_names)));
                }
                "tee_local" => {
                    ops.push(NormalOp::TeeLocal(read_local(args, local_names)));
                }
                "call" => { ops.push(NormalOp::Nop); }
                "callindirect" => { ops.push(NormalOp::Nop); }
                "callimport" => { ops.push(NormalOp::Nop); }
                "i32.load8s" => { ops.push(NormalOp::Nop); }
                "i32.load8u" => { ops.push(NormalOp::Nop); }
                "i32.load16s" => { ops.push(NormalOp::Nop); }
                "i32.load16u" => { ops.push(NormalOp::Nop); }
                "i64.load8s" => { ops.push(NormalOp::Nop); }
                "i64.load8u" => { ops.push(NormalOp::Nop); }
                "i64.load16s" => { ops.push(NormalOp::Nop); }
                "i64.load16u" => { ops.push(NormalOp::Nop); }
                "i64.load32s" => { ops.push(NormalOp::Nop); }
                "i64.load32u" => { ops.push(NormalOp::Nop); }
                "i32.load" => { ops.push(NormalOp::Nop); }
                "i64.load" => { ops.push(NormalOp::Nop); }
                "f32.load" => { ops.push(NormalOp::Nop); }
                "f64.load" => { ops.push(NormalOp::Nop); }
                "i32.store8" => { ops.push(NormalOp::Nop); }
                "i32.store16" => { ops.push(NormalOp::Nop); }
                "i64.store8" => { ops.push(NormalOp::Nop); }
                "i64.store16" => { ops.push(NormalOp::Nop); }
                "i64.store32" => { ops.push(NormalOp::Nop); }
                "i32.store" => { ops.push(NormalOp::Nop); }
                "i64.store" => { ops.push(NormalOp::Nop); }
                "f32.store" => { ops.push(NormalOp::Nop); }
                "f64.store" => { ops.push(NormalOp::Nop); }
                "current_memory" => { ops.push(NormalOp::Nop); }
                "grow_memory" => { ops.push(NormalOp::Nop); }
                "i32.add" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Add));
                }
                "i32.sub" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Sub));
                }
                "i32.mul" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Mul));
                }
                "i32.div_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::DivS));
                }
                "i32.div_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::DivU));
                }
                "i32.rem_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::RemS));
                }
                "i32.rem_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::RemU));
                }
                "i32.and" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::And));
                }
                "i32.or" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Or));
                }
                "i32.xor" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Xor));
                }
                "i32.shl" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Shl));
                }
                "i32.shr_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::ShrU));
                }
                "i32.shr_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::ShrS));
                }
                "i32.rotr" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Rotr));
                }
                "i32.rotl" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int32, IntBinOp::Rotl));
                }
                "i32.eq" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::Eq));
                }
                "i32.ne" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::Ne));
                }
                "i32.lt_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::LtS));
                }
                "i32.le_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::LeS));
                }
                "i32.lt_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::LtU));
                }
                "i32.le_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::LeU));
                }
                "i32.gt_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::GtS));
                }
                "i32.ge_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::GeS));
                }
                "i32.gt_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::GtU));
                }
                "i32.ge_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int32, IntCmpOp::GeU));
                }
                "i32.clz" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntUn(IntType::Int32, IntUnOp::Clz));
                }
                "i32.ctz" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntUn(IntType::Int32, IntUnOp::Ctz));
                }
                "i32.popcnt" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntUn(IntType::Int32, IntUnOp::Popcnt));
                }
                "i32.eqz" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntEqz(IntType::Int32));
                }
                "i64.add" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Add));
                }
                "i64.sub" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Sub));
                }
                "i64.mul" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Mul));
                }
                "i64.div_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::DivS));
                }
                "i64.div_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::DivU));
                }
                "i64.rem_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::RemS));
                }
                "i64.rem_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::RemU));
                }
                "i64.and" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::And));
                }
                "i64.or" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Or));
                }
                "i64.xor" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Xor));
                }
                "i64.shl" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Shl));
                }
                "i64.shr_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::ShrU));
                }
                "i64.shr_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::ShrS));
                }
                "i64.rotr" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Rotr));
                }
                "i64.rotl" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntBin(IntType::Int64, IntBinOp::Rotl));
                }
                "i64.eq" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::Eq));
                }
                "i64.ne" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::Ne));
                }
                "i64.lt_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::LtS));
                }
                "i64.le_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::LeS));
                }
                "i64.lt_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::LtU));
                }
                "i64.le_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::LeU));
                }
                "i64.gt_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::GtS));
                }
                "i64.ge_s" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::GeS));
                }
                "i64.gt_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::GtU));
                }
                "i64.ge_u" => {
                    assert_eq!(parse_ops(args, ops, local_names), 2);
                    ops.push(NormalOp::IntCmp(IntType::Int64, IntCmpOp::GeU));
                }
                "i64.clz" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntUn(IntType::Int64, IntUnOp::Clz));
                }
                "i64.ctz" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntUn(IntType::Int64, IntUnOp::Ctz));
                }
                "i64.popcnt" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntUn(IntType::Int64, IntUnOp::Popcnt));
                }
                "i64.eqz" => {
                    assert_eq!(parse_ops(args, ops, local_names), 1);
                    ops.push(NormalOp::IntEqz(IntType::Int64));
                }
                "f32.add" => { ops.push(NormalOp::Nop); }
                "f32.sub" => { ops.push(NormalOp::Nop); }
                "f32.mul" => { ops.push(NormalOp::Nop); }
                "f32.div" => { ops.push(NormalOp::Nop); }
                "f32.min" => { ops.push(NormalOp::Nop); }
                "f32.max" => { ops.push(NormalOp::Nop); }
                "f32.abs" => { ops.push(NormalOp::Nop); }
                "f32.neg" => { ops.push(NormalOp::Nop); }
                "f32.copysign" => { ops.push(NormalOp::Nop); }
                "f32.ceil" => { ops.push(NormalOp::Nop); }
                "f32.floor" => { ops.push(NormalOp::Nop); }
                "f32.trunc" => { ops.push(NormalOp::Nop); }
                "f32.nearest" => { ops.push(NormalOp::Nop); }
                "f32.sqrt" => { ops.push(NormalOp::Nop); }
                "f32.eq" => { ops.push(NormalOp::Nop); }
                "f32.ne" => { ops.push(NormalOp::Nop); }
                "f32.lt" => { ops.push(NormalOp::Nop); }
                "f32.le" => { ops.push(NormalOp::Nop); }
                "f32.gt" => { ops.push(NormalOp::Nop); }
                "f32.ge" => { ops.push(NormalOp::Nop); }
                "f64.add" => { ops.push(NormalOp::Nop); }
                "f64.sub" => { ops.push(NormalOp::Nop); }
                "f64.mul" => { ops.push(NormalOp::Nop); }
                "f64.div" => { ops.push(NormalOp::Nop); }
                "f64.min" => { ops.push(NormalOp::Nop); }
                "f64.max" => { ops.push(NormalOp::Nop); }
                "f64.abs" => { ops.push(NormalOp::Nop); }
                "f64.neg" => { ops.push(NormalOp::Nop); }
                "f64.copysign" => { ops.push(NormalOp::Nop); }
                "f64.ceil" => { ops.push(NormalOp::Nop); }
                "f64.floor" => { ops.push(NormalOp::Nop); }
                "f64.trunc" => { ops.push(NormalOp::Nop); }
                "f64.nearest" => { ops.push(NormalOp::Nop); }
                "f64.sqrt" => { ops.push(NormalOp::Nop); }
                "f64.eq" => { ops.push(NormalOp::Nop); }
                "f64.ne" => { ops.push(NormalOp::Nop); }
                "f64.lt" => { ops.push(NormalOp::Nop); }
                "f64.le" => { ops.push(NormalOp::Nop); }
                "f64.gt" => { ops.push(NormalOp::Nop); }
                "f64.ge" => { ops.push(NormalOp::Nop); }
                "i32.trunc_s/f32" => { ops.push(NormalOp::Nop); }
                "i32.trunc_s/f64" => { ops.push(NormalOp::Nop); }
                "i32.trunc_u/f32" => { ops.push(NormalOp::Nop); }
                "i32.trunc_u/f64" => { ops.push(NormalOp::Nop); }
                "i32.wrap/i64" => { ops.push(NormalOp::Nop); }
                "i64.trunc_s/f32" => { ops.push(NormalOp::Nop); }
                "i64.trunc_s/f64" => { ops.push(NormalOp::Nop); }
                "i64.trunc_u/f32" => { ops.push(NormalOp::Nop); }
                "i64.trunc_u/f64" => { ops.push(NormalOp::Nop); }
                "i64.extend_s/i32" => { ops.push(NormalOp::Nop); }
                "i64.extend_u/i32" => { ops.push(NormalOp::Nop); }
                "f32.convert_s/i32" => { ops.push(NormalOp::Nop); }
                "f32.convert_u/i32" => { ops.push(NormalOp::Nop); }
                "f32.convert_s/i64" => { ops.push(NormalOp::Nop); }
                "f32.convert_u/i64" => { ops.push(NormalOp::Nop); }
                "f32.demote/f64" => { ops.push(NormalOp::Nop); }
                "f32.reinterpret/i32" => { ops.push(NormalOp::Nop); }
                "f64.convert_s/i32" => { ops.push(NormalOp::Nop); }
                "f64.convert_u/i32" => { ops.push(NormalOp::Nop); }
                "f64.convert_s/i64" => { ops.push(NormalOp::Nop); }
                "f64.convert_u/i64" => { ops.push(NormalOp::Nop); }
                "f64.promote/f32" => { ops.push(NormalOp::Nop); }
                "f64.reinterpret/i64" => { ops.push(NormalOp::Nop); }
                "i32.reinterpret/f32" => { ops.push(NormalOp::Nop); }
                "i64.reinterpret/f64" => { ops.push(NormalOp::Nop); }
                _ => panic!("unexpected instr: {}", op)
            };
        };
        _ => panic!("unexpected instr: {}", s)
    );
}

fn parse_int(node: &Sexpr, ty: IntType) -> Dynamic {
    match node {
        &Sexpr::Identifier(ref text) => {
            match ty {
                IntType::Int32 => {
                    if text.starts_with("-") {
                        Dynamic::from_i32(i32::from_str_radix(text, 10).unwrap())
                    } else if text.starts_with("0x") {
                        Dynamic::from_u32(u32::from_str_radix(&text[2..], 16).unwrap())
                    } else {
                        Dynamic::from_u32(u32::from_str_radix(text, 10).unwrap())
                    }
                }
                IntType::Int64 => {
                    if text.starts_with("-") {
                        Dynamic::from_i64(i64::from_str_radix(text, 10).unwrap())
                    } else if text.starts_with("0x") {
                        Dynamic::from_u64(u64::from_str_radix(&text[2..], 16).unwrap())
                    } else {
                        Dynamic::from_u64(u64::from_str_radix(text, 10).unwrap())
                    }
                }
            }
        }
        _ => panic!("expected number id: {}", node)
    }
}

fn parse_bin_string(node: &Sexpr) -> Vec<u8> {
    match node {
        &Sexpr::String(ref text) => {
            let text = text.as_bytes();
            let mut res = Vec::new();

            assert!(text[0] == b'"');

            let mut pos = 1;

            while pos < text.len() {
                match text[pos] {
                    b'\\' => {
                        assert!(pos + 2 < text.len());
                        res.push(u8::from_str_radix(str::from_utf8(&text[pos + 1..pos + 2]).unwrap(), 16).unwrap());
                    }
                    b'"' => break,
                    ch => res.push(ch)
                }
                pos += 1;
            }

            res
        }
        _ => panic!()
    }
}
