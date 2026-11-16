//! Runs the learner's code locally and grades it against test cases.
//!
//! Safety rule: only the learner's own code is executed. Test cases from the AI are plain data
//! (arguments, expected values, stdin, expected stdout, SQL rows); the harnesses below are fixed code
//! written here, never generated. SQL runs inside an in-memory SQLite database, not a shell.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

const RUN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT: usize = 20_000;
const RESULTS_MARK: &str = "__CPREP_RESULTS__";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Javascript,
    Sql,
    Java,
}

impl Language {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_lowercase().as_str() {
            "python" | "py" => Language::Python,
            "javascript" | "js" | "node" => Language::Javascript,
            "sql" | "sqlite" | "postgres" | "postgresql" => Language::Sql,
            "java" => Language::Java,
            other => bail!("CPrep can't run {other} yet"),
        })
    }
}

/// How an exercise is checked.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Call a named function with arguments; compare the return value.
    Function,
    /// Run the program with stdin; compare stdout.
    Output,
    /// Run a query against a setup schema; compare result rows.
    Query,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestCase {
    #[serde(default)]
    pub args: Vec<Value>,
    #[serde(default)]
    pub expected: Value,
    #[serde(default)]
    pub stdin: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub millis: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub passed: bool,
    pub hidden: bool,
    /// Shown for visible tests only.
    pub input: Option<String>,
    pub expected: Option<String>,
    pub got: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Grade {
    pub passed: usize,
    pub total: usize,
    pub results: Vec<TestResult>,
    /// Program-level failure (syntax error, crash before tests, timeout).
    pub error: Option<String>,
    pub stdout: String,
}

fn clip(s: &str) -> String {
    if s.len() <= MAX_OUTPUT {
        s.to_string()
    } else {
        let mut end = MAX_OUTPUT;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n… output truncated", &s[..end])
    }
}

fn workdir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("cprep-run").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn python_cmd() -> (String, Vec<String>) {
    if cfg!(windows) && which("py") {
        ("py".into(), vec!["-3".into()])
    } else if which("python") {
        ("python".into(), vec![])
    } else {
        ("python3".into(), vec![])
    }
}

fn which(bin: &str) -> bool {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or(".EXE;.CMD;.BAT".into()).split(';').map(|e| e.to_lowercase()).collect()
    } else {
        vec![String::new()]
    };
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|dir| exts.iter().any(|e| dir.join(format!("{bin}{e}")).is_file()))
        })
        .unwrap_or(false)
}

/// Run a program with a timeout. The child is killed if it runs too long.
async fn exec(program: &str, args: &[String], cwd: &Path, stdin: &str) -> Result<RunOutput> {
    let start = Instant::now();
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // Don't flash a console window from the desktop app.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().with_context(|| format!("couldn't start {program}. Is it installed and on PATH?"))?;
    if let Some(mut si) = child.stdin.take() {
        let input = stdin.to_string();
        tokio::spawn(async move {
            let _ = si.write_all(input.as_bytes()).await;
        });
    }
    match tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output()).await {
        Ok(out) => {
            let out = out?;
            Ok(RunOutput {
                stdout: clip(&String::from_utf8_lossy(&out.stdout)),
                stderr: clip(&String::from_utf8_lossy(&out.stderr)),
                exit_code: out.status.code(),
                timed_out: false,
                millis: start.elapsed().as_millis(),
            })
        }
        Err(_) => Ok(RunOutput {
            stderr: format!("Stopped after {} seconds. Is there an infinite loop?", RUN_TIMEOUT.as_secs()),
            timed_out: true,
            millis: start.elapsed().as_millis(),
            ..Default::default()
        }),
    }
}

fn java_class(code: &str) -> String {
    regex::Regex::new(r"public\s+(?:final\s+)?class\s+(\w+)")
        .unwrap()
        .captures(code)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| "Main".into())
}

