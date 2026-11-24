use std::collections::HashMap;

use crate::ml_structs::{Command, Type, Value, count_expr};

pub type SysFn<C> = fn(&mut C, &[Value]) -> Value;

#[derive(Default)]
pub struct Script {
    pub funcs: Box<[Script]>,
    pub funcs_map: HashMap<String, usize>,
    pub commands: Box<[Command]>,
    pub total_variables: usize,
    pub global_variables: Box<[Value]>,
}

impl Script {
    pub fn main_run<C>(&mut self, ctx: &mut C, funcs: &[SysFn<C>]) -> Value {

        let mut stack: Vec<Value> = vec![Value::Null; self.total_variables];
        let mut global_vars = std::mem::take(&mut self.global_variables);

        let mut ctx: RunTimeCtx<'_, C> = RunTimeCtx {
            stack: &mut stack,
            global_vars: &mut global_vars,
            funcs: &self.funcs,
            sys_funcs: funcs,
            ctx: ctx,
        };

        let result = self.run(&mut ctx, 0);
        self.global_variables = std::mem::take(&mut global_vars);
        result
    }
    // pub fn run<C>(&self, stack: &mut Vec<Value>, global_vars: &mut Box<[Value]>, funcs: &Box<[Script]>, ctx: &mut C, sys_funcs: &[SysFn<C>], base: usize) -> Value {
    pub fn run<C>(&self, runtime_ctx: &mut RunTimeCtx<C>, base: usize) -> Value {
        let mut i: usize = 0;

        while i < self.commands.len() {
            let command = &self.commands[i];

            match command {
                Command::Var(id, expr) => {
                    runtime_ctx.stack[*id as usize + base] = count_expr(
                        expr,
                        runtime_ctx,
                        base,
                    );
                },
                Command::GlobalVar(id, expr) => {
                    runtime_ctx.global_vars[*id as usize] = count_expr(
                        expr,
                        runtime_ctx,
                        base,
                    );
                }
                Command::If(expr, jumpsize) => {
                    let v = count_expr(
                        expr,
                        runtime_ctx,
                        base,
                    );
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

                    // Записываем их в переменные функции и запускаем
                    let script = &runtime_ctx.funcs[*f_id as usize];
                    let new_base = runtime_ctx.stack.len();
                    runtime_ctx.stack.resize(new_base + script.total_variables, Value::Null);

                    for (i, arg) in args.iter().enumerate() {
                        let v = count_expr(
                            arg,
                            runtime_ctx,
                            base,
                        );
                        runtime_ctx.stack[new_base + i] = v;
                    }

                    script.run(runtime_ctx, new_base);
                    runtime_ctx.stack.truncate(new_base);
                },
                Command::SystemCall(f_id, args) => {
                    let new_base = runtime_ctx.stack.len();
                    runtime_ctx.stack.resize(new_base + args.len(), Value::Null);
                    for (i, arg) in args.iter().enumerate() {
                        runtime_ctx.stack[new_base + i] = count_expr(
                            arg,
                            runtime_ctx,
                            base,
                        );
                    }
                    runtime_ctx.sys_funcs[*f_id as usize](runtime_ctx.ctx, &runtime_ctx.stack[new_base..]);
                    runtime_ctx.stack.truncate(new_base);
                },
                Command::While(expr, jumpsize) => {
                    let value = count_expr(
                        expr,
                        runtime_ctx,
                        base,
                    );
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
                    let value = count_expr(
                        expr,
                        runtime_ctx,
                        base,
                    );
                    return value;
                }
            }

            i += 1;
        }

        Value::Null
    }
}

pub struct RunTimeCtx<'a, T> {
    pub stack: &'a mut Vec<Value>,
    pub global_vars: &'a mut Box<[Value]>,
    pub funcs: &'a Box<[Script]>,
    pub ctx: &'a mut T,
    pub sys_funcs: &'a [SysFn<T>],
}