import "../monaco";
import { useCallback, useEffect, useRef, useState } from "react";
import Editor, { OnMount } from "@monaco-editor/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, Exercise, Grade, RunOutput, TestCase } from "../api";
import { Icon, MdIcon, remarkEmojiIcons } from "../icons";

type Props = { lessonId: number; topic: string; onClose: () => void };

const mdPlugins = [remarkGfm, remarkEmojiIcons];
const mdComponents = { i: MdIcon as never };
const LANG_LABEL = { python: "Python", javascript: "JavaScript", sql: "SQL", java: "Java" } as const;

function fmt(v: unknown) {
  return JSON.stringify(v);
}

function exampleLine(ex: Exercise, t: TestCase) {
  if (ex.kind === "function") return `${ex.function_name}(${t.args.map(fmt).join(", ")}) → ${fmt(t.expected)}`;
  if (ex.kind === "output") return `input ${t.stdin.trim() ? fmt(t.stdin) : "(none)"} → prints ${fmt(t.expected_output)}`;
  const rows = Array.isArray(t.expected) ? t.expected.length : 0;
  return `returns ${rows} row${rows === 1 ? "" : "s"}: ${fmt(Array.isArray(t.expected) ? t.expected.slice(0, 4) : t.expected)}`;
}

/** Practice panel: exercise, editor, run, submit, test results and the tutor's review. */
export default function Lab({ lessonId, topic, onClose }: Props) {
  const [exercises, setExercises] = useState<Exercise[] | null>(null);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [code, setCode] = useState("");
  const [stdin, setStdin] = useState("");
  const [busy, setBusy] = useState<"run" | "submit" | "new" | null>(null);
  const [output, setOutput] = useState<RunOutput | null>(null);
  const [grade, setGrade] = useState<Grade | null>(null);
  const [review, setReview] = useState("");
  const [reviewMeta, setReviewMeta] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const runRef = useRef<() => void>(() => {});
  const submitRef = useRef<() => void>(() => {});
  const saveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const active = exercises?.find((e) => e.id === activeId) ?? null;

  const select = useCallback((ex: Exercise) => {
    setActiveId(ex.id);
    setCode(ex.draft ?? ex.starter_code);
    setOutput(null);
    setGrade(null);
    setReview("");
    setReviewMeta(null);
    setError(null);
  }, []);

  useEffect(() => {
    api.listExercises(lessonId).then((list) => {
      setExercises(list);
      if (list.length) select(list[list.length - 1]);
    });
  }, [lessonId, select]);

  const refreshList = async () => {
    const list = await api.listExercises(lessonId);
    setExercises(list);
  };

  const generate = async () => {
    setBusy("new");
    setError(null);
    try {
      const ex = await api.newExercise(lessonId);
      await refreshList();
      select(ex);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onEdit = (value: string | undefined) => {
    const v = value ?? "";
    setCode(v);
    if (!active) return;
    clearTimeout(saveTimer.current);
    const id = active.id;
    saveTimer.current = setTimeout(() => api.saveDraft(id, v).catch(() => {}), 800);
  };

  const run = async () => {
    if (!active || busy) return;
    setBusy("run");
    setError(null);
    setGrade(null);
    try {
      setOutput(await api.runCode(active.id, code, stdin));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const submit = async () => {
    if (!active || busy) return;
    setBusy("submit");
    setError(null);
    setOutput(null);
    setGrade(null);
    setReview("");
    setReviewMeta(null);
    try {
      await api.submitCode(active.id, code, (e) => {
        if (e.type === "graded") setGrade(e.grade);
        if (e.type === "delta") setReview((r) => r + e.text);
        if (e.type === "done") setReviewMeta(e.provider === "CPrep" ? null : `Reviewed by ${e.provider}`);
        if (e.type === "error") setError(e.message);
      });
      await refreshList();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  runRef.current = run;
  submitRef.current = submit;

  const onMount: OnMount = (editor, monaco) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => runRef.current());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter, () => submitRef.current());
  };

  return (
    <aside className="lab" aria-label="Practice">
      <header className="lab-head">
        <div className="lab-title">
          <Icon name="code" size={16} />
          Practice
        </div>
        <div className="lab-tabs" role="tablist" aria-label="Exercises">
          {exercises?.map((ex, i) => (
            <button
              key={ex.id}
              role="tab"
              aria-selected={ex.id === activeId}
              className="lab-tab"
              title={ex.title}
              onClick={() => select(ex)}
            >
              {i + 1}
              {ex.best && ex.best[0] === ex.best[1] && <Icon name="check" size={12} />}
            </button>
          ))}
        </div>
        <button className="btn btn-quiet" disabled={busy !== null} onClick={generate}>
          {busy === "new" ? <span className="spinner" /> : null} New exercise
        </button>
        <button className="icon-btn" aria-label="Close practice" title="Close practice" onClick={onClose}>
          <Icon name="close" size={16} />
        </button>
      </header>

      <div className="lab-body">
        {exercises && exercises.length === 0 && !busy && (
          <div className="lab-empty">
            <p className="lab-empty-title">Practise what you just learned</p>
            <p className="muted">
              CPrep writes a short exercise on <b>{topic}</b> with tests. Your code runs on your laptop and the tutor reviews it.
            </p>
            <button className="btn btn-primary" onClick={generate}>Write an exercise</button>
          </div>
        )}
        {busy === "new" && (
          <p className="faint lab-empty" role="status">Writing an exercise on {topic}…</p>
        )}
        {error && <div className="error-note" role="alert">{error}</div>}

        {active && busy !== "new" && (
          <>
            <section className="lab-task">
              <div className="lab-meta">
                <span>{LANG_LABEL[active.language]}</span>
                {active.best && <span>Best: {active.best[0]}/{active.best[1]} tests</span>}
              </div>
              <h2>{active.title}</h2>
              <div className="lab-prompt">
                <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{active.prompt}</Markdown>
              </div>
              {active.kind === "query" && active.setup_sql && (
                <details className="lab-tables">
                  <summary>Tables in this exercise</summary>
                  <pre><code>{active.setup_sql}</code></pre>
                </details>
              )}
              <ul className="lab-examples">
                {active.examples.map((t, i) => (
                  <li key={i}><code>{exampleLine(active, t)}</code></li>
                ))}
                {active.hidden_count > 0 && (
                  <li className="faint">+ {active.hidden_count} hidden check{active.hidden_count === 1 ? "" : "s"}</li>
                )}
              </ul>
            </section>

            <div className="lab-editor">
              <Editor
                key={active.id}
                height="100%"
                language={active.language}
                theme="cprep-ink"
                value={code}
                onChange={onEdit}
                onMount={onMount}
                loading={<span className="spinner" aria-label="Loading editor" />}
                options={{
                  minimap: { enabled: false },
                  fontSize: 13,
                  fontFamily: '"JetBrains Mono Variable", Consolas, monospace',
                  fontLigatures: false,
                  scrollBeyondLastLine: false,
                  tabSize: 4,
                  insertSpaces: true,
                  padding: { top: 12, bottom: 12 },
                  automaticLayout: true,
                  lineNumbersMinChars: 3,
                  renderLineHighlight: "line",
                  scrollbar: { verticalScrollbarSize: 8, horizontalScrollbarSize: 8 },
                  overviewRulerLanes: 0,
                  ariaLabel: `${LANG_LABEL[active.language]} code editor`,
                }}
              />
            </div>

            {active.kind === "output" && (
              <label className="lab-stdin">
                <span className="faint">Input for Run (stdin)</span>
                <textarea rows={2} value={stdin} onChange={(e) => setStdin(e.target.value)} spellCheck={false} />
              </label>
            )}

            <div className="lab-actions">
              <button className="btn" disabled={busy !== null} onClick={run}>
                {busy === "run" ? <span className="spinner" /> : <Icon name="play" size={14} />}
                Run <kbd>Ctrl+Enter</kbd>
              </button>
              <button className="btn btn-primary" disabled={busy !== null} onClick={submit}>
                {busy === "submit" ? <span className="spinner" /> : null}
                Submit <kbd>Ctrl+Shift+Enter</kbd>
              </button>
              <button
                className="btn btn-quiet"
                disabled={busy !== null || code === active.starter_code}
                onClick={async () => {
                  if (await api.confirm("Reset the editor to the starter code? Your current code will be replaced.", "Reset")) onEdit(active.starter_code);
                }}
              >
                Reset
              </button>
            </div>

            {output && <RunPanel out={output} />}
            {grade && <GradePanel grade={grade} />}
            {(review || busy === "submit") && grade && (
              <section className="lab-review" aria-live="polite">
                <div className="prose">
                  <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{review}</Markdown>
                  {busy === "submit" && <span className="caret" aria-hidden="true" />}
                </div>
                {reviewMeta && <div className="turn-meta">{reviewMeta}</div>}
              </section>
            )}
          </>
        )}
      </div>
    </aside>
  );
}

function RunPanel({ out }: { out: RunOutput }) {
  const failed = out.timed_out || (out.exit_code !== null && out.exit_code !== 0);
  return (
    <section className="lab-console" data-failed={failed} aria-live="polite">
      <div className="lab-console-head">
        <span>{out.timed_out ? "Stopped: took too long" : failed ? `Exited with code ${out.exit_code}` : "Output"}</span>
        <span className="faint">{out.millis} ms</span>
      </div>
      {out.stdout && <pre>{out.stdout}</pre>}
      {out.stderr && <pre className="lab-stderr">{out.stderr}</pre>}
      {!out.stdout && !out.stderr && <pre className="faint">(nothing printed)</pre>}
    </section>
  );
}

function GradePanel({ grade }: { grade: Grade }) {
  const all = grade.passed === grade.total && !grade.error;
  let hiddenN = 0;
  return (
    <section className="lab-grade" aria-live="polite">
      <div className="lab-score" data-all={all}>
        <Icon name={all ? "check" : "cross"} size={16} />
        {grade.passed} of {grade.total} tests passed
      </div>
      {grade.error && <pre className="lab-stderr">{grade.error}</pre>}
      <ol className="lab-results">
        {grade.results.map((r, i) => {
          if (r.hidden) hiddenN++;
          return (
            <li key={i} data-passed={r.passed}>
              <Icon name={r.passed ? "check" : "cross"} size={13} />
              {r.hidden ? (
                <span>Hidden check {hiddenN}</span>
              ) : (
                <span className="lab-result-detail">
                  <code>{r.input}</code>
                  {!r.passed && (
                    <>
                      {" "}expected <code>{r.expected}</code>, got <code>{r.got ?? "—"}</code>
                    </>
                  )}
                </span>
              )}
              {r.error && <span className="lab-result-error">{r.error}</span>}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