/// Plain run: execute the learner's program and show what it prints.
pub async fn run(lang: &Language, code: &str, stdin: &str, setup_sql: &str) -> Result<RunOutput> {
    if *lang == Language::Sql {
        let (setup, code) = (setup_sql.to_string(), code.to_string());
        return tokio::task::spawn_blocking(move || sql_run(&setup, &code)).await?;
    }
    let dir = workdir()?;
    let out = match lang {
        Language::Python => {
            std::fs::write(dir.join("main.py"), code)?;
            let (p, mut a) = python_cmd();
            a.push("main.py".into());
            exec(&p, &a, &dir, stdin).await
        }
        Language::Javascript => {
            std::fs::write(dir.join("main.js"), code)?;
            exec("node", &["main.js".into()], &dir, stdin).await
        }
        Language::Java => {
            let file = format!("{}.java", java_class(code));
            std::fs::write(dir.join(&file), code)?;
            exec("java", &[file], &dir, stdin).await
        }
        Language::Sql => unreachable!(),
    };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

const PY_HARNESS: &str = r#"
import contextlib, io, json, math, sys, traceback
fn_name = sys.argv[1]
tests = json.load(open("tests.json", encoding="utf-8"))
src = open("solution.py", encoding="utf-8").read()
ns = {"__name__": "solution"}
def norm(v):
    return json.loads(json.dumps(v, default=lambda o: list(o) if isinstance(o, (set, tuple)) else repr(o)))
def same(a, b):
    if isinstance(a, float) or isinstance(b, float):
        try: return math.isclose(float(a), float(b), rel_tol=1e-6, abs_tol=1e-9)
        except Exception: return False
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    return a == b
results = []
try:
    with contextlib.redirect_stdout(io.StringIO()):
        exec(compile(src, "solution.py", "exec"), ns)
except Exception:
    print("__CPREP_RESULTS__" + json.dumps({"error": traceback.format_exc(limit=2)}))
    sys.exit(0)
fn = ns.get(fn_name)
if not callable(fn):
    print("__CPREP_RESULTS__" + json.dumps({"error": f"Define a function named {fn_name}."}))
    sys.exit(0)
for t in tests:
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            got = norm(fn(*t.get("args", [])))
        results.append({"ok": same(got, t.get("expected")), "got": got})
    except Exception as e:
        results.append({"ok": False, "error": f"{type(e).__name__}: {e}"})
print("__CPREP_RESULTS__" + json.dumps({"results": results}))
"#;

const JS_HARNESS: &str = r#"
const fs = require("fs"), vm = require("vm");
const fnName = process.argv[2];
const tests = JSON.parse(fs.readFileSync("tests.json", "utf8"));
const src = fs.readFileSync("solution.js", "utf8");
const sandbox = { console: { log() {}, error() {}, warn() {} }, module: { exports: {} } };
sandbox.exports = sandbox.module.exports;
const ctx = vm.createContext(sandbox);
const same = (a, b) => {
  if (typeof a === "number" && typeof b === "number") return Math.abs(a - b) <= 1e-9 + 1e-6 * Math.abs(b);
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => same(x, b[i]));
  if (a && b && typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a).sort(), kb = Object.keys(b).sort();
    return JSON.stringify(ka) === JSON.stringify(kb) && ka.every((k) => same(a[k], b[k]));
  }
  return a === b;
};
let fn;
try {
  vm.runInContext(src, ctx, { filename: "solution.js", timeout: 5000 });
  fn = vm.runInContext(`typeof ${fnName} !== "undefined" ? ${fnName} : (module.exports.${fnName} || module.exports)`, ctx);
} catch (e) {
  console.log("__CPREP_RESULTS__" + JSON.stringify({ error: String(e && e.stack || e).split("\n").slice(0, 3).join("\n") }));
  process.exit(0);
}
if (typeof fn !== "function") {
  console.log("__CPREP_RESULTS__" + JSON.stringify({ error: `Define a function named ${fnName}.` }));
  process.exit(0);
}
const results = tests.map((t) => {
  try {
    const got = JSON.parse(JSON.stringify(fn(...(t.args || [])) ?? null));
    return { ok: same(got, t.expected), got };
  } catch (e) {
    return { ok: false, error: String(e && e.message || e) };
  }
});
console.log("__CPREP_RESULTS__" + JSON.stringify({ results }));
"#;

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

fn show_args(args: &[Value]) -> String {
    args.iter().map(show).collect::<Vec<_>>().join(", ")
}

