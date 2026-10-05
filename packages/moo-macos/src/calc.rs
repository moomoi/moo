//! Inline answers for root search, as Spotlight and Raycast give them while you type: arithmetic
//! ("2^10", "15% of 80", "120 + 15%", "sqrt(2)"), unit conversion ("5 km in mi", "70f",
//! "3 cups to ml"), currency ("100 usd to eur", "€50 in $", given exchange rates) and number bases
//! ("255 in hex", "0xff"). Portable; rates are passed in.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// As shown: "1,024", "3.10686 mi".
    pub display: String,
    /// What enter copies: "1024", "3.10686".
    pub copy: String,
    /// Shown beside it: "5 km = 3.10686 mi", "Calculator".
    pub detail: String,
}

/// Units of each currency per 1 EUR (the ECB's reference rates), keyed by upper-case ISO code.
pub type Rates = HashMap<String, f64>;

pub fn answer(input: &str, rates: Option<&Rates>) -> Option<Answer> {
    let text = normalize(input);
    if text.is_empty() || text.len() > 200 {
        return None;
    }
    if let Some((left, right)) = split_conversion(&text) {
        if let Some(a) = convert(left, right, rates) {
            return Some(a);
        }
    }
    if let Some(a) = default_conversion(&text) {
        return Some(a);
    }
    math(&text)
}

fn normalize(s: &str) -> String {
    s.trim().replace('×', "*").replace('÷', "/").replace('−', "-").replace("**", "^")
}

/// "5 km in mi" -> ("5 km", "mi"), on the last " in ", " to ", " as " or " into ".
fn split_conversion(text: &str) -> Option<(&str, &str)> {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        // Lower-casing changed byte offsets (rare scripts); do not guess.
        return None;
    }
    let mut best: Option<(usize, usize)> = None;
    for sep in [" in ", " to ", " as ", " into ", " = "] {
        if let Some(i) = lower.rfind(sep) {
            if best.is_none_or(|(b, _)| i > b) {
                best = Some((i, sep.len()));
            }
        }
    }
    let (i, n) = best?;
    let (left, right) = (text[..i].trim(), text[i + n..].trim());
    (!left.is_empty() && !right.is_empty()).then_some((left, right))
}

// ── Numbers ─────────────────────────────────────────────────────────────────

/// Up to 12 significant digits, no grouping: what gets copied. None for NaN and infinities.
pub fn plain(v: f64) -> Option<String> {
    sig(v, 12)
}

