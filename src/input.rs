use anyhow;
use regex::{Captures, Regex, RegexSet};
use pyo3::prelude::*;
use pyo3::{
    exceptions::{PyAttributeError, PyRuntimeError, PyIndexError, PyTypeError},
    types::PyTuple,
};
use std::sync::Arc;
//use pyo3_asyncio_0_21::tokio::future_into_py;
use std::{
    cell::RefCell,
    collections::HashMap,
    thread,
};

#[derive(Debug, Clone)]
pub struct RegisteredPattern {
    regex: Regex,
    pattern: String,
    requires_direct: bool,
    token: u32,
    defaults: HashMap<String, String>,
}

#[derive(Debug)]
enum Parameters<'a> {
    Caps(Captures<'a>),
    Vals(HashMap<String, String>),
}

#[derive(Debug)]
pub struct AMatch<'a> {
    index: usize,
    rpattern: RegisteredPattern,
    pub token: u32,
    captures: Parameters<'a>,
}

impl Default for InputHandler {
    fn default() -> Self {
        Self::new("(unnamed)")
    }
}

pub fn to_pyerr(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

#[derive(Debug)]
#[pyclass]
pub struct InputHandler {
    // RegexSet that checks whether any of the regexes match. Invalidated when new patterns are registered.
    anything: RefCell<Option<RegexSet>>,

    // Map command names to (one of) the indexes of the pattern and callback for that command name.
    commands: HashMap<String, usize>,

    command_matcher: RefCell<Option<Regex>>,

    // To distinguish direct commands ("botname: ...").
    name: String,

    // Parallel vectors in the same order as the regexes in `anything`.

    // Full info on the pattern.
    patterns: Vec<RegisteredPattern>,

    // Python callbacks.
    callbacks: Vec<Py<PyAny>>,

    command: Vec<String>,
}

impl InputHandler {
    pub fn new(name: &str) -> Self {
        Self {
            anything: RefCell::new(None),
            commands: HashMap::new(),
            command_matcher: RefCell::new(None),
            name: name.to_string(),
            patterns: vec![],
            callbacks: vec![],
            command: vec![],
        }
    }

    fn invalidate(&mut self) {
        *self.anything.borrow_mut() = None; // Invalidate the "does anything match" RegexSet.
        *self.command_matcher.borrow_mut() = None;
    }

    fn ensure(&self) -> anyhow::Result<()> {
        let mut a = self.anything.borrow_mut();
        //println!("ensure called with ms={:?} while patterns = {:?}", a, self.patterns);
        if a.is_none() {
            let pats = self.patterns.iter().map(|l| l.pattern.as_str());
            let cmds = "^!(".to_string() + &self.command.join("|") + ")(.*)$";
            let anything = RegexSet::new(pats.chain(std::iter::once(cmds.as_ref())))?;
            *a = Some(anything);
            *self.command_matcher.borrow_mut() = Some(Regex::new(cmds.as_ref())?);
        }
        Ok(())
    }

    pub fn add_pattern(&mut self, token: u32, command: &str, pattern: &str, defaults: Option<HashMap<String, String>>, direct_required: bool) -> anyhow::Result<()> {
        let r = Regex::new(pattern)?;
        self.commands.insert(command.to_string(), self.patterns.len());
        self.patterns.push(RegisteredPattern {
            pattern: pattern.to_string(),
            regex: r,
            requires_direct: direct_required,
            token,
            defaults: defaults.unwrap_or_default(),
        });
        self.command.push(command.to_string());
        *self.anything.borrow_mut() = None; // Invalidate the "does anything match" RegexSet.
        // println!("added pattern {}, patterns now = {:?}", pattern, self.patterns);
        Ok(())
    }

    pub fn match_input<'b>(&self, input: &'b str, is_direct: bool) -> anyhow::Result<Option<AMatch<'b>>> {
        self.ensure()?;
        let matches = self.anything.borrow().as_ref().unwrap().matches(input);
        println!("matched against {:?}: {:?}", self.anything.borrow().as_ref().unwrap(), matches);
        for index in matches {
            if index == self.patterns.len() {
                let m = self.command_matcher.borrow().as_ref().unwrap().captures(input).unwrap();
                let cmd = m.get(1).unwrap().as_str();
                let _rest = m.get(2).unwrap().as_str();
                let index = *self.commands.get(cmd).unwrap();
                println!("Index {} matched! command={} (UNIMPLEMENTED!)", index, cmd);
                let listener = &self.patterns[index];
                if listener.requires_direct && !is_direct {
                    continue;
                }
                // TODO: somehow parse `rest` into a caps-like thing for AMatch and return it.
                let amatch = AMatch {
                    index,
                    rpattern: listener.clone(),
                    token: listener.token,
                    captures: Parameters::Vals(HashMap::new()),
                };
                return Ok(Some(amatch));
            } else {
                println!("Index {} matched! command={}", index, &self.command[index]);
                let listener = &self.patterns[index];
                if listener.requires_direct && !is_direct {
                    continue;
                }
                if let Some(caps) = listener.regex.captures(input) {
                    let amatch = AMatch {
                        index,
                        rpattern: listener.clone(),
                        token: listener.token,
                        captures: Parameters::Caps(caps),
                    };
                    return Ok(Some(amatch));
                }
            }
        }
        println!("no captures for any indexes found");
        Ok(None)
    }

    //fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
    fn add_full_pattern(&mut self, token: u32, command: &str, pattern: &str, callback: Py<PyAny>, defaults: Option<HashMap<String, String>>, direct_required: bool) -> PyResult<()> {
        self.callbacks.push(callback);
        self.add_pattern(token, command, pattern, defaults, direct_required).map_err(to_pyerr)?;
        self.invalidate();
        Ok(())
    }

    //pub fn add_callback_pattern(&mut self, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
    pub fn add_callback_pattern(&mut self, command: &str, pattern: &str, callback: Py<PyAny>, defaults: Option<HashMap<String, String>>, direct_required: bool) -> PyResult<()> {
        self.add_full_pattern(0, command, pattern, callback, defaults, direct_required)
    }

    pub fn parse(&self, room: crate::WrappedRoom, event_id: &crate::EventId, mut input: &str) -> PyResult<Py<PyAny>> {
        println!("Parsing on thread {:?}", thread::current().id());

        let mut direct = false;
        let namelen = self.name.len();
        if input.starts_with(&self.name) && input[namelen..].starts_with(": ") {
            direct = true;
            input = &input[(namelen + 2)..];
        }

        let amatch = self.match_input(input, direct).map_err(to_pyerr)?;
        // println!("Parsed: {:?}", amatch);
        if let Some(amatch) = amatch {
            let callback = &self.callbacks[amatch.index];
            let mut pycap = PyCaptures::new(&amatch.rpattern, &amatch.captures);
            pycap.named.extend(amatch.rpattern.defaults);
            Python::with_gil(|py| {
                let caps_arg = Py::new(py, pycap)?;
                let pyevent_id = crate::WrappedEventId { event_id: Arc::new(event_id.to_owned()) };
                let args = PyTuple::new_bound(py, &[room.into_py(py), pyevent_id.into_py(py), caps_arg.into_py(py)]);
                println!("invoking callback {:?}", callback.bind(py));
                callback.call_bound(py, args, None)
            })
        } else {
            Python::with_gil(|py| { Ok(py.None()) })
        }
    }
}