fn norm_out(s: &str) -> String {
    s.replace("\r\n", "\n").lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Grade the learner's code against an exercise's test cases.
pub async fn grade(
    lang: &Language,
    kind: &Kind,
    code: &str,
    function_name: &str,
    setup_sql: &str,
    ordered: bool,
    tests: &[TestCase],
) -> Result<Grade> {
    match kind {
        Kind::Query => {
            let (setup, code, tests) = (setup_sql.to_string(), code.to_string(), tests.to_vec());
            tokio::task::spawn_blocking(move || sql_grade(&setup, &code, ordered, &tests)).await?
        }
        Kind::Output => {
            let mut g = Grade { total: tests.len(), ..Default::default() };
            for t in tests {
                let out = run(lang, code, &t.stdin, setup_sql).await?;
                if out.timed_out {
                    g.error = Some(out.stderr.clone());
                    g.results.push(fail(t, Some("timed out".into())));
                    continue;
                }
                let got = norm_out(&out.stdout);
                let ok = out.exit_code == Some(0) && got == norm_out(&t.expected_output);
                if ok {
                    g.passed += 1;
                }
                if g.stdout.is_empty() {
                    g.stdout = out.stdout.clone();
                }
                let err = (!out.stderr.trim().is_empty()).then(|| out.stderr.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
                g.results.push(TestResult {
                    passed: ok,
                    hidden: t.hidden,
                    input: (!t.hidden).then(|| if t.stdin.is_empty() { "(no input)".into() } else { t.stdin.clone() }),
                    expected: (!t.hidden).then(|| t.expected_output.clone()),
                    got: (!t.hidden).then(|| got.clone()),
                    error: if ok { None } else { err },
                });
            }
            Ok(g)
        }
        Kind::Function => {
            let dir = workdir()?;
            std::fs::write(dir.join("tests.json"), serde_json::to_string(tests)?)?;
            let out = match lang {
                Language::Python => {
                    std::fs::write(dir.join("solution.py"), code)?;
                    std::fs::write(dir.join("harness.py"), PY_HARNESS)?;
                    let (p, mut a) = python_cmd();
                    a.extend(["harness.py".into(), function_name.into()]);
                    exec(&p, &a, &dir, "").await?
                }
                Language::Javascript => {
                    std::fs::write(dir.join("solution.js"), code)?;
                    std::fs::write(dir.join("harness.js"), JS_HARNESS)?;
                    exec("node", &["harness.js".into(), function_name.into()], &dir, "").await?
                }
                _ => bail!("Function tests are only supported for Python and JavaScript"),
            };
            let _ = std::fs::remove_dir_all(&dir);
            Ok(parse_harness(&out, tests))
        }
    }
}

fn fail(t: &TestCase, error: Option<String>) -> TestResult {
    TestResult { passed: false, hidden: t.hidden, input: None, expected: None, got: None, error }
}

fn parse_harness(out: &RunOutput, tests: &[TestCase]) -> Grade {
    let mut g = Grade { total: tests.len(), ..Default::default() };
    if out.timed_out {
        g.error = Some(out.stderr.clone());
        g.results = tests.iter().map(|t| fail(t, Some("timed out".into()))).collect();
        return g;
    }
    let line = out.stdout.lines().rev().find_map(|l| l.strip_prefix(RESULTS_MARK));
    let Some(v) = line.and_then(|l| serde_json::from_str::<Value>(l).ok()) else {
        g.error = Some(if out.stderr.trim().is_empty() { "The test harness produced no results.".into() } else { out.stderr.clone() });
        return g;
    };
    if let Some(e) = v["error"].as_str() {
        g.error = Some(e.to_string());
        g.results = tests.iter().map(|t| fail(t, None)).collect();
        return g;
    }
    let rs = v["results"].as_array().cloned().unwrap_or_default();
    for (t, r) in tests.iter().zip(rs.iter()) {
        let ok = r["ok"].as_bool().unwrap_or(false);
        if ok {
            g.passed += 1;
        }
        g.results.push(TestResult {
            passed: ok,
            hidden: t.hidden,
            input: (!t.hidden).then(|| show_args(&t.args)),
            expected: (!t.hidden).then(|| show(&t.expected)),
            got: (!t.hidden).then(|| r.get("got").map(show).unwrap_or_else(|| "—".into())),
            error: r["error"].as_str().map(String::from),
        });
    }
    g
}

// ---------- SQL (in-memory SQLite) ----------

fn sql_conn(setup: &str) -> Result<rusqlite::Connection> {
    let mut conn = rusqlite::Connection::open_in_memory()?;
    let start = Instant::now();
    conn.progress_handler(10_000, Some(move || start.elapsed() > Duration::from_secs(5)))?;
    conn.execute_batch(setup).map_err(|e| anyhow!("The exercise's setup SQL failed: {e}"))?;
    Ok(conn)
}

fn cell(v: rusqlite::types::ValueRef) -> Value {
    use rusqlite::types::ValueRef::*;
    match v {
        Null => Value::Null,
        Integer(i) => json!(i),
        Real(f) => json!(f),
        Text(t) => json!(String::from_utf8_lossy(t)),
        Blob(b) => json!(format!("<{} bytes>", b.len())),
    }
}

/// Run the learner's statements; return the rows of the last one that produces rows.
fn sql_rows(conn: &rusqlite::Connection, code: &str) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let mut last = (vec![], vec![]);
    let mut rest = code.trim();
    while !rest.is_empty() {
        // rusqlite prepares one statement at a time, so split them ourselves.
        let consumed = sql_statement_len(rest);
        let text = rest[..consumed].trim();
        let (stmt_opt, tail) = {
            if text.trim_end_matches(';').trim().is_empty() {
                (None, &rest[consumed..])
            } else {
                let mut stmt = conn.prepare(text).map_err(|e| anyhow!("{e}"))?;
                let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
                let n = cols.len();
                let mut rows = Vec::new();
                let mut q = stmt.raw_query();
                while let Some(r) = q.next()? {
                    rows.push((0..n).map(|i| cell(r.get_ref(i).unwrap())).collect());
                }
                ((n > 0).then_some((cols, rows)), &rest[consumed..])
            }
        };
        if let Some(x) = stmt_opt {
            last = x;
        }
        rest = tail.trim_start_matches(|c: char| c == ';' || c.is_whitespace());
    }
    Ok(last)
}

/// Length of the first SQL statement (up to and including its `;`), ignoring semicolons in quotes.
fn sql_statement_len(s: &str) -> usize {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ';') => return i + 1,
            _ => {}
        }
    }
    s.len()
}

