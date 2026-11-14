//! A script's text into a tree: words, numbers and signs first, then the
//! statements and expressions they make. Every node keeps where it was written,
//! so an error can say the line and column. The text is bounded before any of
//! this, and so is how deep expressions and blocks nest, so no script can run
//! the parser out of stack.
use crate::{Error, MAX_ARGS, MAX_DEPTH, MAX_TEXT, MAX_TOKENS, MAX_TREE};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct At {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Word(String),
    Text(String),
    /// `#rrggbb` and its kin, as `0xRRGGBBAA`.
    Hex(u32),
    Sign(&'static str),
    End,
}

const SIGNS: [&str; 23] = ["==", "!=", "<=", ">=", "(", ")", "[", "]", "{", "}", ",", "?", ":", "+", "-", "*", "/", "%", "<", ">", "=", ";", "."];

/// The powers of ten a double holds exactly.
const TENS: [f64; 23] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22];

/// `12.5` as a number: its digits, a whole number a double holds exactly, over a
/// power of ten it also holds, so the one division rounds it right — as the
/// standard parser would, without its tables.
fn decimal(text: &str) -> Option<f64> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    if frac.contains('.') || whole.len() + frac.len() == 0 {
        return None;
    }
    let mut digits: u64 = 0;
    for b in whole.bytes().chain(frac.bytes()) {
        digits = digits.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
    }
    if digits >= 1 << 53 {
        return None;
    }
    Some(digits as f64 / TENS.get(frac.len())?)
}

/// `rgb`, `rrggbb` or `rrggbbaa` in hex, as `0xRRGGBBAA`.
pub fn hex_colour(digits: &str) -> Option<u32> {
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        3 => {
            let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
            Some((r * 17) << 24 | (g * 17) << 16 | (b * 17) << 8 | 255)
        }
        6 => Some(v << 8 | 255),
        8 => Some(v),
        _ => None,
    }
}