fn sig(v: f64, digits: i32) -> Option<String> {
    if !v.is_finite() {
        return None;
    }
    if v == 0.0 {
        return Some("0".into());
    }
    let a = v.abs();
    if !(1e-9..1e15).contains(&a) {
        let s = format!("{:.*e}", (digits.min(7) - 1).max(0) as usize, v);
        let (mantissa, exp) = s.split_once('e')?;
        let mantissa = trim_zeros(mantissa);
        let exp: i32 = exp.parse().ok()?;
        return Some(format!("{mantissa}e{}{}", if exp < 0 { "-" } else { "+" }, exp.abs()));
    }
    let int_digits = a.log10().floor() as i32 + 1;
    let decimals = (digits - int_digits).clamp(0, 12) as usize;
    let s = trim_zeros(&format!("{v:.decimals$}"));
    Some(if s == "-0" { "0".into() } else { s })
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// "1234567.5" -> "1,234,567.5"; scientific notation is left alone.
pub fn group(s: &str) -> String {
    if s.contains('e') {
        return s.to_string();
    }
    let (sign, rest) = s.strip_prefix('-').map_or(("", s), |r| ("-", r));
    let (int, frac) = rest.split_once('.').map_or((rest, None), |(i, f)| (i, Some(f)));
    if int.len() <= 3 {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

// ── Expressions ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    /// A number written in another base: "0xff".
    Based(f64),
    Ident(String),
    Op(char),
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '0' && i + 2 < c.len() + 1 && matches!(c.get(i + 1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O')) && c.get(i + 2).is_some_and(|d| d.is_ascii_hexdigit()) {
            let radix = match c[i + 1].to_ascii_lowercase() {
                'x' => 16,
                'b' => 2,
                _ => 8,
            };
            let start = i + 2;
            let mut j = start;
            while j < c.len() && (c[j].is_digit(radix) || c[j] == '_') {
                j += 1;
            }
            let digits: String = c[start..j].iter().filter(|d| **d != '_').collect();
            out.push(Tok::Based(i64::from_str_radix(&digits, radix).ok()? as f64));
            i = j;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut num = String::new();
            let mut j = i;
            while j < c.len() {
                let d = c[j];
                if d.is_ascii_digit() || d == '.' || d == '_' {
                    if d != '_' {
                        num.push(d);
                    }
                    j += 1;
                } else if d == ',' && j + 3 < c.len() + 1 && c[j + 1..].iter().take(3).filter(|x| x.is_ascii_digit()).count() == 3 && !c.get(j + 4).is_some_and(|x| x.is_ascii_digit()) {
                    // Thousands separator: "1,000".
                    j += 1;
                } else if (d == 'e' || d == 'E') && (c.get(j + 1).is_some_and(|x| x.is_ascii_digit()) || (matches!(c.get(j + 1), Some('+' | '-')) && c.get(j + 2).is_some_and(|x| x.is_ascii_digit()))) {
                    num.push('e');
                    if matches!(c[j + 1], '+' | '-') {
                        num.push(c[j + 1]);
                        j += 1;
                    }
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::Num(num.parse().ok()?));
            i = j;
        } else if ch.is_alphabetic() || ch == 'π' || ch == '_' {
            let mut j = i;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_' || c[j] == 'π') {
                j += 1;
            }
            out.push(Tok::Ident(c[i..j].iter().collect::<String>().to_lowercase()));
            i = j;
        } else if "+-*/^%!(),".contains(ch) {
            out.push(Tok::Op(ch));
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

#[derive(Clone, Copy, Debug)]
struct V {
    v: f64,
    /// Written as a percentage: `a + b%` adds b percent of a.
    pct: bool,
}

fn val(v: f64) -> V {
    V { v, pct: false }
}

const CONSTANTS: [(&str, f64); 5] = [
    ("pi", std::f64::consts::PI),
    ("π", std::f64::consts::PI),
    ("e", std::f64::consts::E),
    ("tau", std::f64::consts::TAU),
    ("phi", 1.618_033_988_749_895),
];

const FUNCTIONS: [&str; 26] = [
    "sqrt", "cbrt", "abs", "round", "floor", "ceil", "trunc", "sin", "cos", "tan", "asin", "acos", "atan", "sinh",
    "cosh", "tanh", "ln", "log", "log2", "log10", "exp", "min", "max", "pow", "hypot", "fact",
];

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<V> {
        let mut a = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(c @ ('+' | '-'))) => *c,
                _ => return Some(a),
            };
            self.pos += 1;
            let b = self.term()?;
            let bv = if b.pct && !a.pct { a.v * b.v } else { b.v };
            a = val(if op == '+' { a.v + bv } else { a.v - bv });
        }
    }

    fn starts_operand(&self) -> bool {
        match self.peek() {
            Some(Tok::Op('(')) => true,
            Some(Tok::Ident(id)) => CONSTANTS.iter().any(|(n, _)| n == id) || FUNCTIONS.contains(&id.as_str()),
            _ => false,
        }
    }

    fn term(&mut self) -> Option<V> {
        let mut a = self.unary()?;
        loop {
            match self.peek() {
                Some(Tok::Op('*')) => {
                    self.pos += 1;
                    let b = self.unary()?;
                    a = val(a.v * b.v);
                }
                Some(Tok::Op('/')) => {
                    self.pos += 1;
                    let b = self.unary()?;
                    a = val(a.v / b.v);
                }
                Some(Tok::Ident(id)) if id == "mod" => {
                    self.pos += 1;
                    let b = self.unary()?;
                    a = val(a.v - b.v * (a.v / b.v).floor());
                }
                Some(Tok::Ident(id)) if id == "of" && a.pct => {
                    self.pos += 1;
                    let b = self.unary()?;
                    a = val(a.v * b.v);
                }
                _ if self.starts_operand() => {
                    let b = self.unary()?;
                    a = val(a.v * b.v);
                }
                _ => return Some(a),
            }
        }
    }

    fn unary(&mut self) -> Option<V> {
        if self.eat('-') {
            let x = self.unary()?;
            return Some(V { v: -x.v, pct: x.pct });
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<V> {
        let base = self.postfix()?;
        if self.eat('^') {
            let exp = self.unary()?;
            return Some(val(base.v.powf(exp.v)));
        }
        Some(base)
    }

    fn postfix(&mut self) -> Option<V> {
        let mut x = self.primary()?;
        loop {
            if self.eat('!') {
                x = val(factorial(x.v)?);
            } else if self.eat('%') {
                x = V { v: x.v / 100.0, pct: true };
            } else {
                return Some(x);
            }
        }
    }

    fn primary(&mut self) -> Option<V> {
        match self.peek()?.clone() {
            Tok::Num(n) | Tok::Based(n) => {
                self.pos += 1;
                Some(val(n))
            }
            Tok::Op('(') => {
                self.pos += 1;
                let x = self.expr()?;
                self.eat(')').then_some(val(x.v))
            }
            Tok::Ident(id) => {
                self.pos += 1;
                if let Some((_, c)) = CONSTANTS.iter().find(|(n, _)| *n == id) {
                    return Some(val(*c));
                }
                if !FUNCTIONS.contains(&id.as_str()) {
                    return None;
                }
                let args = if self.eat('(') {
                    let mut args = vec![self.expr()?.v];
                    while self.eat(',') {
                        args.push(self.expr()?.v);
                    }
                    if !self.eat(')') {
                        return None;
                    }
                    args
                } else {
                    vec![self.power()?.v]
                };
                apply(&id, &args).map(val)
            }
            _ => None,
        }
    }
}

fn factorial(n: f64) -> Option<f64> {
    if n < 0.0 || n.fract() != 0.0 || n > 170.0 {
        return None;
    }
    Some((1..=n as u64).fold(1.0, |acc, k| acc * k as f64))
}

fn apply(f: &str, a: &[f64]) -> Option<f64> {
    let x = *a.first()?;
    let one = a.len() == 1;
    Some(match (f, a.len()) {
        ("min", _) => a.iter().copied().fold(f64::INFINITY, f64::min),
        ("max", _) => a.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ("pow", 2) => x.powf(a[1]),
        ("hypot", 2) => x.hypot(a[1]),
        ("log", 2) => x.ln() / a[1].ln(),
        ("round", 2) => {
            let m = 10f64.powi(a[1] as i32);
            (x * m).round() / m
        }
        _ if !one => return None,
        ("sqrt", _) => x.sqrt(),
        ("cbrt", _) => x.cbrt(),
        ("abs", _) => x.abs(),
        ("round", _) => x.round(),
        ("floor", _) => x.floor(),
        ("ceil", _) => x.ceil(),
        ("trunc", _) => x.trunc(),
        ("sin", _) => x.sin(),
        ("cos", _) => x.cos(),
        ("tan", _) => x.tan(),
        ("asin", _) => x.asin(),
        ("acos", _) => x.acos(),
        ("atan", _) => x.atan(),
        ("sinh", _) => x.sinh(),
        ("cosh", _) => x.cosh(),
        ("tanh", _) => x.tanh(),
        ("ln", _) => x.ln(),
        ("log" | "log10", _) => x.log10(),
        ("log2", _) => x.log2(),
        ("exp", _) => x.exp(),
        ("fact", _) => factorial(x)?,
        _ => return None,
    })
}

/// Evaluate an arithmetic expression.
pub fn eval(text: &str) -> Option<f64> {
    let toks = lex(&normalize(text))?;
    let mut p = Parser { toks, pos: 0 };
    let v = p.expr()?;
    (p.pos == p.toks.len()).then_some(v.v)
}

fn math(text: &str) -> Option<Answer> {
    let toks = lex(text)?;
    // A bare number is not a calculation; "0xff" (another base) and constants like "pi" are.
    let interesting = match toks.as_slice() {
        [] => false,
        [Tok::Num(_)] | [Tok::Op('-'), Tok::Num(_)] => false,
        [Tok::Ident(id)] => id.chars().count() >= 2,
        _ => true,
    };
    if !interesting {
        return None;
    }
    let mut p = Parser { toks, pos: 0 };
    let v = p.expr()?;
    if p.pos != p.toks.len() {
        return None;
    }
    let copy = plain(v.v)?;
    Some(Answer { display: group(&copy), copy, detail: "Calculator".into() })
}

// ── Units ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dim {
    Length,
    Mass,
    Volume,
    Temp,
    Time,
    Data,
    Speed,
    Area,
    Energy,
    Pressure,
    Angle,
}

struct Unit {
    names: &'static [&'static str],
    symbol: &'static str,
    dim: Dim,
    /// value in the base unit = value * factor + offset
    factor: f64,
    offset: f64,
}

const fn u(names: &'static [&'static str], symbol: &'static str, dim: Dim, factor: f64) -> Unit {
    Unit { names, symbol, dim, factor, offset: 0.0 }
}