fn sql_run(setup: &str, code: &str) -> Result<RunOutput> {
    let start = Instant::now();
    let conn = sql_conn(setup)?;
    Ok(match sql_rows(&conn, code) {
        Ok((cols, rows)) => RunOutput {
            stdout: format_table(&cols, &rows),
            exit_code: Some(0),
            millis: start.elapsed().as_millis(),
            ..Default::default()
        },
        Err(e) => RunOutput { stderr: e.to_string(), exit_code: Some(1), millis: start.elapsed().as_millis(), ..Default::default() },
    })
}

fn format_table(cols: &[String], rows: &[Vec<Value>]) -> String {
    if cols.is_empty() {
        return "(statement ran, no rows returned)".into();
    }
    let cellstr = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => "NULL".into(),
        o => o.to_string(),
    };
    let mut w: Vec<usize> = cols.iter().map(|c| c.len()).collect();
    for r in rows {
        for (i, v) in r.iter().enumerate() {
            w[i] = w[i].max(cellstr(v).len());
        }
    }
    let line = |vals: Vec<String>| vals.iter().enumerate().map(|(i, v)| format!("{v:<width$}", width = w[i])).collect::<Vec<_>>().join(" | ");
    let mut out = vec![line(cols.to_vec()), w.iter().map(|n| "-".repeat(*n)).collect::<Vec<_>>().join("-+-")];
    out.extend(rows.iter().take(200).map(|r| line(r.iter().map(cellstr).collect())));
    out.push(format!("({} row{})", rows.len(), if rows.len() == 1 { "" } else { "s" }));
    out.join("\n")
}

fn sql_grade(setup: &str, code: &str, ordered: bool, tests: &[TestCase]) -> Result<Grade> {
    let mut g = Grade { total: tests.len().max(1), ..Default::default() };
    let conn = sql_conn(setup)?;
    let (cols, rows) = match sql_rows(&conn, code) {
        Ok(x) => x,
        Err(e) => {
            g.error = Some(e.to_string());
            g.results = tests.iter().map(|t| fail(t, None)).collect();
            return Ok(g);
        }
    };
    g.stdout = format_table(&cols, &rows);
    let mut got: Vec<Value> = rows.into_iter().map(Value::Array).collect();
    for t in tests {
        let mut want: Vec<Value> = t.expected.as_array().cloned().unwrap_or_default();
        if !ordered {
            want.sort_by_key(|v| v.to_string());
            got.sort_by_key(|v| v.to_string());
        }
        let ok = rows_equal(&got, &want);
        if ok {
            g.passed += 1;
        }
        g.results.push(TestResult {
            passed: ok,
            hidden: t.hidden,
            input: (!t.hidden).then(|| "your query".into()),
            expected: (!t.hidden).then(|| format!("{} row(s): {}", want.len(), Value::Array(want.iter().take(5).cloned().collect()))),
            got: (!t.hidden).then(|| format!("{} row(s): {}", got.len(), Value::Array(got.iter().take(5).cloned().collect()))),
            error: None,
        });
    }
    Ok(g)
}