#[derive(Clone)]
#[pyclass]
struct PyCaptures {
    positional: Vec<Option<String>>,
    named: HashMap<String, String>,
}

impl PyCaptures {
    fn new(rpattern: &RegisteredPattern, captures: &Parameters) -> Self {
        let mut named = HashMap::new();
        let mut positional = vec![];
        if let Parameters::Caps(c) = captures {
            for name in rpattern.regex.capture_names().flatten() {
                let value = c.name(name).unwrap().as_str().to_string();
                named.insert(name.to_owned(), value);
            }
            positional = c.iter().map(|cap| {
                cap.map(|m| { m.as_str().to_owned()})
            }).collect();
        } else if let Parameters::Vals(vs) = captures {
            named.extend(vs.clone());
        }
        Self { positional, named, }
    }
}

#[pymethods]
impl PyCaptures {
    fn __getattr__(&self, py: Python, name: &str) -> PyResult<Py<PyAny>> {
        if let Some(val) = self.named.get(name) {
            Ok(val.into_py(py))
        } else {
            Err(PyAttributeError::new_err("no such attribute"))
        }
    }

    fn __getitem__(&self, py: Python, index: usize) -> PyResult<Py<PyAny>> {
        if index >= self.positional.len() {
            return Err(PyIndexError::new_err("not that many match groups"))
        }

        if let Some(val) = &self.positional[index] {
            Ok(val.into_py(py))
        } else {
            Ok(py.None())
        }
    }
}

#[pyfunction(signature = (command, pattern, callback, defaults, direct_required = false))]
pub fn register_input_handler(py: Python, command: &str, pattern: String, callback: Py<PyAny>, defaults: Option<HashMap<String, String>>, direct_required: bool) -> PyResult<()> {
    println!("Registering input handler! {:?}={} on thread {:?}", pattern, callback, thread::current().id());
    Python::with_gil(|py| {
        if !callback.to_object(py).into_bound(py).is_callable() {
            return Err(PyTypeError::new_err("Expected a callable object"));
        };
        Ok(())
    })?;

    use crate::py_input_handler;
    let pih: Py<InputHandler> = py_input_handler(py)?;
    pih.borrow_mut(py).add_callback_pattern(command, &pattern, callback, defaults, direct_required)?;
    Ok(())
}

#[test]
pub fn test() {
    // Note that this is within the scope of `state`, so that the callbacks here can store references to it.
    let mut handler = InputHandler::new("testbot");

    let pattern = r"3a|5a";
    handler.add_pattern(1, "whathex", &(r"(?:what is|literal) (?:0x)?((?:ff)*(?P<code>".to_string() + pattern + r"){1,16}(?:ff)*) ?\??$"), None, false).expect("invalid regex");

    // TODO: Add whatever rust callback setup I come up with.

    let _ = handler.add_pattern(0, "test", r"test (\w+) (?P<second>\w+)", None, false);
    if let Some(amatch) = handler.match_input("test stabby fish", true).expect("should match") {
        if let Parameters::Caps(captures) = amatch.captures {
            assert_eq!(captures.get(1).expect("m[1] must exist").as_str(), "stabby");
            assert_eq!(captures.name("second").expect("m.second must exist").as_str(), "fish");
        }
    }

    let result = handler.match_input("what is 0x3a3a3a3a?", true).expect("should match");
    let result = result.unwrap();
    assert_eq!(result.token, 1);
    if let Parameters::Caps(captures) = result.captures {
        assert_eq!(captures.name("code").expect("?P<code> should be found").as_str(), "3a");
        assert_eq!(captures.get(1).expect("m[1] should be found").as_str(), "3a3a3a3a");
        assert_eq!(captures.get(2).expect("m[2] should be found").as_str(), "3a");
    }
}
