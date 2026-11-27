use core::fmt;
use std::{cmp::max, collections::{BTreeSet, HashMap, HashSet}, fmt::format, process::id};

use crate::{code_reader::{parse_function_call, parse_function_define}, default_compiler_ml::CompileError::{FunctionDefineError, NotAllowedHere, TypeMismatch, UnknownCommand, VariableDoesntDefined}, ml_script::Script, ml_structs::{Command::{self, If, JumpUp, Return, SystemCall, While}, Expr, ExprParser, Op, Type, Value, contain_var, infer_type, replace_var}};

pub fn parse_code(commands_text: Vec<(String, usize)>, parser: &ExprParser, system_calls: &SystemCalls) -> Result<Script, CompileError> {
    let mut compiled_scripts: Vec<Script> = Vec::new();
    let mut funcs: HashMap<String, usize> = HashMap::new();

    let mut global_vars: HashMap<String, usize> = HashMap::new();
    let mut global_types: Vec<Type> = Vec::new();
    let mut events: HashMap<String, Vec<usize>> = HashMap::new();

    let main_script = parse_script(
        commands_text,
        &parser,
        None,
        Vec::new(),
        0,
        &mut funcs,
        &mut events,
        &mut HashSet::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        system_calls,
        &mut global_vars,
        &mut global_types,
        &mut compiled_scripts
    );

    match main_script {
        Ok(mut ms) => {
            ms.funcs = compiled_scripts.into_boxed_slice();
            ms.funcs_map = funcs;
            ms.events = events;
            ms.global_variables = vec![Value::Null; global_vars.len()].into_boxed_slice();
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
    events: &mut HashMap<String, Vec<usize>>,
    pure_funcs: &mut HashSet<String>,
    funcs_args: &mut Vec<Vec<Type>>,
    funcs_types: &mut Vec<Type>,
    system_calls: &SystemCalls,
    global_vars: &mut HashMap<String, usize>,
    global_types: &mut Vec<Type>,
    compiled_scripts: &mut Vec<Script>) -> Result<Script, CompileError>
{
    let mut compile_flags: Vec<String> = Vec::new();

    let mut variables: HashMap<String, usize> = HashMap::new();
    let mut free_ids: HashSet<usize> = HashSet::new();
    let mut types: Vec<Type> = Vec::new();

    for (var_name, var_type) in &args {

        if variables.contains_key(&var_name.to_string()) {
            return Err(CompileError::VariableAlreadyDefined(format!("Init variable '{var_name}'"), start_i));
        }

        let id = variables.len();
        variables.insert(var_name.to_string(), id);
        types.push(var_type.clone());
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut skips: usize = 0;
    let mut block_stack: Vec<Option<usize>> = Vec::new(); // для каждого открытого `{`: индекс If в commands, если блок принадлежит if
    let mut pending_if: Option<usize> = None;
    let mut last_if_jump: Option<usize> = None;
    let mut conditions_jump_indexes: Vec<usize> = Vec::new();

    let mut chains: Vec<Vec<usize>> = Vec::new();  // по списку JumpUp на каждый открытый if
    let mut closed_if: Option<usize> = None;       // If, тело которого только что закрылось
    let mut chain_end = false;                     // только что закрылся else
    let mut pending_else = false;
    let mut block_stack: Vec<(Option<usize>, bool)> = Vec::new();  // (владелец блока, это else?)

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

        let prev_if = closed_if.take();
        let prev_end = std::mem::take(&mut chain_end);
        let continues = matches!(space_splited[0], "else" | "elif");

        if !continues && (prev_if.is_some() || prev_end) {
            // цепочка закончилась: все её прыжки ведут сюда
            let end = commands.len() as u32;
            for j in chains.pop().unwrap_or_default() {
                commands[j] = JumpUp(end);
            }
        }

        // Подсказки компилятору @
        if line.starts_with("@") {
            if in_func != None {
                return Err(NotAllowedHere("You cant use compile flags in functions".to_string(), offset_line_num));
            }

            let mut def_found = false;
            for check_i in i+1..commands_text.len() {
                let check_line = &commands_text[check_i].0;
                if check_line.starts_with("def ") {
                    def_found = true;
                    break;
                }

                if check_line.starts_with("@") || check_line.is_empty() {
                    continue;
                }
                else {
                    break;
                }
            }
            if def_found {
                compile_flags.push(line.to_string());
            }
            else {
                return Err(NotAllowedHere("You can use compile flags only before functions".to_string(), offset_line_num));
            }
        }

        // Объявление переменной float a = 10.5;
        else if parser.types_map.contains_key(space_splited[0]) && space_splited.len() >= 2 && eq_splited.len() > 1 {

            let var_type = parser.types_map.get(space_splited[0]).unwrap().clone();
            let var_name = &space_splited[1..].join(" ").split("=").next().unwrap().trim().to_string();

            let var_id = match variables.get(var_name) {
                Some(_) => {
                    return Err(CompileError::VariableAlreadyDefined(line.to_string(), offset_line_num));
                },
                None => {
                    match free_ids.iter().next().cloned() {
                        Some(new_id) => {
                            free_ids.remove(&new_id);
                            variables.insert(var_name.to_string(), new_id);
                            types[new_id] = var_type.clone();
                            new_id
                        },
                        None => {
                            let new_id = variables.len();
                            variables.insert(var_name.to_string(), new_id);
                            types.push(var_type.clone());
                            new_id
                        }
                    }
                }
            };

            let expr_text = eq_splited[1..].join("=");
            let expr = match parser.parse_tokens_text(expr_text.clone(), &variables, global_vars, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let got_type = infer_type(&expr, &types, global_types, funcs_types, &system_calls.return_types);
            if got_type != var_type && got_type != Type::Null {
                return Err(TypeMismatch(format!("{line}, expected {var_type:?} but got {got_type:?}"), offset_line_num));
            }

            commands.push(Command::Var(var_id as u32, expr));

        }

        // Уничтожение переменных destroy var_name
        // Ручаня оптимизация размера стека. Просто подсказка компилятору
        else if space_splited[0] == "destroy" {
            let var_name = space_splited[1..].join(" ");
            let v_id = match variables.get(&var_name) {
                Some(id) => id,
                None => return Err(VariableDoesntDefined(var_name, offset_line_num))
            };
            free_ids.insert(*v_id);
            variables.remove(&var_name);
        }

        // Объявление глобальной переменной global float a = 10.5;
        else if space_splited[0] == "global" && parser.types_map.contains_key(space_splited[1]) && space_splited.len() >= 3 && eq_splited.len() > 1 {

            if in_func != None {
                return Err(NotAllowedHere(format!("You cant define global variables in function"), offset_line_num));
            }

            let var_type = parser.types_map.get(space_splited[1]).unwrap().clone();
            let var_name = &space_splited[2..].join(" ").split("=").next().unwrap().trim().to_string();

            let var_id = match global_vars.get(var_name) {
                Some(id) => {
                    return Err(CompileError::VariableAlreadyDefined(line.to_string(), offset_line_num));
                },
                None => {
                    let new_id = global_vars.len();
                    global_vars.insert(var_name.to_string(), new_id);
                    new_id
                }
            };
            global_types.push(var_type.clone());

            let expr_text = eq_splited[1..].join("=");
            let expr = match parser.parse_tokens_text(expr_text.clone(), &variables, global_vars, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let got_type = infer_type(&expr, &types, global_types, funcs_types, &system_calls.return_types);
            if got_type != var_type && got_type != Type::Null {
                return Err(TypeMismatch(format!("{line}, expected {var_type:?} but got {got_type:?}"), offset_line_num));
            }

            //println!("global var parsed! name = {var_name}, type = {var_type:?}, len = {}", global_types.len());
            commands.push(Command::GlobalVar(var_id as u32, expr));

        }

        // return Type;
        else if space_splited.len() >= 2 && space_splited[0] == "return" {
            let expr_text = space_splited[1..].join(" ");
            let expr = match parser.parse_tokens_text(expr_text.clone(), &variables, global_vars, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let got_type = infer_type(&expr, &types, global_types, funcs_types, &system_calls.return_types);
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
                let expr = match parser.parse_tokens_text(arg_text.to_string(), &variables, global_vars, funcs, funcs_args, system_calls) {
                    Ok(expr) => expr,
                    Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
                };
                let need_type = funcs_args[func_id][i].clone();
                let got_type = infer_type(&expr, &types, global_types, funcs_types, &system_calls.return_types);

                if got_type != need_type && got_type != Type::Null {
                    return Err(TypeMismatch(format!("{line}, expected {need_type:?} but got {got_type:?}"), offset_line_num));
                }

                args.push(expr);
            }

            let need_args = funcs_args[func_id].len();
            if args.len() != need_args {
                return Err(FunctionDefineError(format!("expected {} but got {} arguments", need_args, func_data.params.len()), offset_line_num));
            }

            if pure_funcs.contains(bracket_splited[0]) {
                continue;
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
                let parsed_expr = match parser.parse_tokens_text(check_arg.to_string(), &variables, global_vars, funcs, funcs_args, system_calls) {
                    Ok(e) => e,
                    Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
                };
                let got_type = infer_type(&parsed_expr, &types, global_types, funcs_types, &system_calls.return_types);
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
            let expr = match parser.parse_tokens_text(expr_text, &variables, global_vars, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };
            commands.push(If(expr, 0));
            pending_if = Some(commands.len() - 1);
            chains.push(Vec::new());   // новая цепочка: сюда попадут прыжки её else/elif
        }
        else if space_splited[0] == "else" || space_splited[0] == "elif" {
            let if_idx = match prev_if {
                Some(idx) => idx,
                None => return Err(NotAllowedHere("else/elif can be used only right after if".to_string(), offset_line_num)),
            };

            // прыжок в конец цепочки, адрес проставится позже
            commands.push(JumpUp(0));
            chains.last_mut().unwrap().push(commands.len() - 1); // None value ОШИБКА

            // ложное условие предыдущего блока должно перепрыгнуть и этот прыжок
            if let If(_, n) = &mut commands[if_idx] {
                *n += 1;
            }

            if space_splited[0] == "elif" {
                let expr_text = space_splited[1..].join(" ");
                let expr = match parser.parse_tokens_text(expr_text, &variables, global_vars, funcs, funcs_args, system_calls) {
                    Ok(expr) => expr,
                    Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
                };
                commands.push(If(expr, 0));
                pending_if = Some(commands.len() - 1);
            } else {
                pending_else = true;
            }
        }

        // Условия if a == true {..}
        else if space_splited.len() > 1 && space_splited[0] == "while" {
            let expr_text = space_splited[1..].join(" ");
            let expr = match parser.parse_tokens_text(expr_text, &variables, global_vars, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };
            commands.push(While(expr, 0));
            pending_if = Some(commands.len() - 1);
        }

        else if space_splited.len() >= 1 && space_splited[0] == "def" {

            if in_func != None {
                return Err(NotAllowedHere(format!("You cant define functions in function"), offset_line_num));
            }

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
            funcs.insert(parsed_func_data.name.clone(), f_id);
            funcs_types.push(return_type.clone());
            funcs_args.push(need_args);

            for flag in compile_flags.iter() {
                if flag == "@pure" {
                    pure_funcs.insert(parsed_func_data.name.to_string());
                }
                else if flag.starts_with("@on\"") && flag.ends_with("\"") {
                    let event_name = &flag.split("\"").collect::<Vec<&str>>()[1].to_string();
                    match events.get_mut(event_name) {
                        Some(vec) => vec.push(f_id),
                        None => {
                            events.insert(event_name.to_string(), [f_id].to_vec());
                        }
                    };
                }
            }
            compile_flags.clear();

            let func_commands = commands_text[i+1..=i + jump_size].to_vec();
            let script = match parse_script(func_commands, parser, Some(return_type), start_args, i, funcs, events, pure_funcs, funcs_args, funcs_types, system_calls, global_vars, global_types, compiled_scripts) {
                Ok(f) => f,
                Err(e) => return  Err(e),
            };

            // Добавляем скомпилированную функцию
            compiled_scripts.push(script);
            skips = jump_size + 1;

            continue;

        }
        else if line.trim() == "{" {
            block_stack.push((pending_if.take(), std::mem::take(&mut pending_else)));
        }
        else if line.trim() == "}" {
            if let Some((owner, is_else)) = block_stack.pop() {
                if is_else {
                    chain_end = true;
                }
                if let Some(idx) = owner {
                    if matches!(commands[idx], While(..)) {
                        commands.push(JumpUp(idx as u32));
                    } else {
                        closed_if = Some(idx);   // if или elif: дальше может идти else/elif
                    }
                    let body_len = (commands.len() - idx - 1) as u32;
                    match &mut commands[idx] {
                        If(_, n) | While(_, n) => *n = body_len,
                        _ => {}
                    }
                }
            }
        }
        // Присваивание переменной
        else if space_splited.len() >= 1 && eq_splited.len() >= 1 {
            let var_name = space_splited[0].to_string();

            let expr_text = eq_splited[1..].join("=");
            let expr = match parser.parse_tokens_text(expr_text, &variables, global_vars, funcs, funcs_args, system_calls) {
                Ok(expr) => expr,
                Err(e) => return Err(CompileError::ExpressionParseError(format!("({e}, {line})"), offset_line_num)),
            };

            let (var_id, is_global) = match variables.get(&var_name) {
                Some(id) => (*id, false),
                None => {
                    match global_vars.get(&var_name) {
                        Some(id) => (*id, true),
                        None => return Err(VariableDoesntDefined(line.to_string(), offset_line_num)),
                    }
                }
            };

            let need_type = match is_global {
                false => types[var_id].clone(),
                true => global_types[var_id].clone(),
            };
            let got_type = infer_type(&expr, &types, global_types, funcs_types, &system_calls.return_types);

            if need_type != got_type && got_type != Type::Null {
                return Err(TypeMismatch(format!("{line}, expected {need_type:?} but got {got_type:?}"), offset_line_num));
            }

            match is_global {
                false => commands.push(Command::Var(var_id as u32, expr)),
                true => commands.push(Command::GlobalVar(var_id as u32, expr)),
            };
        }
        else {
            return Err(UnknownCommand(format!("{line}"), offset_line_num));
        }

    }

    if closed_if.is_some() || chain_end {
        let end = commands.len() as u32;
        for j in chains.pop().unwrap_or_default() {
            commands[j] = JumpUp(end);
        }
    }

    optimizate_jumps(&mut commands);
    let script = Script {
        total_variables: optimizate_stack(&mut commands, types.len(), args.len()),
        global_variables: Vec::new().into_boxed_slice(),
        funcs_map: HashMap::new(),
        events: HashMap::new(),
        funcs: Vec::new().into_boxed_slice(),
        commands: commands.into_boxed_slice(),
    };

    Ok(script)
}
fn optimizate_stack(commands: &mut Vec<Command>, frame_size: usize, params: usize) -> usize {

    let mut vars_history: HashMap<u32, usize> = HashMap::new();

    for i in 0..params {
        vars_history.insert(i as u32, 0);
    }
    
    let mut line_max: usize = 0;
    for (line, command) in commands.iter().enumerate() {
        let corrected_line = max(line, line_max);
        match command {
            Command::Var(v_id, expr) => {
                for (v_id, last_line) in vars_history.iter_mut() {
                    if contain_var(expr, *v_id as usize) {
                        *last_line = corrected_line;
                    }
                }
                vars_history.insert(*v_id, corrected_line);
            },
            Command::Call(_, params) => {
                for arg in params.iter() {
                    for (v_id, last_line) in vars_history.iter_mut() {
                        if contain_var(arg, *v_id as usize) {
                            *last_line = corrected_line;
                        }
                    }
                }
            },
            Command::SystemCall(_, params) => {
                for arg in params.iter() {
                    for (v_id, last_line) in vars_history.iter_mut() {
                        if contain_var(arg, *v_id as usize) {
                            *last_line = corrected_line;
                        }
                    }
                }
            },
            Command::GlobalVar(_, expr) => {
                for (v_id, last_line) in vars_history.iter_mut() {
                    if contain_var(expr, *v_id as usize) {
                        *last_line = corrected_line;
                    }
                }
            },
            Command::If(expr, _) => {
                for (v_id, last_line) in vars_history.iter_mut() {
                    if contain_var(expr, *v_id as usize) {
                        *last_line = corrected_line;
                    }
                }  
            },
            Command::While(expr, js) => {
                line_max = max(line_max, line + *js as usize);
                for (v_id, last_line) in vars_history.iter_mut() {
                    if contain_var(expr, *v_id as usize) {
                        *last_line = corrected_line;
                    }
                }  
            },
            Command::Return(expr) => {
                for (v_id, last_line) in vars_history.iter_mut() {
                    if contain_var(expr, *v_id as usize) {
                        *last_line = corrected_line;
                    }
                }  
            },
            Command::JumpUp(_) => {},
        }
    }

    let mut replace_to: HashMap<usize, usize> = HashMap::new();
    let mut free_ids: BTreeSet<usize> = BTreeSet::new();   // детерминированно, берём наименьший
    let mut seen: HashSet<usize> = HashSet::new();         // id, которые уже объявлены

    for (line, command) in commands.iter_mut().enumerate() {
        match command {
            Command::Var(v_id, expr) => {
                remap_vars(expr, &replace_to);                 // сначала читаем старые слоты
                let orig = *v_id as usize;
                if let Some(&to) = replace_to.get(&orig) {
                    *v_id = to as u32;                         // присваивание уже переехавшей переменной
                } else if seen.insert(orig) {                  // первое появление = объявление
                    if let Some(id) = free_ids.pop_first() {
                        replace_to.insert(orig, id);
                        *v_id = id as u32;
                    }
                }                                              // иначе: переменная осталась в своём слоте
            }
            Command::GlobalVar(_, expr) | Command::If(expr, _)
            | Command::While(expr, _) | Command::Return(expr) => remap_vars(expr, &replace_to),
            Command::Call(_, params) | Command::SystemCall(_, params) => {
                for e in params.iter_mut() { remap_vars(e, &replace_to); }
            }
            Command::JumpUp(_) => {}
        }

        for (var_id, last_line) in vars_history.iter() {
            if *last_line == line {
                let orig = *var_id as usize;
                free_ids.insert(replace_to.get(&orig).copied().unwrap_or(orig));   // фактический слот
            }
        }
    }

    let mut frame_size = frame_size;
    for command in commands.iter() {
        if let Command::Var(id, _) = command {
            frame_size = frame_size.max(*id as usize + 1);
        }
    }
    frame_size

}

fn optimizate_jumps(commands: &mut Vec<Command>) {
    for i in 0..commands.len() {
        let mut target = match commands[i] {
            Command::JumpUp(t) => t as usize,
            _ => continue,
        };

        // идём по цепочке прыжков; лимит защищает от циклов вида a -> b -> a
        for _ in 0..commands.len() {
            match commands.get(target) {
                Some(Command::JumpUp(next)) if *next as usize != target => target = *next as usize,
                _ => break,
            }
        }

        commands[i] = Command::JumpUp(target as u32);
    }
}

fn remap_vars(expr: &mut Expr, map: &HashMap<usize, usize>) {
    match expr {
        Expr::Variable(id) => { if let Some(to) = map.get(&(*id as usize)) { *id = *to as u32; } }
        Expr::Neg(e) | Expr::As(e, _) => remap_vars(e, map),
        Expr::Binary(a, _, b) => { remap_vars(a, map); remap_vars(b, map); }
        Expr::Func(_, args) | Expr::SysFunc(_, args) => for a in args { remap_vars(a, map); },
        Expr::Value(_) | Expr::GlobalVariable(_) => {}
    }
}

#[derive(Debug)]
pub enum CompileError {
    ExpressionParseError(String, usize),
    UnknownCommand(String, usize),
    VariableAlreadyDefined(String, usize),
    VariableDoesntDefined(String, usize),
    TypeMismatch(String, usize),
    FunctionDefineError(String, usize),
    NotAllowedHere(String, usize),
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
            Self::NotAllowedHere(msg, line)  => write!(f, "line {line}: Not allowed here: {msg}"),
        }
    }
}