fn rows_equal(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Value::Array(xs), Value::Array(ys)) => xs.len() == ys.len() && xs.iter().zip(ys).all(|(p, q)| cells_equal(p, q)),
            _ => cells_equal(x, y),
        })
}

fn cells_equal(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-6,
        _ => match (a, b) {
            (Value::String(x), Value::String(y)) => x == y,
            (Value::String(x), y) | (y, Value::String(x)) => *x == y.to_string(),
            _ => a == b,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(args: Value, expected: Value, hidden: bool) -> TestCase {
        TestCase {
            args: args.as_array().cloned().unwrap(),
            expected,
            stdin: String::new(),
            expected_output: String::new(),
            hidden,
        }
    }

    #[tokio::test]
    async fn python_function_grading() {
        let tests = vec![t(json!([2, 3]), json!(5), false), t(json!([-1, 1]), json!(0), true), t(json!([0.1, 0.2]), json!(0.3), false)];
        let g = grade(&Language::Python, &Kind::Function, "def add(a, b):\n    print('noise')\n    return a + b\n", "add", "", true, &tests)
            .await
            .unwrap();
        assert_eq!((g.passed, g.total), (3, 3), "{g:?}");
        assert!(g.results[1].input.is_none(), "hidden test details stay hidden");

        let g = grade(&Language::Python, &Kind::Function, "def add(a, b):\n    return a - b\n", "add", "", true, &tests).await.unwrap();
        assert_eq!(g.passed, 0);
        assert_eq!(g.results[0].got.as_deref(), Some("-1"));

        let g = grade(&Language::Python, &Kind::Function, "def add(a, b)\n    return a\n", "add", "", true, &tests).await.unwrap();
        assert!(g.error.unwrap().contains("SyntaxError"));
    }

    #[tokio::test]
    async fn javascript_function_grading() {
        let tests = vec![t(json!([[3, 1, 2]]), json!([1, 2, 3]), false)];
        let g = grade(&Language::Javascript, &Kind::Function, "function sortNums(a) { return [...a].sort((x, y) => x - y); }", "sortNums", "", true, &tests)
            .await
            .unwrap();
        assert_eq!(g.passed, 1, "{g:?}");
        let g = grade(&Language::Javascript, &Kind::Function, "const sortNums = (a) => a;", "sortNums", "", true, &tests).await.unwrap();
        assert_eq!(g.passed, 0);
    }

    #[tokio::test]
    async fn output_grading_and_timeout() {
        let tests = vec![TestCase { stdin: "4\n".into(), expected_output: "16".into(), ..t(json!([]), Value::Null, false) }];
        let g = grade(&Language::Python, &Kind::Output, "n = int(input())\nprint(n * n)\n", "", "", true, &tests).await.unwrap();
        assert_eq!(g.passed, 1, "{g:?}");
        let out = run(&Language::Python, "while True:\n    pass\n", "", "").await.unwrap();
        assert!(out.timed_out);
    }

    #[tokio::test]
    async fn sql_grading() {
        let setup = "CREATE TABLE users (id INTEGER, name TEXT, age INTEGER);
                     INSERT INTO users VALUES (1, 'Ama', 21), (2, 'Kofi', 17), (3, 'Esi', 30);";
        let tests = vec![TestCase { expected: json!([["Ama"], ["Esi"]]), ..t(json!([]), Value::Null, false) }];
        let g = grade(&Language::Sql, &Kind::Query, "SELECT name FROM users WHERE age >= 18;", "", setup, false, &tests).await.unwrap();
        assert_eq!(g.passed, 1, "{g:?}");
        let g = grade(&Language::Sql, &Kind::Query, "SELECT name FROM users;", "", setup, false, &tests).await.unwrap();
        assert_eq!(g.passed, 0);
        let g = grade(&Language::Sql, &Kind::Query, "SELEC name FROM users;", "", setup, false, &tests).await.unwrap();
        assert!(g.error.is_some());
        let out = run(&Language::Sql, "SELECT COUNT(*) AS n FROM users; SELECT name FROM users WHERE id = 2;", "", setup).await.unwrap();
        assert!(out.stdout.contains("Kofi"), "{}", out.stdout);
    }

    #[tokio::test]
    async fn java_runs() {
        if !which("java") {
            return;
        }
        let out = run(&Language::Java, "public class Hello { public static void main(String[] a) { System.out.println(\"hi \" + (2 + 3)); } }", "", "")
            .await
            .unwrap();
        assert_eq!(out.stdout.trim(), "hi 5", "{out:?}");
    }
}
