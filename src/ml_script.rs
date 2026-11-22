use std::collections::HashMap;

use crate::ml_structs::{Command, Type, Value, count_expr};

pub type SysFn<C> = fn(&mut C, Vec<Value>) -> Value;

#[derive(Default)]
pub struct Script {
    pub funcs: Box<[Script]>,
    pub funcs_map: HashMap<String, usize>,
    pub commands: Box<[Command]>,
    pub total_variables: usize,
}

impl Script {
    pub fn main_run<C>(&self, ctx: &mut C, funcs: &[SysFn<C>]) -> Value {
        self.run(&mut vec![Value::Null; self.total_variables].into_boxed_slice(), &self.funcs, ctx, funcs)
    }
    pub fn run<C>(&self, variables: &mut Box<[Value]>, funcs: &Box<[Script]>, ctx: &mut C, sys_funcs: &[SysFn<C>]) -> Value {
        let mut i: usize = 0;

        while i < self.commands.len() {
            let command = &self.commands[i];

            match command {
                Command::Var(id, expr) => {
                    variables[*id as usize] = count_expr(expr, variables, funcs, ctx, sys_funcs);
                },
                Command::If(expr, jumpsize) => {
                    let v = count_expr(expr, variables, funcs, ctx, sys_funcs);
                    let cond = match v {
                        Value::Bool(v) => v,
                        Value::Float(v) => v > 0.0,
                        Value::Int(v) => v > 0,
                        Value::Null => false,
                        Value::Str(v) => !v.is_empty(),
                        //Value::List(v) => v.len() > 0,
                    };
                    if !cond {
                        i += *jumpsize as usize;
                    }
                },
                Command::Call(f_id, args) => {
                    // Считаем аргументы, пока скрипт никто не трогал
                    let values: Vec<Value> = args
                        .iter()
                        .map(|arg| count_expr(arg, variables, funcs, ctx, sys_funcs))
                        .collect();

                    // Записываем их в переменные функции и запускаем
                    let script = &funcs[*f_id as usize];
                    let mut variables = vec![Value::Null; script.total_variables].into_boxed_slice();
                    for (i, v) in values.into_iter().enumerate() {
                        variables[i] = v;
                    }
                    script.run(&mut variables, funcs, ctx, sys_funcs);
                },
                Command::SystemCall(f_id, args) => {
                    // Считаем аргументы, пока скрипт никто не трогал
                    let values: Vec<Value> = args
                        .iter()
                        .map(|arg| count_expr(arg, variables, funcs, ctx, sys_funcs))
                        .collect();
                    sys_funcs[*f_id as usize](ctx, values);
                },
                Command::While(expr, jumpsize) => {
                    let value = count_expr(expr, variables, funcs, ctx, sys_funcs);
                    let result = match value {
                        Value::Bool(v) => v,
                        _ => false,
                    };

                    if !result {
                        i += *jumpsize as usize;
                    }
                },
                Command::JumpUp(jumpsize) => {
                    i = *jumpsize as usize;
                    continue;
                }
                Command::Return(expr) => {
                    let v = count_expr(expr, variables, funcs, ctx, sys_funcs);
                    return v;
                }
            }

            i += 1;
        }

        Value::Null
    }
}