use anyhow;
use regex::{Captures, Regex, RegexSet};

#[cfg(test)]
use std::cell::RefCell;

pub struct AMatch<'a> {
    token: u32,
    captures: Captures<'a>,
}

pub struct InputHandler<'a> {
    anything: Option<RegexSet>,
    regexes: Vec<Regex>,
    patterns: Vec<String>,
    tokens: Vec<u32>,
    callbacks: Vec<Box<dyn FnMut(&str, &Captures) -> () + 'a>>,
}

fn nop(_input: &str, _dummy: &Captures) -> () {
}

impl<'a> InputHandler<'a> {
    pub fn new() -> Self {
        Self {
            anything: None,
            regexes: vec![],
            patterns: vec![],
            tokens: vec![],
            callbacks: vec![],
        }
    }

    fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: impl FnMut(&str, &Captures) + 'a) -> anyhow::Result<()> {
        let r = Regex::new(pattern)?;
        self.patterns.push(pattern.to_string());
        self.regexes.push(r);
        self.tokens.push(token);
        self.callbacks.push(Box::new(callback));
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
            let anything = RegexSet::new(self.patterns.iter())?;
            self.anything = Some(anything);
        }
        Ok(())
    }

    pub fn test<'b>(&mut self, input: &'b str) -> anyhow::Result<Option<AMatch<'b>>> {
        self.ensure()?;
        let matches = self.anything.as_ref().unwrap().matches(input);
        for index in matches {
            let regex = &self.regexes[index];
            if let Some(caps) = regex.captures(input) {
                let amatch = AMatch {
                    token: self.tokens[index],
                    captures: caps,
                };
                self.callbacks[index](input, &amatch.captures);
                return Ok(Some(amatch));
            }
        }
        Ok(None)
    }
}

#[test]
pub fn test() {
    struct TestState { first: String, second: String }
    let leaked_state = Box::leak(Box::new(RefCell::new(TestState {
        first: "".to_string(),
        second: "".to_string()
    })));

    let mut handler = InputHandler::new();

    let pattern = r"3a|5a";
    handler.add_pattern(1, &(r"(?:what is|literal) (?:0x)?((?:ff)*(?P<code>".to_string() + pattern + r"){1,16}(?:ff)*) ?\??$")).expect("invalid regex");

    handler.add_callback_pattern(r"test (\w+) (?P<second>\w+)", |_input: &str, caps: &Captures| {
        leaked_state.borrow_mut().first = caps.get(1).unwrap().as_str().to_string();
        leaked_state.borrow_mut().second = caps.name("second").unwrap().as_str().to_string();
    }).expect("adding pattern");
    let _ = handler.test("test stabby fish").expect("should match");
    assert_eq!(leaked_state.borrow().first.as_str(), "stabby");
    assert_eq!(leaked_state.borrow().second.as_str(), "fish");

    let result = handler.test("what is 0x3a3a3a3a?").expect("should match");
    let result = result.unwrap();
    assert_eq!(result.token, 1);
    assert_eq!(result.captures.name("code").expect("?P<code> should be found").as_str(), "3a");
    assert_eq!(result.captures.get(1).expect("m[1] should be found").as_str(), "3a3a3a3a");
    assert_eq!(result.captures.get(2).expect("m[2] should be found").as_str(), "3a");
}
