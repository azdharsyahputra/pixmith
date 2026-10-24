//! A small JSON parser and serializer: enough to read Aseprite sprite sheet metadata and to keep Pixmith's own
//! small JSON files (`.pxmt` META chunks, `project.json`).

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    /// Members in file order; duplicate keys are kept and `get` returns the first.
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            // Aseprite writes some numbers as strings ("scale": "1").
            Value::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Arr(a) = self {
            Some(a)
        } else {
            None
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        if let Value::Obj(m) = self {
            Some(m)
        } else {
            None
        }
    }
}

/// Serializes a value as compact JSON (members in order).
pub fn to_string(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_value(v: &Value, out: &mut String) {
    use crate::formats::sheet::json_str;
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => out.push_str(&(*n as i64).to_string()),
        Value::Num(n) => out.push_str(&n.to_string()),
        Value::Str(s) => out.push_str(&json_str(s)),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(x, out);
            }
            out.push(']');
        }
        Value::Obj(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&json_str(k));
                out.push(':');
                write_value(x, out);
            }
            out.push('}');
        }
    }
}

const MAX_DEPTH: usize = 64;

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser { s: text.as_bytes(), pos: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.pos != p.s.len() {
        return Err(p.err("unexpected text after the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> String {
        let line = self.s[..self.pos.min(self.s.len())].iter().filter(|&&b| b == b'\n').count() + 1;
        format!("invalid JSON at line {line}: {msg}")
    }

    fn ws(&mut self) {
        while matches!(self.s.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        match self.s.get(self.pos) {
            None => Err(self.err("unexpected end")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') if self.eat("true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        while matches!(self.s.get(self.pos), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.s[start..self.pos])
            .ok()
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .map(Value::Num)
            .ok_or_else(|| self.err("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.pos..self.pos + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let v = std::str::from_utf8(digits).ok().and_then(|t| u32::from_str_radix(t, 16).ok());
        self.pos += 4;
        v.ok_or_else(|| self.err("bad \\u escape"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1; // opening quote
        let mut out = Vec::new();
        loop {
            let Some(&b) = self.s.get(self.pos) else { return Err(self.err("unterminated string")) };
            self.pos += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.pos) else { return Err(self.err("unterminated string")) };
                    self.pos += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + lo.wrapping_sub(0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("bad escape")),
                    };
                    let mut buf = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1F => return Err(self.err("control character in string")),
                _ => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("invalid UTF-8"))
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat("]") {
            return Ok(Value::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("]") {
                return Ok(Value::Arr(items));
            }
            return Err(self.err("expected , or ]"));
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.pos += 1;
        let mut members = Vec::new();
        self.ws();
        if self.eat("}") {
            return Ok(Value::Obj(members));
        }
        loop {
            self.ws();
            if self.s.get(self.pos) != Some(&b'"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(":") {
                return Err(self.err("expected :"));
            }
            self.ws();
            members.push((key, self.value(depth + 1)?));
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("}") {
                return Ok(Value::Obj(members));
            }
            return Err(self.err("expected , or }"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values() {
        let v = parse(r#" { "a": [1, -2.5e1, "x", true, false, null], "b": { "c": "d" }, "e": [] } "#).unwrap();
        let a = v.get("a").unwrap().as_array().unwrap();
        assert_eq!(a[0].as_f64(), Some(1.0));
        assert_eq!(a[1].as_f64(), Some(-25.0));
        assert_eq!(a[2].as_str(), Some("x"));
        assert_eq!(a[3], Value::Bool(true));
        assert_eq!(a[5], Value::Null);
        assert_eq!(v.get("b").unwrap().get("c").unwrap().as_str(), Some("d"));
        assert_eq!(v.get("e"), Some(&Value::Arr(vec![])));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn keeps_member_order_and_numeric_strings() {
        let v = parse(r#"{"z": 1, "a": 2, "scale": "2"}"#).unwrap();
        let keys: Vec<_> = v.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["z", "a", "scale"]);
        assert_eq!(v.get("scale").unwrap().as_f64(), Some(2.0));
    }

    #[test]
    fn decodes_string_escapes() {
        let v = parse(r#""a\"b\\c\n\u00e9\ud83d\ude00 é""#).unwrap();
        assert_eq!(v.as_str(), Some("a\"b\\c\né\u{1F600} é"));
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "{1: 2}", "tru", "\"abc", "[1] x", "01x", "\"\\q\"", "\"a\nb\"", "-"]
        {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert!(parse(&"[".repeat(200)).unwrap_err().contains("deeply"));
        assert!(parse("[1,\n2,\n}").unwrap_err().contains("line 3"));
    }
}