use Dim::*;

static UNITS: &[Unit] = &[
    u(&["mm", "millimeter", "millimeters", "millimetre", "millimetres"], "mm", Length, 0.001),
    u(&["cm", "centimeter", "centimeters", "centimetre", "centimetres"], "cm", Length, 0.01),
    u(&["m", "meter", "meters", "metre", "metres"], "m", Length, 1.0),
    u(&["km", "kilometer", "kilometers", "kilometre", "kilometres", "kms"], "km", Length, 1000.0),
    u(&["inch", "inches", "\""], "in", Length, 0.0254),
    u(&["ft", "foot", "feet", "'"], "ft", Length, 0.3048),
    u(&["yd", "yard", "yards"], "yd", Length, 0.9144),
    u(&["mi", "mile", "miles"], "mi", Length, 1609.344),
    u(&["nmi", "nauticalmile", "nauticalmiles"], "nmi", Length, 1852.0),
    u(&["mg", "milligram", "milligrams"], "mg", Mass, 1e-6),
    u(&["g", "gram", "grams", "gramme", "grammes"], "g", Mass, 0.001),
    u(&["kg", "kilo", "kilos", "kilogram", "kilograms"], "kg", Mass, 1.0),
    u(&["t", "tonne", "tonnes", "ton", "tons"], "t", Mass, 1000.0),
    u(&["oz", "ounce", "ounces"], "oz", Mass, 0.028_349_523_125),
    u(&["lb", "lbs", "pound", "pounds"], "lb", Mass, 0.453_592_37),
    u(&["st", "stone", "stones"], "st", Mass, 6.350_293_18),
    u(&["ml", "milliliter", "milliliters", "millilitre", "millilitres"], "ml", Volume, 0.001),
    u(&["cl", "centiliter", "centiliters"], "cl", Volume, 0.01),
    u(&["dl", "deciliter", "deciliters"], "dl", Volume, 0.1),
    u(&["l", "liter", "liters", "litre", "litres"], "L", Volume, 1.0),
    u(&["m3", "m³"], "m³", Volume, 1000.0),
    u(&["tsp", "teaspoon", "teaspoons"], "tsp", Volume, 0.004_928_921_593_75),
    u(&["tbsp", "tablespoon", "tablespoons"], "tbsp", Volume, 0.014_786_764_781_25),
    u(&["floz", "fluidounce", "fluidounces"], "fl oz", Volume, 0.029_573_529_562_5),
    u(&["cup", "cups"], "cups", Volume, 0.236_588_236_5),
    u(&["pt", "pint", "pints"], "pt", Volume, 0.473_176_473),
    u(&["qt", "quart", "quarts"], "qt", Volume, 0.946_352_946),
    u(&["gal", "gallon", "gallons"], "gal", Volume, 3.785_411_784),
    Unit { names: &["c", "°c", "celsius", "degc"], symbol: "°C", dim: Temp, factor: 1.0, offset: 273.15 },
    Unit { names: &["f", "°f", "fahrenheit", "degf"], symbol: "°F", dim: Temp, factor: 5.0 / 9.0, offset: 459.67 * 5.0 / 9.0 },
    u(&["k", "kelvin"], "K", Temp, 1.0),
    u(&["ms", "millisecond", "milliseconds"], "ms", Time, 0.001),
    u(&["s", "sec", "secs", "second", "seconds"], "s", Time, 1.0),
    u(&["min", "mins", "minute", "minutes"], "min", Time, 60.0),
    u(&["h", "hr", "hrs", "hour", "hours"], "h", Time, 3600.0),
    u(&["d", "day", "days"], "days", Time, 86400.0),
    u(&["wk", "wks", "week", "weeks"], "weeks", Time, 604_800.0),
    u(&["month", "months"], "months", Time, 2_629_746.0),
    u(&["yr", "yrs", "year", "years"], "years", Time, 31_556_952.0),
    u(&["bit", "bits"], "bit", Data, 0.125),
    u(&["byte", "bytes"], "B", Data, 1.0),
    u(&["kb", "kilobyte", "kilobytes"], "KB", Data, 1e3),
    u(&["mb", "megabyte", "megabytes"], "MB", Data, 1e6),
    u(&["gb", "gigabyte", "gigabytes"], "GB", Data, 1e9),
    u(&["tb", "terabyte", "terabytes"], "TB", Data, 1e12),
    u(&["pb", "petabyte", "petabytes"], "PB", Data, 1e15),
    u(&["kib", "kibibyte", "kibibytes"], "KiB", Data, 1024.0),
    u(&["mib", "mebibyte", "mebibytes"], "MiB", Data, 1_048_576.0),
    u(&["gib", "gibibyte", "gibibytes"], "GiB", Data, 1_073_741_824.0),
    u(&["tib", "tebibyte", "tebibytes"], "TiB", Data, 1_099_511_627_776.0),
    u(&["m/s", "mps"], "m/s", Speed, 1.0),
    u(&["km/h", "kmh", "kph", "kmph"], "km/h", Speed, 1.0 / 3.6),
    u(&["mph"], "mph", Speed, 0.447_04),
    u(&["kn", "kt", "knot", "knots"], "kn", Speed, 0.514_444_444),
    u(&["ft/s", "fps"], "ft/s", Speed, 0.3048),
    u(&["cm2", "cm²"], "cm²", Area, 1e-4),
    u(&["m2", "m²", "sqm"], "m²", Area, 1.0),
    u(&["km2", "km²", "sqkm"], "km²", Area, 1e6),
    u(&["in2", "in²", "sqin"], "in²", Area, 0.000_645_16),
    u(&["ft2", "ft²", "sqft"], "ft²", Area, 0.092_903_04),
    u(&["mi2", "mi²", "sqmi"], "mi²", Area, 2_589_988.110_336),
    u(&["acre", "acres", "ac"], "acres", Area, 4046.856_422_4),
    u(&["ha", "hectare", "hectares"], "ha", Area, 10_000.0),
    u(&["j", "joule", "joules"], "J", Energy, 1.0),
    u(&["kj", "kilojoule", "kilojoules"], "kJ", Energy, 1000.0),
    u(&["cal", "calorie", "calories"], "cal", Energy, 4.184),
    u(&["kcal", "kilocalorie", "kilocalories"], "kcal", Energy, 4184.0),
    u(&["wh"], "Wh", Energy, 3600.0),
    u(&["kwh"], "kWh", Energy, 3.6e6),
    u(&["pa", "pascal", "pascals"], "Pa", Pressure, 1.0),
    u(&["kpa"], "kPa", Pressure, 1000.0),
    u(&["bar", "bars"], "bar", Pressure, 1e5),
    u(&["psi"], "psi", Pressure, 6894.757_293_168),
    u(&["atm"], "atm", Pressure, 101_325.0),
    u(&["mmhg"], "mmHg", Pressure, 133.322_387_415),
    u(&["rad", "radian", "radians"], "rad", Angle, 1.0),
    u(&["deg", "degree", "degrees", "°"], "°", Angle, std::f64::consts::PI / 180.0),
];

