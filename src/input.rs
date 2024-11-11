use anyhow;
use regex::{Captures, Regex, RegexSet};

pub struct AMatch<'a> {
    token: u32,
    captures: Captures<'a>,
}

struct Listener<'a> {
    regex: Regex,
    pattern: String,
    token: u32,
    callback: Box<dyn FnMut(&str, &Captures) -> () + 'a>,
}

pub struct InputHandler<'a> {
    // RegexSet that checks whether any of the regexes match. Invalidated when new patterns are registered.
    anything: Option<RegexSet>,

    // Compiled regular expression.
    listeners: Vec<Listener<'a>>,
}

fn nop(_input: &str, _dummy: &Captures) -> () {
}

impl<'a> InputHandler<'a> {
    pub fn new() -> Self {
        Self {
            anything: None,
            listeners: vec![],
        }
    }

    fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: impl FnMut(&str, &Captures) + 'a) -> anyhow::Result<()> {
        let r = Regex::new(pattern)?;
        self.listeners.push(Listener {
            pattern: pattern.to_string(),
            regex: r,
            token: token,
            callback: Box::new(callback),
        });
        self.anything = None; // Invalidate the "does anything match" RegexSet.
        Ok(())
    }

    pub fn add_callback_pattern(&mut self, pattern: &str, callback: impl FnMut(&str, &Captures) + 'a) -> anyhow::Result<()> {
        self.add_full_pattern(0, pattern, callback)
    }

    pub fn add_pattern(&mut self, token: u32, pattern: &str) -> anyhow::Result<()> {
        self.add_full_pattern(token, pattern, Box::new(nop))
    }

    fn ensure(&mut self) -> anyhow::Result<()> {
        if self.anything.is_none() {
            let anything = RegexSet::new(self.listeners.iter().map(|l| &l.pattern))?;
            self.anything = Some(anything);
        }
        Ok(())
    }

    pub fn parse<'b>(&mut self, input: &'b str) -> anyhow::Result<Option<AMatch<'b>>> {
        self.ensure()?;
        let matches = self.anything.as_ref().unwrap().matches(input);
        for index in matches {
            let listener = &mut self.listeners[index];
            if let Some(caps) = listener.regex.captures(input) {
                let amatch = AMatch {
                    token: listener.token,
                    captures: caps,
                };
                (listener.callback)(input, &amatch.captures);
                return Ok(Some(amatch));
            }
        }
        Ok(None)
    }
}

#[test]
pub fn test() {
    use std::cell::RefCell;

    struct TestState { first: String, second: String }
    let state = RefCell::new(TestState {
        first: String::default(),
        second: String::default(),
    });

    // Note that this is within the scope of `state`, so that the callbacks here can store references to it.
    let mut handler = InputHandler::new();

    let pattern = r"3a|5a";
    handler.add_pattern(1, &(r"(?:what is|literal) (?:0x)?((?:ff)*(?P<code>".to_string() + pattern + r"){1,16}(?:ff)*) ?\??$")).expect("invalid regex");

    handler.add_callback_pattern(r"test (\w+) (?P<second>\w+)", |_input: &str, caps: &Captures| {
        state.borrow_mut().first = caps.get(1).unwrap().as_str().to_string();
        state.borrow_mut().second = caps.name("second").unwrap().as_str().to_string();
    }).expect("adding pattern");
    let _ = handler.parse("test stabby fish").expect("should match");
    assert_eq!(state.borrow().first.as_str(), "stabby");
    assert_eq!(state.borrow().second.as_str(), "fish");

    let result = handler.parse("what is 0x3a3a3a3a?").expect("should match");
    let result = result.unwrap();
    assert_eq!(result.token, 1);
    assert_eq!(result.captures.name("code").expect("?P<code> should be found").as_str(), "3a");
    assert_eq!(result.captures.get(1).expect("m[1] should be found").as_str(), "3a3a3a3a");
    assert_eq!(result.captures.get(2).expect("m[2] should be found").as_str(), "3a");
}