fn lex(src: &str) -> Result<Vec<(Tok, At)>, Error> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    // Inside round brackets a statement runs on to the next line.
    let mut round = 0usize;
    while i < chars.len() {
        if out.len() > MAX_TOKENS {
            return Err(Error::at(At { line, col }, format!("a script is {MAX_TOKENS} words and signs at most")));
        }
        let c = chars[i];
        let at = At { line, col };
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            if round == 0 {
                out.push((Tok::End, at));
            }
            (i, line, col) = (i + 1, line + 1, 1);
            continue;
        }
        if c.is_whitespace() {
            (i, col) = (i + 1, col + 1);
            continue;
        }
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && next.is_some_and(|n| n.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let Some(v) = decimal(&text) else { return Err(Error::at(at, format!("{text} is not a number Frost Script holds: digits and one point, 15 digits at most"))) };
            col += (i - start) as u32;
            out.push((Tok::Num(v), at));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i - start > 40 {
                return Err(Error::at(at, "a name is 40 letters at most"));
            }
            col += (i - start) as u32;
            out.push((Tok::Word(chars[start..i].iter().collect()), at));
            continue;
        }
        if c == '#' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            let Some(v) = hex_colour(&digits) else { return Err(Error::at(at, "a colour is written #rrggbb, #rgb or #rrggbbaa")) };
            col += (i - start + 1) as u32;
            out.push((Tok::Hex(v), at));
            continue;
        }
        if c == '"' {
            i += 1;
            let mut text = String::new();
            let mut closed = false;
            let begun = i;
            while i < chars.len() && chars[i] != '\n' {
                match (chars[i], chars.get(i + 1)) {
                    ('"', _) => {
                        closed = true;
                        break;
                    }
                    // A line break, a quote and a backslash, written inside a text.
                    ('\\', Some('n')) => text.push('\n'),
                    ('\\', Some('"')) => text.push('"'),
                    ('\\', Some('\\')) => text.push('\\'),
                    (ch, _) => {
                        text.push(ch);
                        i += 1;
                        continue;
                    }
                }
                i += 2;
            }
            if !closed {
                return Err(Error::at(at, "this text has no closing \""));
            }
            if text.chars().count() > MAX_TEXT {
                return Err(Error::at(at, format!("a text is {MAX_TEXT} characters at most")));
            }
            col += (i - begun + 2) as u32;
            out.push((Tok::Text(text), at));
            i += 1;
            continue;
        }
        let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let Some(sign) = SIGNS.iter().find(|s| two.starts_with(**s)) else {
            return Err(Error::at(at, format!("{c} is not part of Frost Script")));
        };
        match *sign {
            "(" => round += 1,
            ")" => round = round.saturating_sub(1),
            _ => {}
        }
        i += sign.len();
        col += sign.len() as u32;
        out.push((Tok::Sign(sign), at));
    }
    out.push((Tok::End, At { line, col }));
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Text(String),
    Colour(u32),
    Name(String),
    /// A name's value `back` bars ago: `close[1]`.
    Back(String, Box<Node>),
    Neg(Box<Node>),
    Not(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
    Pick(Box<Node>, Box<Node>, Box<Node>),
    /// A call: its name, what it is given in order, and what by name (`width=2`).
    /// `xs.push(v)` is `.push` given `xs` first.
    Call(String, Vec<Node>, Vec<(String, Node)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub expr: Expr,
    pub at: At,
    /// How many nodes deep it is, itself counted: never past MAX_TREE.
    pub depth: u16,
}

impl Node {
    /// A node over what it holds, or an error where it would be too deep. Each
    /// is counted as it is made, so no tree past the limit is ever built: a chain
    /// of `+` or `and` is as deep as it is long.
    fn new(at: At, expr: Expr) -> Result<Node, Error> {
        let below = match &expr {
            Expr::Num(_) | Expr::Text(_) | Expr::Colour(_) | Expr::Name(_) => 0,
            Expr::Back(_, n) | Expr::Neg(n) | Expr::Not(n) => n.depth,
            Expr::Binary(_, a, b) => a.depth.max(b.depth),
            Expr::Pick(c, a, b) => c.depth.max(a.depth).max(b.depth),
            Expr::Call(_, args, named) => args.iter().chain(named.iter().map(|(_, n)| n)).map(|n| n.depth).max().unwrap_or(0),
        };
        if usize::from(below) >= MAX_TREE {
            return Err(Error::at(at, format!("this is more than {MAX_TREE} deep: give parts of it names of their own first")));
        }
        Ok(Node { expr, at, depth: below + 1 })
    }
}

/// What a setting starts as, which says what kind it is.
#[derive(Clone, Debug, PartialEq)]
pub enum Default {
    /// A number, `true` or `false`, a colour, or `date(…)`.
    Value(Node),
    /// One of a list of texts: `"small" from "tiny", "small", "large"`.
    Choice(String, Vec<String>),
}

/// A call's arguments: those in order, then those by name.
type Args = (Vec<Node>, Vec<(String, Node)>);

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub name: String,
    pub default: Default,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Pane(bool),
    /// Where the table sits: down (0 top, 1 middle, 2 bottom) and across (0 left, 1 centre, 2 right).
    Table(u8, u8),
    Input(Input),
    Set { name: String, value: Node, keep: bool },
    /// `a, b = f(x)`: what a function gives back, each to its name.
    SetMany { names: Vec<String>, value: Node },
    If { arms: Vec<(Node, Vec<Line>)>, otherwise: Vec<Line> },
    For { name: String, from: Node, to: Node, by: Option<Node>, body: Vec<Line> },
    While { cond: Node, body: Vec<Line> },
    Break,
    Continue,
    Fn { name: String, params: Vec<String>, body: Vec<Line> },
    Return(Vec<Node>),
    Do(Node),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub stmt: Stmt,
    pub at: At,
}

struct Parser {
    toks: Vec<(Tok, At)>,
    i: usize,
    depth: usize,
}

pub fn parse(src: &str) -> Result<Vec<Line>, Error> {
    let mut p = Parser { toks: lex(src)?, i: 0, depth: 0 };
    let lines = p.lines(false)?;
    Ok(lines)
}

impl Parser {
    fn peek(&self) -> &Tok {
        self.toks.get(self.i).map_or(&Tok::End, |t| &t.0)
    }

    fn at(&self) -> At {
        self.toks.get(self.i).map_or(At { line: 1, col: 1 }, |t| t.1)
    }