/// "5 km" with no target converts to the other measuring system.
const COUNTERPARTS: [(&str, &str); 20] = [
    ("km", "mi"),
    ("mi", "km"),
    ("m", "ft"),
    ("ft", "m"),
    ("cm", "in"),
    ("in", "cm"),
    ("mm", "in"),
    ("yd", "m"),
    ("kg", "lb"),
    ("lb", "kg"),
    ("g", "oz"),
    ("oz", "g"),
    ("L", "gal"),
    ("gal", "L"),
    ("ml", "fl oz"),
    ("fl oz", "ml"),
    ("°C", "°F"),
    ("°F", "°C"),
    ("km/h", "mph"),
    ("mph", "km/h"),
];

fn unit(name: &str) -> Option<&'static Unit> {
    let n = name.trim().to_lowercase().replace(' ', "");
    UNITS.iter().find(|u| u.names.contains(&n.as_str()) || u.symbol.to_lowercase().replace(' ', "") == n)
}

fn unit_by_symbol(symbol: &str) -> Option<&'static Unit> {
    UNITS.iter().find(|u| u.symbol == symbol)
}

fn convert_unit(v: f64, from: &Unit, to: &Unit) -> f64 {
    (v * from.factor + from.offset - to.offset) / to.factor
}

/// Up to 6 significant digits, never hiding the integer part.
fn short(v: f64) -> Option<String> {
    let int_digits = if v == 0.0 { 1 } else { (v.abs().log10().floor() as i32 + 1).max(1) };
    sig(v, int_digits.max(6))
}

