//! A formula's text into a tree: words, numbers and signs first, then the
//! statements and expressions they make. Every node keeps where it was written,
//! so an error can say the line and column. The text is bounded before any of
//! this, and so is how deep expressions and blocks nest, so no formula can run
//! the parser out of stack.
use crate::{Error, MAX_DEPTH, MAX_TOKENS};

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
    Sign(&'static str),
    End,
}

const SIGNS: [&str; 22] = ["==", "!=", "<=", ">=", "(", ")", "[", "]", "{", "}", ",", "?", ":", "+", "-", "*", "/", "%", "<", ">", "=", ";"];

fn lex(src: &str) -> Result<Vec<(Tok, At)>, Error> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    while i < chars.len() {
        if out.len() > MAX_TOKENS {
            return Err(Error::at(At { line, col }, format!("a script is {MAX_TOKENS} words and signs at most")));
        }
        let c = chars[i];
        let at = At { line, col };
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            out.push((Tok::End, at));
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
            let Ok(v) = text.parse::<f64>() else { return Err(Error::at(at, format!("{text} is not a number"))) };
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
        if c == '"' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != '"' && chars[i] != '\n' {
                i += 1;
            }
            if chars.get(i) != Some(&'"') {
                return Err(Error::at(at, "this text has no closing \""));
            }
            if i - start > 40 {
                return Err(Error::at(at, "a title is 40 characters at most"));
            }
            col += (i + 1 - start + 1) as u32;
            out.push((Tok::Text(chars[start..i].iter().collect()), at));
            i += 1;
            continue;
        }
        let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let Some(sign) = SIGNS.iter().find(|s| two.starts_with(**s)) else {
            return Err(Error::at(at, format!("{c} is not part of Frost Script")));
        };
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
    Name(String),
    /// A name's value `back` bars ago: `close[1]`.
    Back(String, Box<Node>),
    Neg(Box<Node>),
    Not(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
    Pick(Box<Node>, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub expr: Expr,
    pub at: At,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub name: String,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Pane(bool),
    Input(Input),
    Set { name: String, value: Node, keep: bool },
    If { arms: Vec<(Node, Vec<Line>)>, otherwise: Vec<Line> },
    For { name: String, from: Node, to: Node, body: Vec<Line> },
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
            _ => Err(Error::at(self.toks.get(self.i - 1).map_or(self.at(), |t| t.1), "a name was expected here")),
        }
    }

    /// Deeper by one, and an error past the limit: what stops a formula of a thousand brackets.
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
            _ => Err(Error::at(self.toks.get(self.i - 1).map_or(self.at(), |t| t.1), "a number was expected here")),
        }
    }

    fn stmt(&mut self) -> Result<Stmt, Error> {
        if self.is_word("pane") {
            self.i += 1;
            return match self.bump() {
                Tok::Word(w) if w == "own" => Ok(Stmt::Pane(true)),
                Tok::Word(w) if w == "price" => Ok(Stmt::Pane(false)),
                _ => Err(Error::at(self.toks.get(self.i - 1).map_or(self.at(), |t| t.1), "pane own or pane price")),
            };
        }
        if self.is_word("input") {
            self.i += 1;
            let name = self.name()?;
            self.expect_sign("=")?;
            let default = self.signed()?;
            let mut input = Input { name, default, min: None, max: None, step: None, label: None };
            loop {
                if self.is_word("min") {
                    self.i += 1;
                    input.min = Some(self.signed()?);
                } else if self.is_word("max") {
                    self.i += 1;
                    input.max = Some(self.signed()?);
                } else if self.is_word("step") {
                    self.i += 1;
                    input.step = Some(self.signed()?);
                } else if let Tok::Text(t) = self.peek().clone() {
                    self.i += 1;
                    input.label = Some(t);
                } else {
                    break;
                }
            }
            return Ok(Stmt::Input(input));
        }
        if self.is_word("state") {
            self.i += 1;
            let name = self.name()?;
            self.expect_sign("=")?;
            return Ok(Stmt::Set { name, value: self.expr()?, keep: true });
        }
        if self.is_word("if") {
            self.i += 1;
            let mut arms = vec![(self.expr()?, self.block()?)];
            let mut otherwise = Vec::new();
            loop {
                // `else` may sit on the line after the closing brace.
                let back = self.i;
                self.skip_ends();
                if !self.is_word("else") {
                    self.i = back;
                    break;
                }
                self.i += 1;
                if self.is_word("if") {
                    self.i += 1;
                    arms.push((self.expr()?, self.block()?));
                } else {
                    otherwise = self.block()?;
                    break;
                }
            }
            return Ok(Stmt::If { arms, otherwise });
        }
        if self.is_word("for") {
            self.i += 1;
            let name = self.name()?;
            self.expect_sign("=")?;
            let from = self.expr()?;
            if !self.is_word("to") {
                return Err(Error::at(self.at(), "for i = 1 to 10"));
            }
            self.i += 1;
            let to = self.expr()?;
            return Ok(Stmt::For { name, from, to, body: self.block()? });
        }
        if let (Tok::Word(w), Some((Tok::Sign("="), _))) = (self.peek().clone(), self.toks.get(self.i + 1)) {
            if crate::compile::is_keyword(&w) {
                return Err(Error::at(self.at(), format!("{w} is a word of the language, not a name")));
            }
            self.i += 2;
            return Ok(Stmt::Set { name: w, value: self.expr()?, keep: false });
        }
        Ok(Stmt::Do(self.expr()?))
    }

    fn expr(&mut self) -> Result<Node, Error> {
        self.enter()?;
        let cond = self.or()?;
        let node = if self.is_sign("?") {
            self.i += 1;
            let yes = self.expr()?;
            self.expect_sign(":")?;
            let no = self.expr()?;
            Node { at: cond.at, expr: Expr::Pick(Box::new(cond), Box::new(yes), Box::new(no)) }
        } else {
            cond
        };
        self.depth -= 1;
        Ok(node)
    }

    fn or(&mut self) -> Result<Node, Error> {
        let mut left = self.and()?;
        while self.is_word("or") {
            self.i += 1;
            let right = self.and()?;
            left = Node { at: left.at, expr: Expr::Binary(Op::Or, Box::new(left), Box::new(right)) };
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, Error> {
        let mut left = self.not()?;
        while self.is_word("and") {
            self.i += 1;
            let right = self.not()?;
            left = Node { at: left.at, expr: Expr::Binary(Op::And, Box::new(left), Box::new(right)) };
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
            return Ok(Node { at, expr: Expr::Not(Box::new(inner)) });
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
        Ok(Node { at: left.at, expr: Expr::Binary(op, Box::new(left), Box::new(right)) })
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
            left = Node { at: left.at, expr: Expr::Binary(op, Box::new(left), Box::new(right)) };
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
            left = Node { at: left.at, expr: Expr::Binary(op, Box::new(left), Box::new(right)) };
        }
    }

    fn unary(&mut self) -> Result<Node, Error> {
        if self.is_sign("-") {
            let at = self.at();
            self.i += 1;
            self.enter()?;
            let inner = self.unary()?;
            self.depth -= 1;
            return Ok(Node { at, expr: Expr::Neg(Box::new(inner)) });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, Error> {
        let at = self.at();
        match self.bump() {
            Tok::Num(v) => Ok(Node { at, expr: Expr::Num(v) }),
            Tok::Text(t) => Ok(Node { at, expr: Expr::Text(t) }),
            Tok::Sign("(") => {
                let inner = self.expr()?;
                self.expect_sign(")")?;
                Ok(inner)
            }
            Tok::Word(w) => {
                if self.is_sign("(") {
                    self.i += 1;
                    let mut args = Vec::new();
                    if !self.is_sign(")") {
                        loop {
                            args.push(self.expr()?);
                            if args.len() > 8 {
                                return Err(Error::at(at, "8 arguments at most"));
                            }
                            if self.is_sign(",") {
                                self.i += 1;
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_sign(")")?;
                    return Ok(Node { at, expr: Expr::Call(w, args) });
                }
                if self.is_sign("[") {
                    self.i += 1;
                    let back = self.expr()?;
                    self.expect_sign("]")?;
                    return Ok(Node { at, expr: Expr::Back(w, Box::new(back)) });
                }
                Ok(Node { at, expr: Expr::Name(w) })
            }
            Tok::End => Err(Error::at(at, "the line ends where a value was expected")),
            Tok::Sign(s) => Err(Error::at(at, format!("{s} is not where a value starts"))),
        }
    }
}
