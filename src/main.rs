use std::{collections::{HashMap, HashSet}, ffi::VaList, io::{self, Read, Write}, rc::Rc};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{code_reader::{load_script, parse_function_define}, default_compiler_ml::{CompileError, SystemCalls, parse_code}, engine::Engine, ml_script::SysFn, ml_structs::{ExprParser, Op, Type, Value}};

mod ml_structs;
mod default_compiler_ml;
mod code_reader;
mod ml_script;
mod engine;


fn main() {

    let code = load_script("test").unwrap();
    for line in code {
        println!("{}", line.0);
    }

    let mut engine = Engine::<MyCtx>::new();
    engine.register("print", &[Type::Str], Type::Null, write);
    engine.register("input", &[Type::Str], Type::Str, value_input);
    engine.register("time", &[], Type::Int, get_time);

    let mut script = match engine.compile(engine.read_code("test").unwrap()) {
        Ok(s) => s,
        Err(e) => {
            panic!("{e}")
        },
    };

    let mut ctx = &mut MyCtx { num: 0 };
    engine.script_run(&mut script, ctx);
    match engine.script_call(&mut script, "main", &[], ctx) {
        Some(_) => println!("Метод вызван"),
        None => println!("Метод не найден"),
    }
    match engine.script_call(&mut script, "recurcy", &[Value::Int(10)], ctx) {
        Some(_) => println!("Метод вызван"),
        None => println!("Метод не найден"),
    }

}

pub fn write(ctx: &mut MyCtx, args: &[Value]) -> Value {
    if args.len() > 0 {
        match &args[0] {
            Value::Str(t) => println!("{}", t),
            _ => {}
        };
    }
    Value::Null
}
pub fn value_sqrt(ctx: &mut MyCtx, args: &[Value]) -> Value {
    match args[0] {
        Value::Float(v) => Value::Float(v.sqrt()),
        _ => Value::Null
    }
}
pub fn value_input(_ctx: &mut MyCtx, args: &[Value]) -> Value {
    // Подсказка, как в Python: input("Имя: ")
    if let Some(Value::Str(prompt)) = args.first() {
        print!("{prompt}");
    }
    if io::stdout().flush().is_err() {
        return Value::Null;
    }

    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        // Конец ввода (Ctrl+D / Ctrl+Z) или ошибка чтения -> None
        Ok(0) | Err(_) => Value::Null,
        Ok(_) => {
            // Убираем один перевод строки в конце, как это делает Python
            let text = line.strip_suffix('\n').unwrap_or(&line);
            let text = text.strip_suffix('\r').unwrap_or(text);
            Value::Str(Rc::from(text.to_string()))
        }
    }
}
pub fn get_number(ctx: &mut MyCtx, args: &[Value]) -> Value {
    ctx.num += 1;
    Value::Int(ctx.num)
}
pub fn get_time(ctx: &mut MyCtx, args: &[Value]) -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Системное время оказалось раньше 1970 года");
    Value::Int(now.as_millis() as i64)
}

pub struct MyCtx {
    num: i64,
}
