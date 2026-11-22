use core::fmt;
use std::{collections::HashMap, fmt::format};

use crate::{code_reader::{parse_function_call, parse_function_define}, default_compiler_ml::CompileError::{FunctionDefineError, TypeMismatch, UnknownCommand, VariableDoesntDefined}, ml_script::Script, ml_structs::{Command::{self, If, JumpUp, Return, SystemCall, While}, Expr, ExprParser, Op, Type, Value, infer_type}};

pub fn parse_code(commands_text: Vec<(String, usize)>, parser: &ExprParser, system_calls: &SystemCalls) -> Result<Script, CompileError> {
    let mut compiled_scripts: Vec<Script> = Vec::new();
    let mut funcs: HashMap<String, usize> = HashMap::new();

    let main_script = parse_script(
        commands_text,
        &parser,
        None,
        Vec::new(),
        0,
        &mut funcs,
        &mut Vec::new(),
        &mut Vec::new(),
        system_calls,
        &mut compiled_scripts
    );

    match main_script {
        Ok(mut ms) => {
            ms.funcs = compiled_scripts.into_boxed_slice();
            ms.funcs_map = funcs;
            Ok(ms)
        },
        Err(e) => Err(e),
    }
}

pub struct SystemCalls {
    pub names_map: HashMap<String, usize>,
    pub names: Vec<String>,
    pub types: Vec<Vec<Type>>,
    pub return_types: Vec<Type>,
}
impl SystemCalls {
    pub fn new() -> Self {
        Self {
            names_map: HashMap::new(),
            names: Vec::new(),
            types: Vec::new(),
            return_types: Vec::new(),
        }
    }
    pub fn include(&mut self, name: &str, args: Vec<Type>, return_type: Type) {
        if self.names_map.contains_key(name) {
            return;
        }
        self.names_map.insert(name.to_string(), self.names.len());
        self.names.push(name.to_string());
        self.types.push(args);
        self.return_types.push(return_type);
    }
}