// ── Currency ────────────────────────────────────────────────────────────────

const CURRENCY_WORDS: [(&str, &str); 30] = [
    ("$", "USD"),
    ("us$", "USD"),
    ("dollar", "USD"),
    ("dollars", "USD"),
    ("€", "EUR"),
    ("euro", "EUR"),
    ("euros", "EUR"),
    ("£", "GBP"),
    ("sterling", "GBP"),
    ("¥", "JPY"),
    ("yen", "JPY"),
    ("₹", "INR"),
    ("rupee", "INR"),
    ("rupees", "INR"),
    ("yuan", "CNY"),
    ("rmb", "CNY"),
    ("₩", "KRW"),
    ("won", "KRW"),
    ("₺", "TRY"),
    ("lira", "TRY"),
    ("₪", "ILS"),
    ("shekel", "ILS"),
    ("shekels", "ILS"),
    ("฿", "THB"),
    ("baht", "THB"),
    ("franc", "CHF"),
    ("francs", "CHF"),
    ("real", "BRL"),
    ("reais", "BRL"),
    ("peso", "MXN"),
];

fn currency(name: &str, rates: &Rates) -> Option<String> {
    let n = name.trim().to_lowercase();
    if let Some((_, code)) = CURRENCY_WORDS.iter().find(|(w, _)| *w == n) {
        return Some(code.to_string());
    }
    let up = n.to_uppercase();
    (up.len() == 3 && (up == "EUR" || rates.contains_key(&up))).then_some(up)
}

