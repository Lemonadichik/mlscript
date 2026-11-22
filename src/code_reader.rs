use std::{arch::naked_asm, fs::read_to_string};

use crate::ml_structs::Token;

#[derive(Debug)]
pub enum LoadError {
    ReadFileError(String),
}

pub fn load_script(path: &str) -> Result<Vec<(String, usize)>, LoadError> {

    let data = match read_to_string(path) {
        Ok(f) => f,
        Err(e) => return Err(LoadError::ReadFileError(format!("file {path} dont read {e}"))),
    };

    let lines: Vec<&str> = data.lines().collect();
    let mut commands: Vec<(String, usize)> = Vec::new();
    let mut local_buffer: Vec<char> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();

        for char in chars {

            if char == '{' || char == '}' {
                if !local_buffer.is_empty() {
                    commands.push((local_buffer.iter().collect(), i));
                    local_buffer.clear();
                }
                commands.push((char.to_string(), i));
                continue;
            }

            if char == ';'{
                if !local_buffer.is_empty() {
                    commands.push((local_buffer.iter().collect(), i));
                    local_buffer.clear();
                }
                continue;
            }

            if char == '#' {
                commands.push((local_buffer.iter().collect(), i));
                local_buffer.clear();
                break;
            }

            local_buffer.push(char);

        }
    }

    let cleaned_commands: Vec<(String, usize)> = commands.iter().map(|c| (c.0.trim().to_string(), c.1)).collect();

    return Ok(cleaned_commands);
}

pub struct FuncSig {
    pub name: String,
    pub params: Vec<(String, String)>, // (type, name)
    pub ret: Option<String>,
}
pub struct FuncCall {
    pub name: String,
    pub params: Vec<String>, // expt_text
}

pub fn parse_function_define(scr: &str) -> Result<FuncSig, String> {
    let open = scr.find('(').ok_or("missing '('")?;
    let close = scr.find(')').ok_or("missing ')'")?;
    if close < open {
        return Err("')' appears before '('".into());
    }

    let name = scr[..open].trim().to_string();
    if name.is_empty() {
        return Err("missing function name".into());
    }

    let mut params = Vec::new();
    let inner = scr[open + 1..close].trim();
    if !inner.is_empty() {
        for part in inner.split(',') {
            let mut it = part.split_whitespace();
            match (it.next(), it.next(), it.next()) {
                (Some(ty), Some(n), None) => params.push((ty.to_string(), n.to_string())),
                _ => return Err(format!("invalid parameter: '{}'", part.trim())),
            }
        }
    }

    let rest = scr[close + 1..].trim();
    let ret = match rest.strip_prefix('>') {
        Some(t) => Some(t.trim().to_string()),
        None if rest.is_empty() => None,
        None => return Err(format!("unexpected text after ')': '{rest}'")),
    };

    Ok(FuncSig { name, params, ret })
}

pub fn parse_function_call(scr: &str) -> Result<FuncCall, String> {
    let scr = scr.trim();

    let open = scr.find('(').ok_or("missing '('")?;
    let name = scr[..open].trim().to_string();
    if name.is_empty() {
        return Err("missing function name".into());
    }

    // Ищем закрывающую скобку, парную первой '(' (с учётом вложенности и строк)
    let mut depth = 0usize;
    let mut in_str: Option<char> = None;
    let mut escaped = false;
    let mut close = None;

    for (i, c) in scr[open..].char_indices() {
        let i = open + i;
        if let Some(q) = in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_str = Some(c),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    if in_str.is_some() {
        return Err("unterminated string literal".into());
    }
    let close = close.ok_or("missing ')'")?;

    let rest = scr[close + 1..].trim();
    if !rest.is_empty() {
        return Err(format!("unexpected text after ')': '{rest}'"));
    }

    let params = split_args(&scr[open + 1..close])?;
    Ok(FuncCall { name, params })
}

/// Делит строку аргументов по запятым верхнего уровня.
fn split_args(inner: &str) -> Result<Vec<String>, String> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut in_str: Option<char> = None;
    let mut escaped = false;

    for c in inner.chars() {
        if let Some(q) = in_str {
            cur.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_str = Some(c);
                cur.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth = depth.checked_sub(1).ok_or("unbalanced brackets in arguments")?;
                cur.push(c);
            }
            ',' if depth == 0 => {
                let arg = cur.trim();
                if arg.is_empty() {
                    return Err("empty argument".into());
                }
                args.push(arg.to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }

    let arg = cur.trim();
    if arg.is_empty() {
        return Err("empty argument (trailing comma?)".into());
    }
    args.push(arg.to_string());
    Ok(args)
}