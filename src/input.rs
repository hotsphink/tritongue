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
    token: u32,
    defaults: HashMap<String, String>,
}

#[derive(Debug)]
pub struct AMatch<'a> {
    index: usize,
    rpattern: RegisteredPattern,
    pub token: u32,
    pub captures: Captures<'a>,
}

#[derive(Debug)]
pub struct HandlerRegistry {
    // RegexSet that checks whether any of the regexes match. Invalidated when new patterns are registered.
    anything: RefCell<Option<RegexSet>>,
    patterns: Vec<RegisteredPattern>,
}

#[pyclass]
pub struct InputHandler {
    pub registry: HandlerRegistry,
    callbacks: Vec<Py<PyAny>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            anything: RefCell::new(None),
            patterns: vec![],
        }
    }

    pub fn add_pattern(&mut self, token: u32, pattern: &str, defaults: Option<HashMap<String, String>>) -> anyhow::Result<()> {
        let r = Regex::new(pattern)?;
        self.patterns.push(RegisteredPattern {
            pattern: pattern.to_string(),
            regex: r,
            token,
            defaults: defaults.unwrap_or_default(),
        });
        *self.anything.borrow_mut() = None; // Invalidate the "does anything match" RegexSet.
        println!("added pattern {}, patterns now = {:?}", pattern, self.patterns);
        Ok(())
    }

    fn ensure(&self) -> anyhow::Result<()> {
        let mut a = self.anything.borrow_mut();
        println!("ensure called with ms={:?} while patterns = {:?}", a, self.patterns);
        if a.is_none() {
            let anything = RegexSet::new(self.patterns.iter().map(|l| &l.pattern))?;
            *a = Some(anything);
        }
        Ok(())
    }

    pub fn parse<'b>(&self, input: &'b str) -> anyhow::Result<Option<AMatch<'b>>> {
        self.ensure()?;
        let matches = self.anything.borrow().as_ref().unwrap().matches(input);
        println!("matched against {:?}: {:?}", self.anything.borrow().as_ref().unwrap(), matches);
        for index in matches {
            println!("Index {} matched!", index);
            let listener = &self.patterns[index];
            if let Some(caps) = listener.regex.captures(input) {
                let amatch = AMatch {
                    index,
                    rpattern: listener.clone(),
                    token: listener.token,
                    captures: caps,
                };
                return Ok(Some(amatch));
            }
        }
        println!("no captures for any indexes found");
        Ok(None)
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[pymethods]
impl InputHandler {
    #[new]
    pub fn new() -> Self {
        Self {
            registry: HandlerRegistry::new(),
            callbacks: vec![],
        }
    }
}

impl Default for InputHandler {
    fn default() -> Self {
        Self::new()
    }
}

pub fn to_pyerr(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

impl InputHandler {
    //fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
    fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: Py<PyAny>, defaults: Option<HashMap<String, String>>) -> PyResult<()> {
        self.callbacks.push(callback);
        self.registry.add_pattern(token, pattern, defaults).map_err(to_pyerr)?;
        *self.registry.anything.borrow_mut() = None; // Invalidate the "does anything match" RegexSet.
        Ok(())
    }

    //pub fn add_callback_pattern(&mut self, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
        pub fn add_callback_pattern(&mut self, pattern: &str, callback: Py<PyAny>, defaults: Option<HashMap<String, String>>) -> PyResult<()> {
        self.add_full_pattern(0, pattern, callback, defaults)
    }

    pub fn parse(&self, room: crate::WrappedRoom, event_id: &crate::EventId, input: &str) -> PyResult<Py<PyAny>> {
        println!("Parsing on thread {:?}", thread::current().id());
        let amatch = self.registry.parse(input).map_err(to_pyerr)?;
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
    fn new(rpattern: &RegisteredPattern, captures: &Captures) -> Self {
        let mut named = HashMap::new();
        for name in rpattern.regex.capture_names().flatten() {
            let value = captures.name(name).unwrap().as_str().to_string();
            named.insert(name.to_owned(), value);
        }
        let positional = captures.iter().map(|cap| {
            cap.map(|m| { m.as_str().to_owned()})
        }).collect();
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

#[pyfunction]
fn register_input_handler(py: Python, pattern: String, callback: Py<PyAny>, defaults: Option<HashMap<String, String>>) -> PyResult<()> {
    println!("Registering input handler! {:?}={} on thread {:?}", pattern, callback, thread::current().id());
    Python::with_gil(|py| {
        if !callback.to_object(py).into_bound(py).is_callable() {
            return Err(PyTypeError::new_err("Expected a callable object"));
        };
        Ok(())
    })?;

    use crate::py_input_handler;
    let pih: Py<InputHandler> = py_input_handler(py)?;
    let result = pih.borrow_mut(py).add_callback_pattern(&pattern, callback, defaults);
    result
}

#[pyfunction]
fn make_text_response(py: Python, text: String) -> PyResult<Py<PyAny>> {
    use crate::AnyEventPy;
    let ev = AnyEventPy::RoomTextMessage { text };
    Ok(Py::new(py, ev)?.as_any().to_owned())
}

#[pymodule]
pub fn trinity(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    use crate::{send_text, send_html, react};
    println!("running trinity!!!");
    m.add_function(wrap_pyfunction!(register_input_handler, m)?)?;
    m.add_function(wrap_pyfunction!(make_text_response, m)?)?;
    m.add_function(wrap_pyfunction!(send_text, m)?)?;
    m.add_function(wrap_pyfunction!(send_html, m)?)?;
    m.add_function(wrap_pyfunction!(react, m)?)?;
    Ok(())
}

#[test]
pub fn test() {
    // Note that this is within the scope of `state`, so that the callbacks here can store references to it.
    let mut handler = InputHandler::new();
    let mut registry = HandlerRegistry::new();

    let pattern = r"3a|5a";
    registry.add_pattern(1, &(r"(?:what is|literal) (?:0x)?((?:ff)*(?P<code>".to_string() + pattern + r"){1,16}(?:ff)*) ?\??$"), None).expect("invalid regex");

    // TODO: Add whatever rust callback setup I come up with.

    let _ = handler.registry.add_pattern(0, r"test (\w+) (?P<second>\w+)", None);
    if let Some(amatch) = handler.registry.parse("test stabby fish").expect("should match") {
        assert_eq!(amatch.captures.get(1).expect("m[1] must exist").as_str(), "stabby");
        assert_eq!(amatch.captures.name("second").expect("m.second must exist").as_str(), "fish");
    }

    let result = registry.parse("what is 0x3a3a3a3a?").expect("should match");
    let result = result.unwrap();
    assert_eq!(result.token, 1);
    assert_eq!(result.captures.name("code").expect("?P<code> should be found").as_str(), "3a");
    assert_eq!(result.captures.get(1).expect("m[1] should be found").as_str(), "3a3a3a3a");
    assert_eq!(result.captures.get(2).expect("m[2] should be found").as_str(), "3a");
}
