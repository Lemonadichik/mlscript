use std::{cmp::Reverse, collections::HashMap, fmt::Error, iter::Peekable, ops::Deref, rc::Rc, str, vec::IntoIter};

use crate::{default_compiler_ml::SystemCalls, ml_script::{Script, SysFn}};

pub type ParseResult<T> = Result<T, String>;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<String>),
}

#[derive(Clone, Debug)]
pub enum Op {
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Gr, // >
    Lr, // <
    Gt, // >=
    Lt, // <=
    Eq, // ==
    Mod, // %
    And, // and
    Or, // or
    As,
}
impl Op {
    /// Приоритет бинарного оператора. Чем больше, тем сильнее связывает.
    fn precedence(&self) -> Option<u8> {
        match self {
            Op::Or => Some(1),
            Op::And => Some(2),
            Op::Eq => Some(3),
            Op::Gr | Op::Lr | Op::Gt | Op::Lt => Some(4),
            Op::Add | Op::Sub => Some(5),
            Op::Mul | Op::Div | Op::Mod => Some(6),
            Op::As => None, // `as` разбирается отдельно, это не бинарный оператор
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Value(Value),                 // 10, false, "Hello World"
    Variable(usize),              // a, b, test (индекс переменной)
    Neg(Box<Expr>),               // -a, -(1 + 2)
    Binary(Box<Expr>, Op, Box<Expr>), // a + b * 2
    As(Box<Expr>, Type),          // x as int
    Func(usize, Vec<Expr>),
    SysFunc(usize, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    Null,
    Float, // 0.5, 0.99, 100.5
    Int, // 10, 20, 30, 10000 ..
    Bool, // true/false
    Str, // "Hello World"
}

#[derive(Debug)]
pub enum Token {
    Variable(String), // my_var[0]
    Null, // null
    Int(i64), // 10, 20, 30, 10000 ..
    Float(f64), // 0.5, 0.99, 100.5
    Bool(bool), // true/false
    Str(String), // "Hello World"
    Opertation(Op), // + - = / *
    Type(Type), // float, int, bool
    BracketIn, // (
    BracketOut, // )
    Comma, // ,
    Other(String),
}

struct TokenStream<'a> {
    tokens: Peekable<IntoIter<Token>>,
    vars: &'a HashMap<String, usize>,
    funcs: &'a HashMap<String, usize>, // имя функции -> id
    func_args: &'a [Vec<Type>],        // func_id -> типы аргументов по порядку
    sys: &'a SystemCalls,              // системные функции (имя -> id, типы аргументов)
}
impl<'a> TokenStream<'a> {
    /// Бинарные операторы с учётом приоритетов (левая ассоциативность).
    fn parse_expr(&mut self, min_prec: u8) -> ParseResult<Expr> {
        let mut lhs = self.parse_unary()?;

        loop {
            let (op, prec) = match self.tokens.peek() {
                Some(Token::Opertation(op)) => match op.precedence() {
                    Some(p) if p >= min_prec => (op.clone(), p),
                    _ => break,
                },
                _ => break,
            };

            self.tokens.next();
            let rhs = self.parse_expr(prec + 1)?;
            lhs = Expr::Binary(Box::new(lhs), op, Box::new(rhs));
        }

        Ok(lhs)
    }

    /// Унарный минус и приведение типов: `-x`, `x as int`.
    fn parse_unary(&mut self) -> ParseResult<Expr> {
        if let Some(Token::Opertation(Op::Sub)) = self.tokens.peek() {
            self.tokens.next();
            return Ok(Expr::Neg(Box::new(self.parse_unary()?)));
        }

        let mut expr = self.parse_primary()?;

        while let Some(Token::Opertation(Op::As)) = self.tokens.peek() {
            self.tokens.next();
            match self.tokens.next() {
                Some(Token::Type(ty)) => expr = Expr::As(Box::new(expr), ty),
                other => return Err(format!("после 'as' ожидался тип, найдено: {other:?}")),
            }
        }

        Ok(expr)
    }

    /// Литералы, переменные и выражения в скобках.
    fn parse_primary(&mut self) -> ParseResult<Expr> {
        match self.tokens.next() {
            Some(Token::Int(v)) => Ok(Expr::Value(Value::Int(v))),
            Some(Token::Float(v)) => Ok(Expr::Value(Value::Float(v))),
            Some(Token::Bool(v)) => Ok(Expr::Value(Value::Bool(v))),
            Some(Token::Null) => Ok(Expr::Value(Value::Null)),
            Some(Token::Str(s)) => Ok(Expr::Value(Value::Str(Rc::from(s)))),

            Some(Token::Variable(name)) => {
                // `name(` - это вызов функции
                if let Some(Token::BracketIn) = self.tokens.peek() {
                    return self.parse_call(&name);
                }
                self.vars
                    .get(&name)
                    .copied()
                    .map(Expr::Variable)
                    .ok_or_else(|| format!("неизвестная переменная `{name}`"))
            }

            Some(Token::BracketIn) => {
                let inner = self.parse_expr(1)?;
                match self.tokens.next() {
                    Some(Token::BracketOut) => Ok(inner),
                    other => Err(format!("ожидалась `)`, найдено: {other:?}")),
                }
            }

            Some(other) => Err(format!("неожиданный токен: {other:?}")),
            None => Err("выражение оборвалось".to_string()),
        }
    }

    /// Вызов функции: `name(arg1, arg2, ...)`. Токен `(` ещё не съеден.
    /// Проверяет, что функция существует, число аргументов совпадает,
    /// и (там, где тип известен на этапе парсинга) что типы подходят.
    fn parse_call(&mut self, name: &str) -> ParseResult<Expr> {
        let funcs = self.funcs;
        let func_args = self.func_args;
        let sys = self.sys;

        // Сначала ищем среди пользовательских функций, потом среди системных.
        // Дальше логика общая, отличается только откуда берутся id и типы аргументов.
        let (id, is_sys, expected): (usize, bool, &Vec<Type>) = if let Some(&id) = funcs.get(name) {
            let expected = func_args
                .get(id)
                .ok_or_else(|| format!("для функции `{name}` (id {id}) не заданы типы аргументов"))?;
            (id, false, expected)
        } else if let Some(&id) = sys.names_map.get(name) {
            (id, true, &sys.types[id])
        } else {
            return Err(format!("неизвестная функция `{name}`"));
        };

        self.tokens.next(); // `(`

        let mut args: Vec<Expr> = Vec::new();
        if let Some(Token::BracketOut) = self.tokens.peek() {
            self.tokens.next(); // пустой список: f()
        } else {
            loop {
                args.push(self.parse_expr(1)?);
                match self.tokens.next() {
                    Some(Token::Comma) => {}
                    Some(Token::BracketOut) => break,
                    other => {
                        return Err(format!(
                            "в вызове `{name}` ожидалась `,` или `)`, найдено: {other:?}"
                        ))
                    }
                }
            }
        }

        if args.len() != expected.len() {
            return Err(format!(
                "функция `{name}` принимает аргументов: {}, передано: {}",
                expected.len(),
                args.len()
            ));
        }

        for (i, (arg, want)) in args.iter().zip(expected).enumerate() {
            if let Some(got) = static_type(arg, sys) {
                if got != *want {
                    return Err(format!(
                        "аргумент {} функции `{name}`: ожидался {want:?}, получен {got:?}",
                        i + 1
                    ));
                }
            }
        }

        if is_sys {
            Ok(Expr::SysFunc(id, args))
        } else {
            Ok(Expr::Func(id, args))
        }
    }
}

/// Тип выражения, если его можно узнать без значений переменных.
/// `None` - тип неизвестен на этапе парсинга (переменная, вызов пользовательской функции),
/// такие аргументы не проверяются.
fn static_type(expr: &Expr, sys: &SystemCalls) -> Option<Type> {
    match expr {
        Expr::Value(Value::Int(_)) => Some(Type::Int),
        Expr::Value(Value::Float(_)) => Some(Type::Float),
        Expr::Value(Value::Bool(_)) => Some(Type::Bool),
        Expr::Value(Value::Str(_)) => Some(Type::Str),
        Expr::Value(Value::Null) => Some(Type::Null),
        Expr::Value(_) => None,
        Expr::SysFunc(id, _) => sys.return_types.get(*id).cloned(),
        Expr::Variable(_) | Expr::Func(..) => None,
        Expr::As(_, ty) => Some(ty.clone()),
        Expr::Neg(inner) => match static_type(inner, sys)? {
            t @ (Type::Int | Type::Float) => Some(t),
            _ => Some(Type::Null),
        },
        Expr::Binary(a, op, b) => {
            let ta = static_type(a, sys)?;
            let tb = static_type(b, sys)?;
            if ta != tb {
                return Some(Type::Null); // count_expr вернёт Null при разных типах
            }
            match (&ta, op) {
                (Type::Null, _) => Some(Type::Null),
                (_, Op::Eq | Op::Gr | Op::Lr | Op::Gt | Op::Lt) => Some(Type::Bool),
                (Type::Bool, Op::And | Op::Or) => Some(Type::Bool),
                (Type::Int, Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod) => Some(Type::Int),
                (Type::Float, Op::Add | Op::Sub | Op::Mul | Op::Div) => Some(Type::Float),
                (Type::Str, Op::Add) => Some(Type::Str),
                _ => Some(Type::Null),
            }
        }
    }
}

pub struct ExprParser {
    pub types_map: HashMap<String, Type>,
    pub operator_map: HashMap<String, Op>,
}
impl ExprParser {
    pub fn new() -> Self {
        let types_map: HashMap<String, Type> = HashMap::from([
            ("float".to_string(), Type::Float),
            ("int".to_string(), Type::Int),
            ("bool".to_string(), Type::Bool),
            ("str".to_string(), Type::Str),
        ]);
        let operator_map: HashMap<String, Op> = HashMap::from([
            ("+".to_string(),   Op::Add),
            ("-".to_string(),   Op::Sub),
            ("/".to_string(),   Op::Div),
            ("*".to_string(),   Op::Mul),
            ("%".to_string(),   Op::Mod),
            ("==".to_string(),  Op::Eq ),
            (">".to_string(),   Op::Gr ),
            ("<".to_string(),   Op::Lr ),
            (">=".to_string(),  Op::Gt ),
            ("<=".to_string(),  Op::Lt ),
            ("and".to_string(), Op::And),
            ("or".to_string(),  Op::Or ),
            ("as".to_string(),  Op::As ),
        ]);
        Self { types_map, operator_map }
    }

    pub fn tokens_sort(tokens: &mut Vec<String>) {
        tokens.sort_by_key(|s| Reverse(s.chars().count()));
    }

    /// `funcs` - имя функции -> id, `func_args[id]` - типы аргументов функции по порядку.
    /// `sys` - системные функции, привязанные разработчиком.
    pub fn parse_tokens_text(
        &self,
        scr: String,
        vars: &HashMap<String, usize>,
        funcs: &HashMap<String, usize>,
        func_args: &[Vec<Type>],
        sys: &SystemCalls,
    ) -> ParseResult<Expr> {
        let chars: Vec<char> = scr.chars().collect();

        let mut tokens_str: Vec<String> = Vec::new();
        let mut local_buffer: Vec<char> = Vec::new();
        let mut quotes: bool = false;
        let mut last_char_is_operator: bool = false;

        for char in chars {

            // Обрабатываем кавычки
            if char == '"' {
                quotes = !quotes;
                if quotes == false {
                    local_buffer.push('"'); // Ставим одну кавычку в конце чтобы по ней определять
                    tokens_str.push(local_buffer.iter().collect());
                    local_buffer.clear();
                }
                continue;
            }
            if quotes {
                local_buffer.push(char);
                continue;
            }

            // Обрабатываем круглые скобки и запятые
            if char == '(' || char == ')' || char == ',' {
                tokens_str.push(local_buffer.iter().collect());
                tokens_str.push(char.to_string());
                local_buffer.clear();

                continue;
            }

            if char == ' ' {
                if local_buffer.is_empty() {
                    continue;
                }
                tokens_str.push(local_buffer.iter().collect());
                local_buffer.clear();

                continue;
            }

            if ['+', '-', '/', '*', '!', '%', '='].contains(&char) {
                if last_char_is_operator == false {
                    tokens_str.push(local_buffer.iter().collect());
                    local_buffer.clear();
                    local_buffer.push(char);

                    last_char_is_operator = true;
                }
                else {
                    local_buffer.push(char);
                }

                continue;
            }
            else {
                if last_char_is_operator == true {
                    tokens_str.push(local_buffer.iter().collect());
                    local_buffer.clear();
                    last_char_is_operator = false;
                }
            }

            local_buffer.push(char);

        }

        tokens_str.push(local_buffer.iter().collect());
        local_buffer.clear();

        // Строковые литералы (оканчиваются на `"`) не трогаем, иначе пропадут пробелы внутри строки
        let cleaned: Vec<String> = tokens_str
            .into_iter()
            .map(|a| if a.ends_with('"') { a } else { a.replace(' ', "") })
            .filter(|a| !a.is_empty())
            .collect();

        ExprParser::tokens_to_expr(self.parse_token_text_to_tokens(cleaned), vars, funcs, func_args, sys)

    }
    pub fn parse_token_text_to_tokens(&self, tokens_text: Vec<String>) -> Vec<Token> {
        let mut tokens: Vec<Token> = Vec::new();

        for token in tokens_text {

            // Пытаемся спарсить в int
            match token.parse::<i64>() {
                Ok(v) =>{
                    tokens.push(Token::Int(v));
                    continue;
                },
                _ => {},
            };

            // Пытаемся спарсить в float
            match token.parse::<f64>() {
                Ok(v) =>{
                    tokens.push(Token::Float(v));
                    continue;
                },
                _ => {},
            };

            // Пытаемя спарсить в bool
            if token == "True" {
                tokens.push(Token::Bool(true));
                continue;
            }
            if token == "False" {
                tokens.push(Token::Bool(false));
                continue;
            }

            // Пытаемя спарсить в null
            if token == "None" {
                tokens.push(Token::Null);
                continue;
            }

            if token == "(" {
                tokens.push(Token::BracketIn);
                continue;
            }
            if token == ")" {
                tokens.push(Token::BracketOut);
                continue;
            }
            if token == "," {
                tokens.push(Token::Comma);
                continue;
            }

            match self.operator_map.get(&token) {
                Some(op) =>{
                    tokens.push(Token::Opertation(op.clone()));
                    continue;
                },
                _ => {}
            }
            match self.types_map.get(&token) {
                Some(var_type) =>{
                    tokens.push(Token::Type(var_type.clone()));
                    continue;
                },
                _ => {}
            }

            if token.ends_with('"') {
                let mut chars = token.chars();
                chars.next_back();
                tokens.push(Token::Str(chars.as_str().to_string()));
                continue;
            }

            tokens.push(Token::Variable(token));

        }

        return tokens;
    }
    pub fn tokens_to_expr(
        tokens: Vec<Token>,
        vars: &HashMap<String, usize>,
        funcs: &HashMap<String, usize>,
        func_args: &[Vec<Type>],
        sys: &SystemCalls,
    ) -> ParseResult<Expr> {
        let mut stream = TokenStream {
            tokens: tokens.into_iter().peekable(),
            vars,
            funcs,
            func_args,
            sys,
        };

        let expr = stream.parse_expr(1)?;

        match stream.tokens.next() {
            None => Ok(expr),
            Some(extra) => Err(format!("лишний токен после выражения: {extra:?}")),
        }
    }
}

pub fn count_expr<C>(expr: &Expr, vars: &Box<[Value]>, scripts: &Box<[Script]>, ctx: &mut C, sys_func: &[SysFn<C>]) -> Value {
    match expr {
        Expr::Value(v) => v.clone(),
        Expr::Binary(a, op, b) => {
            let v1 = count_expr(a, vars, scripts, ctx, sys_func);
            let v2 = count_expr(b, vars, scripts, ctx, sys_func);

            match v1 {
                Value::Bool(b_val1) => {
                    let b_val2 = match v2 {
                        Value::Bool(b_val2) => b_val2,
                        Value::Null => {
                            match op {
                                Op::Eq => return Value::Bool(false),
                                _ => false,
                            }
                        },
                        _ => return Value::Null,
                    };

                    match op {
                        Op::And => return Value::Bool(b_val1 && b_val2),
                        Op::Or => return Value::Bool(b_val1 || b_val2),
                        Op::Eq => return Value::Bool(b_val1 == b_val2),
                        Op::Gr => return Value::Bool(b_val1 > b_val2),
                        Op::Lr => return Value::Bool(b_val1 < b_val2),
                        Op::Gt => return Value::Bool(b_val1 >= b_val2),
                        Op::Lt => return Value::Bool(b_val1 <= b_val2),
                        _ => return Value::Null,
                    }

                },
                Value::Float(f_val1) => {
                    let f_val2 = match v2 {
                        Value::Float(f_val2) => f_val2,
                        Value::Null => {
                            match op {
                                Op::Eq => return Value::Bool(false),
                                _ => 0.0,
                            }
                        },
                        _ => return Value::Null,
                    };

                    match op {
                        Op::Add => return Value::Float(f_val1 + f_val2),
                        Op::Sub => return Value::Float(f_val1 - f_val2),
                        Op::Div => return Value::Float(f_val1 / f_val2),
                        Op::Mul => return Value::Float(f_val1 * f_val2),
                        Op::Gr => return Value::Bool(f_val1 > f_val2),
                        Op::Lr => return Value::Bool(f_val1 < f_val2),
                        Op::Gt => return Value::Bool(f_val1 >= f_val2),
                        Op::Lt => return Value::Bool(f_val1 <= f_val2),
                        Op::Eq => return Value::Bool(f_val1 == f_val2),
                        _ => return Value::Null,
                    }

                },
                Value::Int(i_val1) => {
                    let i_val2 = match v2 {
                        Value::Int(i_val2) => i_val2,
                        Value::Null => {
                            match op {
                                Op::Eq => return Value::Bool(false),
                                _ => 0,
                            }
                        },
                        _ => return Value::Null,
                    };

                    match op {
                        Op::Add => return Value::Int(i_val1 + i_val2),
                        Op::Sub => return Value::Int(i_val1 - i_val2),
                        Op::Div => return Value::Int(i_val1 / i_val2),
                        Op::Mul => return Value::Int(i_val1 * i_val2),
                        Op::Mod => return Value::Int(i_val1 % i_val2),
                        Op::Gr => return Value::Bool(i_val1 > i_val2),
                        Op::Lr => return Value::Bool(i_val1 < i_val2),
                        Op::Gt => return Value::Bool(i_val1 >= i_val2),
                        Op::Lt => return Value::Bool(i_val1 <= i_val2),
                        Op::Eq => return Value::Bool(i_val1 == i_val2),
                        _ => return Value::Null,
                    }

                },
                Value::Str(s_val1) => {
                    let s_val2 = match v2 {
                        Value::Str(s_val2) => s_val2,
                        Value::Null => {
                            match op {
                                Op::Eq => return Value::Bool(false),
                                _ => Rc::from("".to_string()),
                            }
                        },
                        _ => return Value::Null,
                    };

                    match op {
                        Op::Add => return Value::Str(Rc::from(format!("{s_val1}{s_val2}"))),
                        Op::Gr => return Value::Bool(s_val1 > s_val2),
                        Op::Lr => return Value::Bool(s_val1 < s_val2),
                        Op::Gt => return Value::Bool(s_val1 >= s_val2),
                        Op::Lt => return Value::Bool(s_val1 <= s_val2),
                        Op::Eq => return Value::Bool(s_val1 == s_val2),
                        _ => return Value::Null,
                    }

                },
                Value::Null => {
                    let n_val2 = match v2 {
                        Value::Null => true,
                        _ => false,
                    };

                    match op {
                        Op::Eq => return Value::Bool(n_val2),
                        _ => return Value::Null,
                    }

                }
                _ => return Value::Null,

            }
        },
        Expr::Neg(expr) => {
            let value = count_expr(expr, vars, scripts, ctx, sys_func);
            match value {
                Value::Float(v) => Value::Float(-v),
                Value::Int(v) => Value::Int(-v),
                _ => Value::Null
            }
        },
        Expr::As(expr, to_type) => {
            let value = count_expr(expr, vars, scripts, ctx, sys_func);

            match value {
                Value::Bool(v) => {
                    match to_type {
                        Type::Bool => Value::Bool(v),
                        Type::Float => Value::Float(v as i64 as f64),
                        Type::Int => Value::Int(v as i64),
                        Type::Str => Value::Str(Rc::from(format!("{}", v))),
                        Type::Null => Value::Null,
                    }
                },
                Value::Float(v) => {
                    match to_type {
                        Type::Bool => Value::Bool(v > 0.0),
                        Type::Float => Value::Float(v),
                        Type::Int => Value::Int(v as i64),
                        Type::Str => Value::Str(Rc::from(format!("{}", v))),
                        Type::Null => Value::Null,
                    }
                },
                Value::Int(v) => {
                    match to_type {
                        Type::Bool => Value::Bool(v > 0),
                        Type::Float => Value::Float(v as f64),
                        Type::Int => Value::Int(v),
                        Type::Str => Value::Str(Rc::from(format!("{}", v))),
                        Type::Null => Value::Null,
                    }
                },
                Value::Str(v) => {
                    match to_type { // Тут надо разобраться
                        Type::Bool => {
                            match v.parse::<bool>() {
                                Ok(pv) => Value::Bool(pv),
                                Err(_) => Value::Null,
                            }
                        },
                        Type::Float => {
                            match v.parse::<f64>() {
                                Ok(pv) => Value::Float(pv),
                                Err(_) => Value::Null,
                            }
                        },
                        Type::Int => {
                            match v.parse::<i64>() {
                                Ok(pv) => Value::Int(pv),
                                Err(_) => Value::Null,
                            }
                        },
                        Type::Str => Value::Str(v),
                        Type::Null => Value::Null,
                    }
                },
                _ => Value::Null,
            }
        },
        Expr::Variable(v_id) => {
            vars[*v_id].clone()
        },
        Expr::Func(id, args) => {
            // Считаем аргументы, пока скрипт никто не трогал
            let values: Vec<Value> = args
                .iter()
                .map(|arg| count_expr(arg, vars, scripts, ctx, sys_func))
                .collect();

            // Записываем их в переменные функции и запускаем
            let script = &scripts[*id];
            let mut variables = vec![Value::Null; script.total_variables].into_boxed_slice();
            for (i, v) in values.into_iter().enumerate() {
                variables[i] = v;
            }
            script.run(&mut variables, scripts, ctx, sys_func)
        },
        Expr::SysFunc(id, args) => {
            // Считаем аргументы, пока скрипт никто не трогал
            let values: Vec<Value> = args
                .iter()
                .map(|arg| count_expr(arg, vars, scripts, ctx, sys_func))
                .collect();
            sys_func[*id as usize](ctx, values)
        }
    }
}
pub fn infer_type(expr: &Expr, types: &Vec<Type>, funcs_types: &Vec<Type>, sys_types: &Vec<Type>) -> Type {
    match expr {
        Expr::Value(v) => {
            match v {
                Value::Bool(_) => Type::Bool,
                Value::Float(_) => Type::Float,
                Value::Int(_) => Type::Int,
                Value::Str(_) => Type::Str,
                _ => Type::Null,
            }
        },
        Expr::Binary(a, op, b) => {
            let v1 = infer_type(a, types, funcs_types, sys_types);
            let v2 = infer_type(b, types, funcs_types, sys_types);

            match v1 {
                Type::Bool => {
                    match &v2{
                        Type::Bool => v2.clone(),
                        _ => Type::Null,
                    }
                },
                Type::Float => {
                    match &v2{
                        Type::Float => v2,
                        _ => Type::Null,
                    }
                },
                Type::Int => {
                    match &v2{
                        Type::Int => v2,
                        _ => Type::Null,
                    }
                },
                Type::Str => {
                    match &v2{
                        Type::Str => v2,
                        _ => Type::Null,
                    }
                },
                _ => return Type::Null,

            }
        },
        Expr::Neg(expr) => {
            infer_type(expr, types, funcs_types, sys_types)
        },
        Expr::As(_, to_type) => {
            to_type.clone()
        },
        Expr::Variable(v_id) => {
            types[*v_id].clone()
        },
        Expr::Func(id, args) => {
            funcs_types[*id].clone()
        }
        Expr::SysFunc(id, args) => {
            sys_types[*id as usize].clone()
        }
    }
}


#[derive(Clone, Debug)]
pub enum Command {
    Var(u32, Expr),
    If(Expr, u32),
    Call(u32, Box<[Expr]>),
    SystemCall(u32, Box<[Expr]>),
    Return(Expr),
    While(Expr, u32),
    JumpUp(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParseResult<Expr> {
        let types_map: HashMap<String, Type> = [
            ("int", Type::Int),
            ("float", Type::Float),
            ("bool", Type::Bool),
            ("str", Type::Str),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let operator_map: HashMap<String, Op> = [
            ("+", Op::Add),
            ("-", Op::Sub),
            ("*", Op::Mul),
            ("/", Op::Div),
            (">", Op::Gr),
            ("<", Op::Lr),
            (">=", Op::Gt),
            ("<=", Op::Lt),
            ("==", Op::Eq),
            ("%", Op::Mod),
            ("and", Op::And),
            ("or", Op::Or),
            ("as", Op::As),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let vars: HashMap<String, usize> = [("x".to_string(), 0)].into_iter().collect();

        let funcs: HashMap<String, usize> = [("add", 0), ("greet", 1), ("zero", 2), ("mix", 3)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let func_args: Vec<Vec<Type>> = vec![
            vec![Type::Int, Type::Int],
            vec![Type::Str],
            vec![],
            vec![Type::Int, Type::Float, Type::Bool],
        ];

        let mut sys = SystemCalls::new();
        sys.include("write", vec![Type::Str], Type::Null);
        sys.include("sqrt", vec![Type::Float], Type::Float);

        ExprParser::new().parse_tokens_text(src.to_string(), &vars, &funcs, &func_args, &sys)
    }

    fn dbg(src: &str) -> String {
        format!("{:?}", parse(src).unwrap())
    }

    #[test]
    fn simple_call() {
        assert_eq!(dbg("add(10, 20)"), "Func(0, [Value(Int(10)), Value(Int(20))])");
        assert_eq!(dbg("add(10,20)"), dbg("add ( 10 , 20 )"));
    }

    #[test]
    fn no_args() {
        assert_eq!(dbg("zero()"), "Func(2, [])");
    }

    #[test]
    fn string_arg_keeps_commas_and_spaces() {
        assert_eq!(dbg(r#"greet("a, b")"#), r#"Func(1, [Value(Str("a, b"))])"#);
    }

    #[test]
    fn nested_and_expressions() {
        assert_eq!(
            dbg("add(add(1, 2), 3 * 4)"),
            "Func(0, [Func(0, [Value(Int(1)), Value(Int(2))]), Binary(Value(Int(3)), Mul, Value(Int(4)))])"
        );
        assert_eq!(dbg("add(x, -5 + 1)"), "Func(0, [Variable(0), Binary(Neg(Value(Int(5))), Add, Value(Int(1)))])");
        assert_eq!(dbg("mix(1, 2 as float, True)"), "Func(3, [Value(Int(1)), As(Value(Int(2)), Float), Value(Bool(true))])");
    }

    #[test]
    fn call_inside_expression() {
        assert_eq!(dbg("add(1, 2) + 3"), "Binary(Func(0, [Value(Int(1)), Value(Int(2))]), Add, Value(Int(3)))");
        assert_eq!(dbg("-add(1, 2)"), "Neg(Func(0, [Value(Int(1)), Value(Int(2))]))");
        assert_eq!(dbg("add(1, 2) as float"), "As(Func(0, [Value(Int(1)), Value(Int(2))]), Float)");
        assert_eq!(dbg("(add(1, 2))"), dbg("add(1, 2)"));
    }

    #[test]
    fn errors() {
        assert!(parse("add(1)").unwrap_err().contains("принимает аргументов: 2, передано: 1"));
        assert!(parse("zero(1)").unwrap_err().contains("принимает аргументов: 0, передано: 1"));
        assert!(parse("add(1, 2.5)").unwrap_err().contains("ожидался Int, получен Float"));
        assert!(parse("mix(1, 2, True)").unwrap_err().contains("аргумент 2"));
        assert!(parse("greet(1 + 2)").unwrap_err().contains("ожидался Str"));
        assert!(parse("nope(1)").unwrap_err().contains("неизвестная функция"));
        assert!(parse("add(1, 2").is_err());
        assert!(parse("add(1 2)").is_err());
        assert!(parse("add(1,, 2)").is_err());
        assert!(parse("add(1, 2,)").is_err());
        assert!(parse("add(1, 2))").is_err());
    }

    #[test]
    fn system_calls_parse() {
        assert_eq!(dbg(r#"write("hi")"#), r#"SysFunc(0, [Value(Str("hi"))])"#);
        assert_eq!(dbg("sqrt(2.0) + 1.0"), "Binary(SysFunc(1, [Value(Float(2.0))]), Add, Value(Float(1.0)))");
        assert_eq!(dbg("sqrt(sqrt(16.0))"), "SysFunc(1, [SysFunc(1, [Value(Float(16.0))])])");
        // пользовательские функции продолжают работать, в том числе с системным вызовом внутри
        assert_eq!(dbg("add(1, 2)"), "Func(0, [Value(Int(1)), Value(Int(2))])");
        assert_eq!(dbg(r#"greet(x)"#), "Func(1, [Variable(0)])");
    }

    #[test]
    fn system_calls_errors() {
        assert!(parse("write()").unwrap_err().contains("принимает аргументов: 1, передано: 0"));
        assert!(parse("sqrt(1)").unwrap_err().contains("ожидался Float, получен Int"));
        assert!(parse("add(1, sqrt(4.0))").unwrap_err().contains("ожидался Int, получен Float"));
        assert!(parse(r#"greet(write("x"))"#).unwrap_err().contains("ожидался Str, получен Null"));
        assert!(parse("nope(1)").unwrap_err().contains("неизвестная функция"));
    }

    fn sys_write(ctx: &mut Vec<String>, args: Vec<Value>) -> Value {
        if let Some(Value::Str(s)) = args.first() {
            ctx.push(s.to_string());
        }
        Value::Null
    }
    fn sys_sqrt(_: &mut Vec<String>, args: Vec<Value>) -> Value {
        match args.first() {
            Some(Value::Float(x)) => Value::Float(x.sqrt()),
            _ => Value::Null,
        }
    }

    #[test]
    fn system_calls_run() {
        let funcs: [SysFn<Vec<String>>; 2] = [sys_write, sys_sqrt];
        let mut ctx: Vec<String> = Vec::new();
        let scripts: Box<[Script]> = Vec::new().into_boxed_slice();
        let vars: Box<[Value]> = Vec::new().into_boxed_slice();

        let e = parse("sqrt(sqrt(16.0)) + 1.0").unwrap();
        assert!(matches!(count_expr(&e, &vars, &scripts, &mut ctx, &funcs), Value::Float(x) if x == 3.0));

        let e = parse(r#"write("hi")"#).unwrap();
        count_expr(&e, &vars, &scripts, &mut ctx, &funcs);
        assert_eq!(ctx, ["hi"]);
    }
}