fn per_eur(code: &str, rates: &Rates) -> Option<f64> {
    if code == "EUR" {
        Some(1.0)
    } else {
        rates.get(code).copied()
    }
}

// ── Quantities and conversions ──────────────────────────────────────────────

enum Measure {
    Unit(&'static Unit),
    Money(String),
}

const SYMBOL_PREFIXES: [char; 10] = ['$', '€', '£', '¥', '₹', '₩', '₺', '₪', '฿', '₽'];

/// "5 km", "5km", "$50", "50 usd" -> (5, km) etc.
fn quantity(text: &str, rates: Option<&Rates>) -> Option<(f64, Measure)> {
    let t = text.trim();
    let empty = Rates::new();
    let rates = rates.unwrap_or(&empty);
    let first = t.chars().next()?;
    if SYMBOL_PREFIXES.contains(&first) {
        let code = currency(&first.to_string(), rates)?;
        return Some((eval(&t[first.len_utf8()..])?, Measure::Money(code)));
    }
    let last = t.chars().last()?;
    if SYMBOL_PREFIXES.contains(&last) {
        let code = currency(&last.to_string(), rates)?;
        return Some((eval(&t[..t.len() - last.len_utf8()])?, Measure::Money(code)));
    }
    // The unit is the last word ("5 km", "3 fl oz") or the letters stuck to the number ("5km").
    let (expr, name) = match t.rfind(char::is_whitespace) {
        Some(i) if t[i + 1..].starts_with(|c: char| c.is_alphabetic() || c == '°' || c == '"' || c == '\'') => {
            let (mut e, mut n) = (t[..i].trim(), t[i + 1..].to_string());
            // Two-word units: "fl oz", "nautical miles".
            if let Some(j) = e.rfind(char::is_whitespace) {
                let two = format!("{}{}", &e[j + 1..], n);
                if unit(&two).is_some() {
                    n = two;
                    e = e[..j].trim();
                }
            }
            (e.to_string(), n)
        }
        _ => {
            let split = t
                .char_indices()
                .rev()
                .take_while(|(_, c)| c.is_alphabetic() || matches!(c, '°' | '/' | '"' | '\''))
                .last()
                .map(|(i, _)| i)?;
            (t[..split].trim().to_string(), t[split..].to_string())
        }
    };
    if expr.is_empty() {
        return None;
    }
    let v = eval(&expr)?;
    if let Some(u) = unit(&name) {
        return Some((v, Measure::Unit(u)));
    }
    Some((v, Measure::Money(currency(&name, rates)?)))
}

fn base_of(name: &str) -> Option<u32> {
    match name.trim().to_lowercase().as_str() {
        "hex" | "hexadecimal" => Some(16),
        "bin" | "binary" => Some(2),
        "oct" | "octal" => Some(8),
        "dec" | "decimal" => Some(10),
        _ => None,
    }
}

fn in_base(v: f64, base: u32) -> Option<String> {
    if v.fract() != 0.0 || v.abs() > 9.007e15 {
        return None;
    }
    let n = v as i64;
    let (sign, m) = if n < 0 { ("-", n.unsigned_abs()) } else { ("", n as u64) };
    Some(match base {
        16 => format!("{sign}0x{m:X}"),
        2 => format!("{sign}0b{m:b}"),
        8 => format!("{sign}0o{m:o}"),
        _ => format!("{n}"),
    })
}

fn convert(left: &str, right: &str, rates: Option<&Rates>) -> Option<Answer> {
    if let Some(base) = base_of(right) {
        let v = eval(left)?;
        let s = in_base(v, base)?;
        return Some(Answer { display: s.clone(), copy: s.clone(), detail: format!("{left} = {s}") });
    }
    let (v, from) = quantity(left, rates)?;
    match from {
        Measure::Unit(from) => {
            let to = unit(right)?;
            if to.dim != from.dim {
                return None;
            }
            unit_answer(v, from, to)
        }
        Measure::Money(code) => {
            let rates = rates?;
            let to = currency(right, rates)?;
            let out = v / per_eur(&code, rates)? * per_eur(&to, rates)?;
            let amount = format!("{out:.2}");
            let display = format!("{} {to}", group(&amount));
            Some(Answer { display: display.clone(), copy: amount, detail: format!("{} {code} = {display}", group(&plain(v)?)) })
        }
    }
}

fn unit_answer(v: f64, from: &Unit, to: &Unit) -> Option<Answer> {
    let out = short(convert_unit(v, from, to))?;
    let display = format!("{} {}", group(&out), to.symbol);
    Some(Answer { display: display.clone(), copy: out, detail: format!("{} {} = {display}", group(&short(v)?), from.symbol) })
}

/// "5 km" (a number and a unit, nothing else) -> miles.
fn default_conversion(text: &str) -> Option<Answer> {
    let (v, m) = quantity(text, None)?;
    let Measure::Unit(from) = m else { return None };
    let (_, to) = COUNTERPARTS.iter().find(|(f, _)| *f == from.symbol)?;
    unit_answer(v, from, unit_by_symbol(to)?)
}

/// Parse the ECB's daily reference rates (eurofxref-daily.xml): `currency='USD' rate='1.0812'`.
pub fn parse_ecb(xml: &str) -> (Rates, String) {
    let mut rates = Rates::new();
    let mut date = String::new();
    for part in xml.split("<Cube").skip(1) {
        let attr = |name: &str| {
            let key = format!("{name}='");
            let i = part.find(&key)? + key.len();
            let j = part[i..].find('\'')? + i;
            Some(part[i..j].to_string())
        };
        if let Some(t) = attr("time") {
            date = t;
        }
        if let (Some(c), Some(r)) = (attr("currency"), attr("rate").and_then(|r| r.parse::<f64>().ok())) {
            rates.insert(c, r);
        }
    }
    (rates, date)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(s: &str) -> Option<String> {
        answer(s, None).map(|a| a.display)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(show("2+2").as_deref(), Some("4"));
        assert_eq!(show("2^10").as_deref(), Some("1,024"));
        assert_eq!(show("(1+2)*3").as_deref(), Some("9"));
        assert_eq!(show("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(show("10/4").as_deref(), Some("2.5"));
        assert_eq!(show("-3^2").as_deref(), Some("-9"));
        assert_eq!(show("2^3^2").as_deref(), Some("512"));
        assert_eq!(show("5!").as_deref(), Some("120"));
        assert_eq!(show("17 mod 5").as_deref(), Some("2"));
        assert_eq!(show("sqrt(16)").as_deref(), Some("4"));
        assert_eq!(show("sqrt 2").as_deref(), Some("1.41421356237"));
        assert_eq!(show("2pi").as_deref(), Some("6.28318530718"));
        assert_eq!(show("max(3, 9, 4)").as_deref(), Some("9"));
        assert_eq!(show("1,000 * 3").as_deref(), Some("3,000"));
        assert_eq!(show("3 × 4 ÷ 2").as_deref(), Some("6"));
        assert_eq!(show("1e3 + 1").as_deref(), Some("1,001"));
        assert_eq!(show("2^100").as_deref(), Some("1.267651e+30"));
        assert_eq!(show("pi").as_deref(), Some("3.14159265359"));
    }

    #[test]
    fn percentages() {
        assert_eq!(show("15% of 80").as_deref(), Some("12"));
        assert_eq!(show("120 + 15%").as_deref(), Some("138"));
        assert_eq!(show("200 - 10%").as_deref(), Some("180"));
        assert_eq!(show("50%").as_deref(), Some("0.5"));
    }

    #[test]
    fn not_calculations() {
        for s in ["42", "-5", "safari", "e", "hello world", "1/0", "2 3", "visual studio code", "sqrt(", "3.1.4"] {
            assert_eq!(show(s), None, "{s}");
        }
    }

    #[test]
    fn units() {
        assert_eq!(show("5 km in mi").as_deref(), Some("3.10686 mi"));
        assert_eq!(show("5km to miles").as_deref(), Some("3.10686 mi"));
        assert_eq!(show("100 c in f").as_deref(), Some("212 °F"));
        assert_eq!(show("70f").as_deref(), Some("21.1111 °C"));
        assert_eq!(show("0 k in c").as_deref(), Some("-273.15 °C"));
        assert_eq!(show("3 cups to ml").as_deref(), Some("709.765 ml"));
        assert_eq!(show("2 fl oz in ml").as_deref(), Some("59.1471 ml"));
        assert_eq!(show("1 gib in mb").as_deref(), Some("1,073.74 MB"));
        assert_eq!(show("90 min in h").as_deref(), Some("1.5 h"));
        assert_eq!(show("100 kph to mph").as_deref(), Some("62.1371 mph"));
        assert_eq!(show("1 acre in m2").as_deref(), Some("4,046.86 m²"));
        assert_eq!(show("2*3 kg in lb").as_deref(), Some("13.2277 lb"));
        assert_eq!(show("1 km in kg"), None, "different dimensions");
        let a = answer("5 km", None).unwrap();
        assert_eq!((a.display.as_str(), a.copy.as_str(), a.detail.as_str()), ("3.10686 mi", "3.10686", "5 km = 3.10686 mi"));
        assert_eq!(show("1000000 m in km").as_deref(), Some("1,000 km"));
    }

    #[test]
    fn bases() {
        assert_eq!(show("255 in hex").as_deref(), Some("0xFF"));
        assert_eq!(show("10 in binary").as_deref(), Some("0b1010"));
        assert_eq!(show("0xff").as_deref(), Some("255"));
        assert_eq!(show("0b1010 + 1").as_deref(), Some("11"));
        assert_eq!(show("0x10 in dec").as_deref(), Some("16"));
        assert_eq!(show("1.5 in hex"), None);
    }

    #[test]
    fn currencies() {
        let (rates, date) = parse_ecb(
            "<Cube><Cube time='2026-10-02'><Cube currency='USD' rate='1.1000'/><Cube currency='GBP' rate='0.8500'/><Cube currency='JPY' rate='160.00'/></Cube></Cube>",
        );
        assert_eq!(date, "2026-10-02");
        assert_eq!(rates.len(), 3);
        let a = |s: &str| answer(s, Some(&rates)).map(|a| a.display);
        assert_eq!(a("100 usd to eur").as_deref(), Some("90.91 EUR"));
        assert_eq!(a("€50 in $").as_deref(), Some("55.00 USD"));
        assert_eq!(a("10 gbp in usd").as_deref(), Some("12.94 USD"));
        assert_eq!(a("1000 yen to euros").as_deref(), Some("6.25 EUR"));
        assert_eq!(a("100 usd to xyz"), None);
        assert_eq!(answer("100 usd to eur", None), None, "no rates, no answer");
    }

    #[test]
    fn grouping() {
        assert_eq!(group("123"), "123");
        assert_eq!(group("1234"), "1,234");
        assert_eq!(group("12345"), "12,345");
        assert_eq!(group("-1234567.25"), "-1,234,567.25");
        assert_eq!(group("1.2e+20"), "1.2e+20");
    }
}
