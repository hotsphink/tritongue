use anyhow;
use regex::{Captures, Regex, RegexSet};
use pyo3::prelude::*;
use pyo3::{
    exceptions::{PyAttributeError, PyRuntimeError, PyIndexError, PyTypeError},
    types::PyTuple
};
//use pyo3_asyncio_0_21::tokio::future_into_py;
use std::{
    cell::RefCell,
    collections::HashMap
};

#[derive(Debug, Clone)]
pub struct RegisteredPattern {
    regex: Regex,
    pattern: String,
    token: u32,
}

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

pub struct ThreadInputHandler<'a> {
    pub registry: HandlerRegistry,
    callbacks: Vec<Box<dyn Fn(RegisteredPattern, &str, &Captures) -> () + 'a>>,
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

    pub fn add_pattern(&mut self, token: u32, pattern: &str) -> anyhow::Result<()> {
        let r = Regex::new(pattern)?;
        self.patterns.push(RegisteredPattern {
            pattern: pattern.to_string(),
            regex: r,
            token: token,
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
        println!("matched against {:?}", self.anything.borrow().as_ref().unwrap());
        for index in matches {
            println!("Index {} matched!", index);
            let listener = &self.patterns[index];
            if let Some(caps) = listener.regex.captures(input) {
                let amatch = AMatch {
                    index: index,
                    rpattern: listener.clone(),
                    token: listener.token,
                    captures: caps,
                };
                return Ok(Some(amatch));
            }
        }
        Ok(None)
    }
}

impl<'a> ThreadInputHandler<'a> {
    pub fn new() -> Self {
        Self {
            registry: HandlerRegistry::new(),
            callbacks: vec![],
        }
    }

    fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
        self.callbacks.push(Box::new(callback));
        self.registry.add_pattern(token, pattern)?;
        *self.registry.anything.borrow_mut() = None; // Invalidate the "does anything match" RegexSet.
        Ok(())
    }

    pub fn add_callback_pattern(&mut self, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
        self.add_full_pattern(0, pattern, callback)
    }

    pub fn parse<'b>(&self, input: &'b str) -> anyhow::Result<Option<AMatch<'b>>> {
        let amatch = self.registry.parse(input)?;
        if let Some(amatch) = amatch {
            let callback = &self.callbacks[amatch.index];
            let _ = callback(amatch.rpattern, input, &amatch.captures);
            Ok(None) // FIXME
        } else {
            Ok(None)
        }
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

fn to_pyerr(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

impl InputHandler {
    //fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
    fn add_full_pattern(&mut self, token: u32, pattern: &str, callback: Py<PyAny>) -> PyResult<()> {
        self.callbacks.push(callback);
        self.registry.add_pattern(token, pattern).map_err(to_pyerr)?;
        *self.registry.anything.borrow_mut() = None; // Invalidate the "does anything match" RegexSet.
        Ok(())
    }

    //pub fn add_callback_pattern(&mut self, pattern: &str, callback: impl Fn(RegisteredPattern, &str, &Captures) + 'a) -> anyhow::Result<()> {
    pub fn add_callback_pattern(&mut self, pattern: &str, callback: Py<PyAny>) -> PyResult<()> {
        self.add_full_pattern(0, pattern, callback)
    }

    pub fn parse(&self, input: &str) -> PyResult<Py<PyAny>> {
        let amatch = self.registry.parse(input).map_err(to_pyerr)?;
        if let Some(amatch) = amatch {
            let callback = &self.callbacks[amatch.index];
            let pycap = PyCaptures::new(amatch.rpattern, &amatch.captures);
            Python::with_gil(|py| {
                let caps_arg = Py::new(py, pycap)?;
                let args = PyTuple::new_bound(py, &[caps_arg]);
                callback.call_bound(py, args, None)
            })
        } else {
            Python::with_gil(|py| { Ok(py.None()) })
        }
    }
}

#[pyclass]
struct PyCaptures {
    positional: Vec<Option<String>>,
    named: HashMap<String, String>,
}

impl PyCaptures {
    fn new(rpattern: RegisteredPattern, captures: &Captures) -> Self {
        let mut named = HashMap::new();
        for name in rpattern.regex.capture_names() {
            if let Some(name) = name {
                let value = captures.name(name).unwrap().as_str().to_string();
                named.insert(name.to_owned(), value);
            }
        }
        let positional = captures.iter().map(|cap| {
            cap.map(|m| { m.as_str().to_owned()})
        }).collect();
        Self {
            positional: positional,
            named: named,
        }
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
//fn register_input_handler(py: Python, pattern: String, callback: PyObject) -> PyResult<Bound<PyAny>> {
fn register_input_handler(py: Python, pattern: String, callback: Py<PyAny>) -> PyResult<()> {
    use std::thread;
    println!("Registering input handler! {:?}={} {} on thread {:?}", pattern, callback, callback, thread::current().id());
    Python::with_gil(|py| {
        if !callback.to_object(py).into_bound(py).is_callable() {
            return Err(PyTypeError::new_err("Expected a callable object"));
        };
        Ok(())
    })?;

    let pih: Py<InputHandler> = py.import_bound("sys")?.getattr("app")?.extract()?;
    let result = pih.borrow_mut(py).add_callback_pattern(&pattern, callback);
    result
}

#[pymodule]
pub fn trinity(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    println!("running trinity!!!");
    m.add_function(wrap_pyfunction!(register_input_handler, m)?)?;
    Ok(())
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
    let handler = ThreadInputHandler::new();
    let mut registry = HandlerRegistry::new();

    let pattern = r"3a|5a";
    registry.add_pattern(1, &(r"(?:what is|literal) (?:0x)?((?:ff)*(?P<code>".to_string() + pattern + r"){1,16}(?:ff)*) ?\??$")).expect("invalid regex");

    //handler.add_callback_pattern(r"test (\w+) (?P<second>\w+)", |_rpattern, _input: &str, caps: &Captures| {
    //    state.borrow_mut().first = caps.get(1).unwrap().as_str().to_string();
    //    state.borrow_mut().second = caps.name("second").unwrap().as_str().to_string();
    //}).expect("adding pattern");
    let _ = handler.parse("test stabby fish").expect("should match");
    assert_eq!(state.borrow().first.as_str(), "stabby");
    assert_eq!(state.borrow().second.as_str(), "fish");

    let result = registry.parse("what is 0x3a3a3a3a?").expect("should match");
    let result = result.unwrap();
    assert_eq!(result.token, 1);
    assert_eq!(result.captures.name("code").expect("?P<code> should be found").as_str(), "3a");
    assert_eq!(result.captures.get(1).expect("m[1] should be found").as_str(), "3a3a3a3a");
    assert_eq!(result.captures.get(2).expect("m[2] should be found").as_str(), "3a");
}