fn parse_script(
    commands_text: Vec<(String, usize)>,
    parser: &ExprParser,
    in_func: Option<Type>,
    args: Vec<(String, Type)>,
    start_i: usize,
    funcs: &mut HashMap<String, usize>,
    funcs_args: &mut Vec<Vec<Type>>,
    funcs_types: &mut Vec<Type>,
    system_calls: &SystemCalls,
    compiled_scripts: &mut Vec<Script>) -> Result<Script, CompileError>
{

    let mut variables: HashMap<String, usize> = HashMap::new();
    let mut types: Vec<Type> = Vec::new();

    for (var_name, var_type) in args {

        if variables.contains_key(&var_name) {
            return Err(CompileError::VariableAlreadyDefined(format!("Init variable '{var_name}'"), start_i));
        }

        let id = variables.len();
        variables.insert(var_name.to_string(), id);
        types.push(var_type);
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut skips: usize = 0;
    let mut block_stack: Vec<Option<usize>> = Vec::new(); // для каждого открытого `{`: индекс If в commands, если блок принадлежит if
    let mut pending_if: Option<usize> = None;

    for (i, (line, line_num)) in commands_text.iter().enumerate() {

        if skips > 0 {
            skips -= 1;
            continue;
        }

        let space_splited: Vec<&str> = line.split_whitespace().collect();
        let eq_splited: Vec<&str> = line.split('=').collect();
        let bracket_splited: Vec<&str> = line.split('(').collect();

        let offset_line_num = line_num + 1 + start_i;

        if space_splited.is_empty() {
            continue;
        }

        // Объявление переменной float a = 10.5;
        if parser.types_map.contains_key(space_splited[0]) && space_splited.len() >= 2 && eq_splited.len() > 1 {

            let var_type = parser.types_map.get(space_splited[0]).unwrap().clone();
            let var_name = space_splited[1];

            let var_id = match variables.get(var_name) {
                Some(id) => {
                    return Err(CompileError::VariableAlreadyDefined(line.to_string(), offset_line_num));
                },
                None => {
                    let new_id = variables.len();
                    variables.insert(var_name.to_string(), new_id);
                    new_id
                }
            };

            let expr_text = eq_splited[1..].join("=");
            let expr = match parser.parse_tokens_text(expr_text.clone(), &variables, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let got_type = infer_type(&expr, &types, funcs_types, &system_calls.return_types);
            if got_type != var_type && got_type != Type::Null {
                return Err(TypeMismatch(format!("{line}, expected {var_type:?} but got {got_type:?}"), offset_line_num));
            }

            types.push(var_type);
            commands.push(Command::Var(var_id as u32, expr));

        }

        // return Type;
        else if space_splited.len() >= 2 && space_splited[0] == "return" {
            let expr_text = space_splited[1..].join(" ");
            let expr = match parser.parse_tokens_text(expr_text.clone(), &variables, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let got_type = infer_type(&expr, &types, funcs_types, &system_calls.return_types);
            match in_func {
                Some(ref t) => {
                    if got_type != *t && got_type != Type::Null {
                        return Err(TypeMismatch(format!("{line}, expected {t:?} but got {got_type:?}"), offset_line_num));
                    }
                    commands.push(Return(expr));
                },
                None => {
                    commands.push(Return(expr));
                }
            }
        }

        // Вызов функции
        else if bracket_splited.len() > 0 && funcs.contains_key(bracket_splited[0]) {
            let func_id = *funcs.get(bracket_splited[0]).unwrap();

            let func_data = match parse_function_call(line) {
                Ok(f) => f,
                Err(e) => return Err(FunctionDefineError(format!("{line} : {e}"), offset_line_num)),
            };

            let mut args: Vec<Expr> = Vec::new();
            
            for (i, arg_text) in func_data.params.iter().enumerate() {
                let expr = match parser.parse_tokens_text(arg_text.to_string(), &variables, funcs, funcs_args, system_calls) {
                    Ok(expr) => expr,
                    Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
                };
                let need_type = funcs_args[func_id][i].clone();
                let got_type = infer_type(&expr, &types, funcs_types, &system_calls.return_types);

                if got_type != need_type && got_type != Type::Null {
                    return Err(TypeMismatch(format!("{line}, expected {need_type:?} but got {got_type:?}"), offset_line_num));
                }

                args.push(expr);
            }

            let need_args = funcs_args[func_id].len();
            if args.len() != need_args {
                return Err(FunctionDefineError(format!("expected {} but got {} arguments", need_args, func_data.params.len()), offset_line_num));
            }

            commands.push(Command::Call(func_id as u32, args.into_boxed_slice()));

        }

        // Вызов системной функции
        else if bracket_splited.len() > 0 && system_calls.names.contains(&bracket_splited[0].to_string()) {

            let mut func_i: usize = 0;
            for (check_i, name) in system_calls.names.iter().enumerate() {
                if name == bracket_splited[0] {
                    func_i = check_i;
                    break;
                }
            }

            let name = &system_calls.names[func_i];
            let args = &system_calls.types[func_i];
            let return_type = system_calls.return_types[func_i].clone();

            let parsed_func = match parse_function_call(line) {
                Ok(f) => f,
                Err(e) => return Err(FunctionDefineError(format!("{line} : {e}"), offset_line_num)),
            };

            if parsed_func.params.len() != args.len() {
                return Err(FunctionDefineError(format!("expected {} but got {} arguments", args.len(), parsed_func.params.len()), offset_line_num));
            }

            let mut parsed_args: Vec<Expr> = Vec::new();
            for (i, check_arg) in parsed_func.params.iter().enumerate() {
                let parsed_expr = match parser.parse_tokens_text(check_arg.to_string(), &variables, funcs, funcs_args, system_calls) {
                    Ok(e) => e,
                    Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
                };
                let got_type = infer_type(&parsed_expr, &types, funcs_types, &system_calls.return_types);
                let need_type = args[i].clone();
                if got_type != need_type && got_type != Type::Null {
                    return Err(TypeMismatch(format!("{line}, expected {need_type:?} but got {got_type:?}"), offset_line_num));
                }

                parsed_args.push(parsed_expr);
            }

            commands.push(SystemCall(func_i as u32, parsed_args.into_boxed_slice()));

        }

        // Условия if a == true {..}
        else if space_splited.len() > 1 && space_splited[0] == "if" {
            let expr_text = space_splited[1..].join(" ");
            let expr = match parser.parse_tokens_text(expr_text, &variables, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };
            commands.push(If(expr, 0));
            pending_if = Some(commands.len() - 1);
        }

        // Условия if a == true {..}
        else if space_splited.len() > 1 && space_splited[0] == "while" {
            let expr_text = space_splited[1..].join(" ");
            let expr = match parser.parse_tokens_text(expr_text, &variables, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };
            commands.push(While(expr, 0));
            pending_if = Some(commands.len() - 1);
        }

        else if space_splited.len() >= 1 && space_splited[0] == "def" {
            let func_data = space_splited[1..].join(" ");
            let parsed_func_data = match parse_function_define(&func_data) {
                Ok(f) => f,
                Err(e) => return Err(FunctionDefineError(format!("{line} : {e}"), offset_line_num))
            };

            if funcs.contains_key(&parsed_func_data.name) {
                return Err(FunctionDefineError(format!("{line} : function already defined"), offset_line_num));
            }

            let return_type = match parser.types_map.get(&parsed_func_data.ret.unwrap_or("None".to_string())) {
                Some(t) => t.clone(),
                None => Type::Null,
            };

            let mut jump_size: usize = 0;
            let mut brackets: i32 = 1;

            for check_i in i+2..commands_text.len() {
                let check_line = &commands_text[check_i].0;
                if check_line == "{" {
                    brackets += 1;
                    jump_size += 1;
                    continue;
                }
                else if check_line == "}" {
                    brackets -= 1;
                    jump_size += 1;
                    continue;
                }
                if brackets == 0 {
                    break;
                }

                jump_size += 1;
                
            }

            let mut need_args: Vec<Type> = Vec::new();
            let mut start_args: Vec<(String, Type)> = Vec::new();

            for (type_text, name) in parsed_func_data.params {
                let arg_type = match parser.types_map.get(&type_text) {
                    Some(t) => t,
                    None => return Err(FunctionDefineError(format!("{line} : type '{type_text}' incorrect"), offset_line_num)),
                };
                need_args.push(arg_type.clone());
                start_args.push((name, arg_type.clone()));
            }

            // Заранее добавляем функцию еще до компиляции чтобы функция могла вызывать сама себя
            let f_id = funcs.len();
            funcs.insert(parsed_func_data.name, f_id);
            funcs_types.push(return_type.clone());
            funcs_args.push(need_args);

            let func_commands = commands_text[i+1..=i + jump_size].to_vec();
            let script = match parse_script(func_commands, parser, Some(return_type), start_args, i, funcs, funcs_args, funcs_types, system_calls, compiled_scripts) {
                Ok(f) => f,
                Err(e) => return  Err(e),
            };

            // Добавляем скомпилированную функцию
            compiled_scripts.push(script);
            skips = jump_size + 1;

            continue;

        }
        else if line.trim() == "{" {
            block_stack.push(pending_if.take());
        }
        else if line.trim() == "}" {
            if let Some(Some(idx)) = block_stack.pop() {
                // Для while в конец тела добавляем прыжок обратно к самому while
                if matches!(commands[idx], While(..)) {
                    commands.push(JumpUp(idx as u32));
                }
                // Число команд в теле. У while сюда входит и JumpUp,
                // поэтому при ложном условии пропускается тело вместе с ним
                let body_len = (commands.len() - idx - 1) as u32;
                match &mut commands[idx] {
                    If(_, n) | While(_, n) => *n = body_len,
                    _ => {}
                }
            }
        }
        // Присваивание переменной
        else if space_splited.len() >= 1 && eq_splited.len() >= 1 {
            let var_name = space_splited[0].to_string();

            let expr_text = eq_splited[1..].join("=");
            let expr = match parser.parse_tokens_text(expr_text, &variables, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let var_id = match variables.get(&var_name) {
                Some(id) => *id,
                None => {
                    return Err(VariableDoesntDefined(line.to_string(), offset_line_num));
                }
            };

            let need_type = types[var_id].clone();
            let got_type = infer_type(&expr, &types, funcs_types, &system_calls.return_types);

            if need_type != got_type && got_type != Type::Null {
                return Err(TypeMismatch(format!("{line}, expected {need_type:?} but got {got_type:?}"), offset_line_num));
            }

            commands.push(Command::Var(var_id as u32, expr));
        }
        else {
            return Err(UnknownCommand(format!("{line}"), offset_line_num));
        }

    }

    println!("len = {}", variables.len());

    let script = Script {
        total_variables: variables.len(),
        funcs_map: HashMap::new(),
        funcs: Vec::new().into_boxed_slice(),
        commands: commands.into_boxed_slice(),
    };

    Ok(script)
}

#[derive(Debug)]
pub enum CompileError {
    ExpressionParseError(String, usize),
    UnknownCommand(String, usize),
    VariableAlreadyDefined(String, usize),
    VariableDoesntDefined(String, usize),
    TypeMismatch(String, usize),
    FunctionDefineError(String, usize),
}
impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpressionParseError(msg, line) => write!(f, "line {line}: expression parse error: {msg}"),
            Self::UnknownCommand(cmd, line)       => write!(f, "line {line}: unknown command '{cmd}'"),
            Self::VariableAlreadyDefined(v, line) => write!(f, "line {line}: variable '{v}' alredy defined"),
            Self::VariableDoesntDefined(v, line)  => write!(f, "line {line}: variable '{v}' doesn't defined"),
            Self::TypeMismatch(msg, line)         => write!(f, "line {line}: Type mismatch: {msg}"),
            Self::FunctionDefineError(msg, line)  => write!(f, "line {line}: Function define error: {msg}"),
        }
    }
}