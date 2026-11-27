use crate::{CompileError, SysFn, SystemCalls, Type, Value, code_reader::{self, LoadError}, default_compiler_ml, ml_script::{RunTimeCtx, Script}, ml_structs::{ExprParser, Op}};

pub struct Engine<T> {
    parser: ExprParser,
    sys_calls: SystemCalls,
    funcs: Vec<SysFn<T>>,
}
impl<T> Engine<T> {
    pub fn new() -> Self {
        Self {
            parser: ExprParser::new(),
            sys_calls: SystemCalls::new(),
            funcs: Vec::new(),
        }
    }
    pub fn register(&mut self, name: &str, params: &[Type], return_type: Type, func: SysFn<T>) {
        let old_len = self.sys_calls.names.len();
        self.sys_calls.include(name, params.to_vec(), return_type);
        if self.sys_calls.names.len() != old_len {
            self.funcs.push(func);
        }
    }

    pub fn read_code(&self, path: &str) -> Result<Vec<(String, usize)>, LoadError> {
        match code_reader::load_script(path) {
            Ok(c) => Ok(c),
            Err(e) => {
                Err(e)
            },
        }
    }

    pub fn compile(&self, code_text: Vec<(String, usize)>) -> Result<Script, CompileError> {
        match default_compiler_ml::parse_code(code_text, &self.parser, &self.sys_calls) {
            Ok(s) => Ok(s),
            Err(e) => Err(e),
        }
    }

    pub fn reset_globals(&self, script: &mut Script) {
        for i in 0..script.global_variables.len() {
            script.global_variables[i] = Value::Null;
        }
    }
    pub fn script_run(&self, script: &mut Script, ctx: &mut T) -> Value {
        script.main_run(ctx, &self.funcs)
    }
    pub fn script_call(&self, script: &mut Script, name: &str, params: &[Value], ctx: &mut T) -> Option<Value> {
        let f_id = match script.funcs_map.get(name) {
            Some(id) => id,
            None => return None,
        };

        let mut global_vars = std::mem::take(&mut script.global_variables);
        let sub_script = &script.funcs[*f_id];

        let mut stack = vec![Value::Null; sub_script.total_variables];
        for (i, val) in params.iter().enumerate() {
            stack[i] = val.clone();
        }

        let mut ctx = RunTimeCtx {
            stack: &mut stack,
            global_vars: &mut global_vars,
            funcs: &script.funcs,
            ctx: ctx,
            sys_funcs: &self.funcs,
        };

        let result = Some(sub_script.run::<T>(&mut ctx, 0));
        script.global_variables = std::mem::take(&mut global_vars);
        result
    }
    pub fn script_event(&self, script: &mut Script, name: &str, params: &[Value], ctx: &mut T) {
        let f_id_vec = match script.events.get(name) {
            Some(id) => id,
            None => return,
        };

        let mut global_vars = std::mem::take(&mut script.global_variables);
        for f_id in f_id_vec {
            let sub_script = &script.funcs[*f_id];
    
            let mut stack = vec![Value::Null; sub_script.total_variables];
            for (i, val) in params.iter().enumerate() {
                stack[i] = val.clone();
            }
    
            let mut ctx = RunTimeCtx {
                stack: &mut stack,
                global_vars: &mut global_vars,
                funcs: &script.funcs,
                ctx: ctx,
                sys_funcs: &self.funcs,
            };
    
            let result = Some(sub_script.run::<T>(&mut ctx, 0));
            script.global_variables = std::mem::take(&mut global_vars);
        }
    }
}