    /// Where the word just read was.
    fn was(&self) -> At {
        self.toks.get(self.i.saturating_sub(1)).map_or(self.at(), |t| t.1)
    }

    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        self.i += 1;
        t
    }

    fn is_sign(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sign(x) if *x == s)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Word(x) if x == w)
    }

    fn take_word(&mut self, w: &str) -> bool {
        let is = self.is_word(w);
        if is {
            self.i += 1;
        }
        is
    }

    fn expect_sign(&mut self, s: &str) -> Result<(), Error> {
        if self.is_sign(s) {
            self.i += 1;
            Ok(())
        } else {
            Err(Error::at(self.at(), format!("{s} was expected here")))
        }
    }

    fn name(&mut self) -> Result<String, Error> {
        match self.bump() {
            Tok::Word(w) if !crate::compile::is_keyword(&w) => Ok(w),
            _ => Err(Error::at(self.was(), "a name was expected here")),
        }
    }

    /// Deeper by one, and an error past the limit: what stops a script of a thousand brackets.
    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(Error::at(self.at(), format!("nested more than {MAX_DEPTH} deep"))) } else { Ok(()) }
    }

    fn skip_ends(&mut self) {
        while matches!(self.peek(), Tok::End) && self.i + 1 < self.toks.len() || self.is_sign(";") {
            self.i += 1;
        }
    }

    fn lines(&mut self, block: bool) -> Result<Vec<Line>, Error> {
        let mut out = Vec::new();
        loop {
            self.skip_ends();
            if block && self.is_sign("}") {
                self.i += 1;
                return Ok(out);
            }
            if self.i + 1 >= self.toks.len() {
                if block {
                    return Err(Error::at(self.at(), "this block has no closing }"));
                }
                return Ok(out);
            }
            let at = self.at();
            let stmt = self.stmt()?;
            out.push(Line { stmt, at });
            if !(matches!(self.peek(), Tok::End) || self.is_sign(";") || (block && self.is_sign("}"))) {
                return Err(Error::at(self.at(), "one statement a line: this one goes on"));
            }
        }
    }

    fn block(&mut self) -> Result<Vec<Line>, Error> {
        self.enter()?;
        self.expect_sign("{")?;
        let lines = self.lines(true)?;
        self.depth -= 1;
        Ok(lines)
    }

    fn signed(&mut self) -> Result<f64, Error> {
        let neg = self.is_sign("-");
        if neg {
            self.i += 1;
        }
        match self.bump() {
            Tok::Num(v) => Ok(if neg { -v } else { v }),
            _ => Err(Error::at(self.was(), "a number was expected here")),
        }
    }

    fn input(&mut self) -> Result<Stmt, Error> {
        let name = self.name()?;
        self.expect_sign("=")?;
        let default = if let Tok::Text(first) = self.peek().clone() {
            self.i += 1;
            if !self.take_word("from") {
                return Err(Error::at(self.at(), "a choice names what it is one of: input size = \"small\" from \"tiny\", \"small\", \"large\""));
            }
            let mut options = Vec::new();
            loop {
                match self.bump() {
                    Tok::Text(t) => options.push(t),
                    _ => return Err(Error::at(self.was(), "a choice in quotes was expected here")),
                }
                if !self.is_sign(",") {
                    break;
                }
                self.i += 1;
            }
            Default::Choice(first, options)
        } else {
            Default::Value(self.unary()?)
        };
        let mut input = Input { name, default, min: None, max: None, step: None, label: None };
        loop {
            if self.take_word("min") {
                input.min = Some(self.signed()?);
            } else if self.take_word("max") {
                input.max = Some(self.signed()?);
            } else if self.take_word("step") {
                input.step = Some(self.signed()?);
            } else if let Tok::Text(t) = self.peek().clone() {
                self.i += 1;
                input.label = Some(t);
            } else {
                break;
            }
        }
        Ok(Stmt::Input(input))
    }

    fn stmt(&mut self) -> Result<Stmt, Error> {
        if self.take_word("pane") {
            return match self.bump() {
                Tok::Word(w) if w == "own" => Ok(Stmt::Pane(true)),
                Tok::Word(w) if w == "price" => Ok(Stmt::Pane(false)),
                _ => Err(Error::at(self.was(), "pane own or pane price")),
            };
        }
        if self.is_word("table") && !matches!(self.toks.get(self.i + 1), Some((Tok::Sign(_), _))) {
            self.i += 1;
            let (mut down, mut across) = (0u8, 2u8);
            while let Tok::Word(w) = self.peek().clone() {
                match w.as_str() {
                    "top" => down = 0,
                    "middle" => down = 1,
                    "bottom" => down = 2,
                    "left" => across = 0,
                    "center" | "centre" => across = 1,
                    "right" => across = 2,
                    _ => return Err(Error::at(self.at(), "table top right: top, middle or bottom, then left, center or right")),
                }
                self.i += 1;
            }
            return Ok(Stmt::Table(down, across));
        }
        if self.take_word("input") {
            return self.input();
        }
        if self.take_word("state") {
            let name = self.name()?;
            self.expect_sign("=")?;
            return Ok(Stmt::Set { name, value: self.expr()?, keep: true });
        }
        if self.take_word("if") {
            let mut arms = vec![(self.expr()?, self.block()?)];
            let mut otherwise = Vec::new();
            loop {
                // `else` may sit on the line after the closing brace.
                let back = self.i;
                self.skip_ends();
                if !self.take_word("else") {
                    self.i = back;
                    break;
                }
                if self.take_word("if") {
                    arms.push((self.expr()?, self.block()?));
                } else {
                    otherwise = self.block()?;
                    break;
                }
            }
            return Ok(Stmt::If { arms, otherwise });
        }
        if self.take_word("for") {
            let name = self.name()?;
            self.expect_sign("=")?;
            let from = self.expr()?;
            if !self.take_word("to") {
                return Err(Error::at(self.at(), "for i = 1 to 10"));
            }
            let to = self.expr()?;
            let by = if self.take_word("by") { Some(self.expr()?) } else { None };
            return Ok(Stmt::For { name, from, to, by, body: self.block()? });
        }
        if self.take_word("while") {
            return Ok(Stmt::While { cond: self.expr()?, body: self.block()? });
        }
        if self.take_word("break") {
            return Ok(Stmt::Break);
        }
        if self.take_word("continue") {
            return Ok(Stmt::Continue);
        }
        if self.take_word("fn") {
            let name = self.name()?;
            self.expect_sign("(")?;
            let mut params = Vec::new();
            while !self.is_sign(")") {
                params.push(self.name()?);
                if params.len() > MAX_ARGS {
                    return Err(Error::at(self.at(), format!("{MAX_ARGS} arguments at most")));
                }
                if !self.is_sign(",") {
                    break;
                }
                self.i += 1;
            }
            self.expect_sign(")")?;
            return Ok(Stmt::Fn { name, params, body: self.block()? });
        }
        if self.take_word("return") {
            let mut values = Vec::new();
            if !(matches!(self.peek(), Tok::End) || self.is_sign(";") || self.is_sign("}")) {
                loop {
                    values.push(self.expr()?);
                    if values.len() > 8 {
                        return Err(Error::at(self.at(), "a function gives back 8 values at most"));
                    }
                    if !self.is_sign(",") {
                        break;
                    }
                    self.i += 1;
                }
            }
            return Ok(Stmt::Return(values));
        }
        if let (Tok::Word(w), Some((Tok::Sign(next), _))) = (self.peek().clone(), self.toks.get(self.i + 1)) {
            if *next == "=" {
                if crate::compile::is_keyword(&w) {
                    return Err(Error::at(self.at(), format!("{w} is a word of the language, not a name")));
                }
                self.i += 2;
                return Ok(Stmt::Set { name: w, value: self.expr()?, keep: false });
            }
            // `a, b = f(x)`: names, commas, then the sign.
            if *next == "," {
                let back = self.i;
                let mut names = vec![self.name()?];
                while self.is_sign(",") {
                    self.i += 1;
                    match self.peek() {
                        Tok::Word(_) => names.push(self.name()?),
                        _ => break,
                    }
                }
                if self.is_sign("=") && names.len() <= 8 {
                    self.i += 1;
                    return Ok(Stmt::SetMany { names, value: self.expr()? });
                }
                self.i = back;
            }
        }
        Ok(Stmt::Do(self.expr()?))
    }

    /// What a call is given, to its closing bracket: in order, then by name, as `plot(x, "T", width=2)`.
    fn args(&mut self, at: At) -> Result<Args, Error> {
        let (mut args, mut named) = (Vec::new(), Vec::new());
        while !self.is_sign(")") {
            if let (Tok::Word(w), Some((Tok::Sign("="), _))) = (self.peek().clone(), self.toks.get(self.i + 1)) {
                self.i += 2;
                named.push((w, self.expr()?));
            } else if named.is_empty() {
                args.push(self.expr()?);
            } else {
                return Err(Error::at(self.at(), "what is given by name comes after what is given in order"));
            }
            if args.len() + named.len() > MAX_ARGS {
                return Err(Error::at(at, format!("{MAX_ARGS} arguments at most")));
            }
            if !self.is_sign(",") {
                break;
            }
            self.i += 1;
        }
        self.expect_sign(")")?;
        Ok((args, named))
    }

    fn expr(&mut self) -> Result<Node, Error> {
        self.enter()?;
        let cond = self.or()?;
        let node = if self.is_sign("?") {
            self.i += 1;
            let yes = self.expr()?;
            self.expect_sign(":")?;
            let no = self.expr()?;
            Node::new(cond.at, Expr::Pick(Box::new(cond), Box::new(yes), Box::new(no)))?
        } else {
            cond
        };
        self.depth -= 1;
        Ok(node)
    }

    fn or(&mut self) -> Result<Node, Error> {
        let mut left = self.and()?;
        while self.take_word("or") {
            let right = self.and()?;
            left = Node::new(left.at, Expr::Binary(Op::Or, Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, Error> {
        let mut left = self.not()?;
        while self.take_word("and") {
            let right = self.not()?;
            left = Node::new(left.at, Expr::Binary(Op::And, Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Node, Error> {
        if self.is_word("not") {
            let at = self.at();
            self.i += 1;
            self.enter()?;
            let inner = self.not()?;
            self.depth -= 1;
            return Node::new(at, Expr::Not(Box::new(inner)));
        }
        self.compare()
    }

    fn compare(&mut self) -> Result<Node, Error> {
        let left = self.sum()?;
        let op = match self.peek() {
            Tok::Sign("==") => Op::Eq,
            Tok::Sign("!=") => Op::Ne,
            Tok::Sign("<") => Op::Lt,
            Tok::Sign("<=") => Op::Le,
            Tok::Sign(">") => Op::Gt,
            Tok::Sign(">=") => Op::Ge,
            _ => return Ok(left),
        };
        self.i += 1;
        let right = self.sum()?;
        Node::new(left.at, Expr::Binary(op, Box::new(left), Box::new(right)))
    }

    fn sum(&mut self) -> Result<Node, Error> {
        let mut left = self.product()?;
        loop {
            let op = match self.peek() {
                Tok::Sign("+") => Op::Add,
                Tok::Sign("-") => Op::Sub,
                _ => return Ok(left),
            };
            self.i += 1;
            let right = self.product()?;
            left = Node::new(left.at, Expr::Binary(op, Box::new(left), Box::new(right)))?;
        }
    }

    fn product(&mut self) -> Result<Node, Error> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Sign("*") => Op::Mul,
                Tok::Sign("/") => Op::Div,
                Tok::Sign("%") => Op::Rem,
                _ => return Ok(left),
            };
            self.i += 1;
            let right = self.unary()?;
            left = Node::new(left.at, Expr::Binary(op, Box::new(left), Box::new(right)))?;
        }
    }

    fn unary(&mut self) -> Result<Node, Error> {
        if self.is_sign("-") {
            let at = self.at();
            self.i += 1;
            self.enter()?;
            let inner = self.unary()?;
            self.depth -= 1;
            return Node::new(at, Expr::Neg(Box::new(inner)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, Error> {
        let at = self.at();
        match self.bump() {
            Tok::Num(v) => Node::new(at, Expr::Num(v)),
            Tok::Text(t) => Node::new(at, Expr::Text(t)),
            Tok::Hex(v) => Node::new(at, Expr::Colour(v)),
            Tok::Sign("(") => {
                let inner = self.expr()?;
                self.expect_sign(")")?;
                Ok(inner)
            }
            Tok::Word(w) => {
                if self.is_sign("(") {
                    self.i += 1;
                    let (args, named) = self.args(at)?;
                    return Node::new(at, Expr::Call(w, args, named));
                }
                // `xs.push(v)`: a list's own function, given the list first.
                if self.is_sign(".") {
                    self.i += 1;
                    let method = self.name()?;
                    if !self.is_sign("(") {
                        return Err(Error::at(self.at(), format!("{w}.{method}(…): a list's functions take brackets")));
                    }
                    self.i += 1;
                    let (mut args, named) = self.args(at)?;
                    args.insert(0, Node::new(at, Expr::Name(w))?);
                    return Node::new(at, Expr::Call(format!(".{method}"), args, named));
                }
                if self.is_sign("[") {
                    self.i += 1;
                    let back = self.expr()?;
                    self.expect_sign("]")?;
                    return Node::new(at, Expr::Back(w, Box::new(back)));
                }
                Node::new(at, Expr::Name(w))
            }
            Tok::End => Err(Error::at(at, "the line ends where a value was expected")),
            Tok::Sign(s) => Err(Error::at(at, format!("{s} is not where a value starts"))),
        }
    }
}
