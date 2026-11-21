use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub enum Value {
    Dict(Vec<(String, Value)>),
    Array(Vec<Value>),
    Str(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    Data(Vec<u8>),
    Date(String),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(d) => d.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn path(&self, keys: &[&str]) -> Option<&Value> {
        let mut v = self;
        for k in keys {
            v = v.get(k)?;
        }
        Some(v)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_dict(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_data(&self) -> Option<&[u8]> {
        match self {
            Value::Data(d) => Some(d),
            _ => None,
        }
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';') else { break };
        let ent = &rest[1..j];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                if let Some(h) = ent.strip_prefix("#x") {
                    u32::from_str_radix(h, 16).ok().and_then(char::from_u32)
                } else if let Some(d) = ent.strip_prefix('#') {
                    d.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

enum Frame {
    Dict(Vec<(String, Value)>, Option<String>),
    Array(Vec<Value>),
}

fn place(stack: &mut [Frame], root: &mut Option<Value>, v: Value) -> Result<()> {
    match stack.last_mut() {
        Some(Frame::Dict(items, key)) => {
            let k = key.take().ok_or_else(|| anyhow::anyhow!("value without key"))?;
            items.push((k, v));
        }
        Some(Frame::Array(a)) => a.push(v),
        None => *root = Some(v),
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<Value> {
    if text.as_bytes().starts_with(b"bplist") {
        bail!("binary plist is not supported");
    }
    let len = text.len();
    let mut i = 0;
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Value> = None;
    while i < len {
        let Some(p) = text[i..].find('<') else { break };
        i += p;
        if text[i..].starts_with("<?") {
            i = text[i..].find("?>").map(|p| i + p + 2).unwrap_or(len);
            continue;
        }
        if text[i..].starts_with("<!--") {
            i = text[i..].find("-->").map(|p| i + p + 3).unwrap_or(len);
            continue;
        }
        if text[i..].starts_with("<!") {
            i = text[i..].find('>').map(|p| i + p + 1).unwrap_or(len);
            continue;
        }
        let end = match text[i..].find('>') {
            Some(p) => i + p,
            None => bail!("unterminated tag"),
        };
        let tag = &text[i + 1..end];
        i = end + 1;
        let selfclose = tag.ends_with('/');
        let name = tag.trim_end_matches('/').split_whitespace().next().unwrap_or("");
        match name {
            "plist" | "/plist" => {}
            "dict" => {
                if selfclose {
                    place(&mut stack, &mut root, Value::Dict(Vec::new()))?;
                } else {
                    stack.push(Frame::Dict(Vec::new(), None));
                }
            }
            "array" => {
                if selfclose {
                    place(&mut stack, &mut root, Value::Array(Vec::new()))?;
                } else {
                    stack.push(Frame::Array(Vec::new()));
                }
            }
            "/dict" => match stack.pop() {
                Some(Frame::Dict(d, _)) => place(&mut stack, &mut root, Value::Dict(d))?,
                _ => bail!("unbalanced dict"),
            },
            "/array" => match stack.pop() {
                Some(Frame::Array(a)) => place(&mut stack, &mut root, Value::Array(a))?,
                _ => bail!("unbalanced array"),
            },
            "true" => place(&mut stack, &mut root, Value::Bool(true))?,
            "false" => place(&mut stack, &mut root, Value::Bool(false))?,
            "key" | "string" | "integer" | "real" | "data" | "date" => {
                let content = if selfclose {
                    String::new()
                } else {
                    let close = format!("</{name}>");
                    let p = match text[i..].find(&close) {
                        Some(p) => p,
                        None => bail!("missing {close}"),
                    };
                    let c = unescape(&text[i..i + p]);
                    i += p + close.len();
                    c
                };
                match name {
                    "key" => match stack.last_mut() {
                        Some(Frame::Dict(_, k)) => *k = Some(content),
                        _ => bail!("key outside dict"),
                    },
                    "string" => place(&mut stack, &mut root, Value::Str(content))?,
                    "integer" => {
                        let t = content.trim();
                        let v = t.parse::<i64>().or_else(|_| t.parse::<u64>().map(|v| v as i64))?;
                        place(&mut stack, &mut root, Value::Int(v))?
                    }
                    "real" => place(&mut stack, &mut root, Value::Real(content.trim().parse()?))?,
                    "date" => place(&mut stack, &mut root, Value::Date(content))?,
                    _ => {
                        use base64::Engine;
                        let clean: String = content.chars().filter(|c| !c.is_whitespace()).collect();
                        let d = base64::engine::general_purpose::STANDARD.decode(clean)?;
                        place(&mut stack, &mut root, Value::Data(d))?
                    }
                }
            }
            other => bail!("unknown plist tag {other}"),
        }
    }
    root.ok_or_else(|| anyhow::anyhow!("empty plist"))
}
