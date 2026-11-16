import { useCallback, useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import Markdown from "react-markdown";
import { MdIcon, remarkEmojiIcons } from "../icons";

const mdPlugins = [remarkEmojiIcons];
const mdComponents = { i: MdIcon as never };
import {
  api,
  IngestProgress,
  Lesson,
  PlacementResult,
  Question,
  TrackDetail,
  TrackSummary,
} from "../api";

const TOPICS: Record<string, string[]> = {
  backend: ["Python", "Flask", "FastAPI", "Django", "HTTP & REST", "SQL", "Git", "Java", "JavaScript", "Docker"],
  "ai-eng": ["Python", "NumPy", "LLM APIs", "Prompting", "Embeddings", "RAG", "Agents"],
  databases: ["SQL basics", "Joins", "Indexes", "Postgres", "ORMs", "Transactions"],
  dsa: ["Big-O", "Arrays", "Hashing", "Linked lists", "Trees", "Graphs", "Dynamic programming"],
  "system-design": ["Caching", "Load balancing", "Databases at scale", "Queues", "APIs"],
  networking: ["OSI model", "IP & subnetting", "TCP/UDP", "DNS", "HTTP", "Sockets"],
  automation: ["Python", "APIs", "Webhooks", "Scraping", "LLM APIs", "n8n / Zapier"],
  interview: ["DSA", "System design", "Behavioural", "Resume"],
  hardware: [],
};

type Props = { track: TrackSummary; onOpenLesson: (id: number) => void; onChanged: () => void };

export default function TrackHome({ track, onOpenLesson, onChanged }: Props) {
  const [detail, setDetail] = useState<TrackDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await api.getTrack(track.id));
    } catch (e) {
      setError(String(e));
    }
  }, [track.id]);

  useEffect(() => {
    setDetail(null);
    load();
  }, [load]);

  if (track.status === "paused") {
    return (
      <div className="page">
        <h1 className="page-title">{track.name}</h1>
        <p className="page-lede">
          This track is paused while you focus on software. Its folders and progress will be here when you turn it back on.
        </p>
      </div>
    );
  }

  const hasMaterial = (detail?.folders.some((f) => f.documents > 0)) ?? false;
  const placementDone = !!detail && (detail.placement !== null || detail.known.length > 0);
  const hasCourse = !!detail?.curriculum;

  return (
    <div className="page">
      <div className="track-head">
        <span className="track-dot" style={{ background: track.accent }} />
        <span className="faint">{track.blurb}</span>
      </div>
      <h1 className="page-title">{track.name}</h1>
      {hasCourse && detail?.curriculum && (
        <p className="page-lede">{detail.curriculum.summary}</p>
      )}
      {error && <div className="error-note" role="alert">{error}</div>}

      {detail && (
        <ol className="steps">
          <Step n={1} state={hasMaterial ? "done" : "current"} title="Your material">
            <Material trackId={track.id} detail={detail} reload={async () => { await load(); onChanged(); }} />
          </Step>
          <Step n={2} state={placementDone ? "done" : hasMaterial ? "current" : "todo"} title="What you already know">
            <Placement trackId={track.id} detail={detail} reload={load} />
          </Step>
          <Step n={3} state={hasCourse ? "done" : placementDone ? "current" : "todo"} title="Your course">
            <Course
              trackId={track.id}
              detail={detail}
              ready={placementDone}
              reload={async () => { await load(); onChanged(); }}
              onOpenLesson={onOpenLesson}
            />
          </Step>
        </ol>
      )}
    </div>
  );
}

function Step(props: { n: number; state: "done" | "current" | "todo"; title: string; children: React.ReactNode }) {
  return (
    <li className="step" data-state={props.state}>
      <div className="step-num" aria-hidden="true">{props.n}</div>
      <div className="step-body">
        <h2>{props.title}</h2>
        {props.children}
      </div>
    </li>
  );
}

// ---------- 1. Material ----------

function Material({ trackId, detail, reload }: { trackId: string; detail: TrackDetail; reload: () => Promise<void> }) {
  const [progress, setProgress] = useState<Record<number, IngestProgress>>({});
  const [busy, setBusy] = useState<number | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let off = () => {};
    api.onIngestProgress((p) => setProgress((cur) => ({ ...cur, [p.folder_id]: p }))).then((f) => (off = f));
    return () => off();
  }, []);

  const ingest = async (folderId: number) => {
    setBusy(folderId);
    setError(null);
    setNote(null);
    try {
      const r = await api.ingestFolder(folderId);
      const failed = r.failed.length ? ` ${r.failed.length} file(s) couldn't be read.` : "";
      setNote(
        r.updated || r.removed
          ? `Read ${r.updated} file(s) and indexed ${r.chunks_embedded} passages.${failed}`
          : `Everything is up to date.${failed}`,
      );
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const add = async () => {
    const path = await api.pickFolder();
    if (!path) return;
    try {
      const id = await api.addFolder(trackId, path);
      await reload();
      await ingest(id);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <>
      <p className="muted">
        Connect folders of notes, PDFs and code. CPrep reads them where they are and teaches from them. It skips <code>.env</code> files,
        databases and logs, and hides passwords it finds in code.
      </p>
      {detail.folders.map((f) => {
        const p = progress[f.id];
        const running = busy === f.id;
        const pct = p && p.total ? p.done / p.total : 0;
        return (
          <div className="folder-row" key={f.id}>
            <div style={{ minWidth: 0 }}>
              <div className="folder-path">{f.path}</div>
              <div className="faint">
                {running && p
                  ? stageLabel(p)
                  : `${f.documents} files · ${f.chunks} passages${f.last_ingested ? ` · read ${new Date(f.last_ingested).toLocaleString()}` : ""}`}
              </div>
              {running && (
                <div className="progress-bar" aria-hidden="true">
                  <span style={{ transform: `scaleX(${p?.stage === "done" ? 1 : Math.max(0.03, pct)})` }} />
                </div>
              )}
            </div>
            <div className="row" style={{ marginTop: 0 }}>
              <button className="btn btn-quiet" disabled={busy !== null} onClick={() => ingest(f.id)}>
                {running ? <span className="spinner" aria-label="Reading" /> : "Re-read"}
              </button>
              <button
                className="btn btn-quiet"
                disabled={busy !== null}
                onClick={async () => {
                  const ok = await api.confirm(
                    `Disconnect ${f.path}? Your files stay where they are; CPrep forgets what it read from them.`,
                    "Disconnect",
                  );
                  if (!ok) return;
                  await api.removeFolder(f.id);
                  await reload();
                }}
              >
                Disconnect
              </button>
            </div>
          </div>
        );
      })}
      <div className="row">
        <button className={detail.folders.length ? "btn" : "btn btn-primary"} disabled={busy !== null} onClick={add}>
          Connect a folder
        </button>
      </div>
      {note && <p className="faint" role="status">{note}</p>}
      {error && <div className="error-note" role="alert">{error}</div>}
    </>
  );
}

function stageLabel(p: IngestProgress) {
  switch (p.stage) {
    case "scanning":
      return "Looking for files…";
    case "reading":
      return `Reading ${p.done + 1} of ${p.total}: ${p.current}`;
    case "embedding":
      return `Indexing passages: ${p.done} of ${p.total}`;
    default:
      return "Finishing…";
  }
}

// ---------- 2. Placement ----------

function Placement({ trackId, detail, reload }: { trackId: string; detail: TrackDetail; reload: () => Promise<void> }) {
  const [known, setKnown] = useState<string[]>(detail.known);
  const [note, setNote] = useState("");
  const [quiz, setQuiz] = useState<Question[] | null>(null);
  const [answers, setAnswers] = useState<(number | null)[]>([]);
  const [index, setIndex] = useState(0);
  const [result, setResult] = useState<PlacementResult | null>(detail.placement);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(!detail.placement && detail.known.length === 0);

  const topics = TOPICS[trackId] ?? [];
  const toggle = (t: string) => setKnown((k) => (k.includes(t) ? k.filter((x) => x !== t) : [...k, t]));

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const qs = await api.startPlacement(trackId, known, note);
      setQuiz(qs);
      setAnswers(qs.map(() => null));
      setIndex(0);
      setResult(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const submit = async () => {
    setBusy(true);
    try {
      const r = await api.submitPlacement(trackId, answers);
      setResult(r);
      setQuiz(null);
      setEditing(false);
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (quiz) {
    const q = quiz[index];
    const last = index === quiz.length - 1;
    return (
      <div className="quiz">
        <div className="quiz-meta">
          <span>{q.topic}</span>
          <span>Question {index + 1} of {quiz.length}</span>
        </div>
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={index}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
          >
            <div className="quiz-prompt"><Markdown remarkPlugins={mdPlugins} components={mdComponents}>{q.prompt}</Markdown></div>
            <div className="options" role="group" aria-label="Answers">
              {q.options.map((o, i) => (
                <button
                  key={i}
                  className="option"
                  aria-pressed={answers[index] === i}
                  onClick={() => setAnswers((a) => a.map((x, j) => (j === index ? i : x)))}
                >
                  <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{o}</Markdown>
                </button>
              ))}
            </div>
          </motion.div>
        </AnimatePresence>
        <div className="row">
          <button className="btn btn-quiet" disabled={index === 0} onClick={() => setIndex(index - 1)}>Back</button>
          {last ? (
            <button className="btn btn-primary" disabled={busy} onClick={submit}>
              {busy ? <span className="spinner" /> : null} Finish quiz
            </button>
          ) : (
            <button className="btn btn-primary" onClick={() => setIndex(index + 1)}>
              {answers[index] === null ? "Skip question" : "Next question"}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!editing && (result || detail.known.length)) {
    return (
      <>
        {result ? (
          <>
            <p className="muted">
              You scored {result.correct} of {result.total}. The course skips what you've proven and spends more time on the rest.
            </p>
            <div className="score-line">
              {result.strong.length > 0 && <span><b className="ok">Solid:</b> {result.strong.join(", ")}</span>}
              {result.weak.length > 0 && <span><b className="bad">Needs work:</b> {result.weak.join(", ")}</span>}
            </div>
            {result.review.length > 0 && <QuizReview result={result} />}
          </>
        ) : (
          <p className="muted">You said you know: {detail.known.join(", ")}. No quiz taken.</p>
        )}
        <div className="row">
          <button className="btn btn-quiet" onClick={() => setEditing(true)}>Retake placement</button>
        </div>
      </>
    );
  }

  return (
    <>
      <p className="muted">Pick what you already know. A short quiz checks it, so the course can skip the basics you have down.</p>
      <div className="chips" role="group" aria-label="Topics you know">
        {topics.map((t) => (
          <button key={t} className="chip" aria-pressed={known.includes(t)} onClick={() => toggle(t)}>{t}</button>
        ))}
      </div>
      <input
        className="text-input"
        name="background-note"
        aria-label="Anything else about what you know"
        autoComplete="off"
        placeholder="Anything else? e.g. built two Flask apps, never touched Docker…"
        value={note}
        onChange={(e) => setNote(e.target.value)}
      />
      <div className="row">
        <button className="btn btn-primary" disabled={busy} onClick={start}>
          {busy && <span className="spinner" />} {busy ? "Writing your quiz…" : "Take the 8-question quiz"}
        </button>
        <button
          className="btn btn-quiet"
          disabled={busy}
          onClick={async () => { await api.skipPlacement(trackId, known); setEditing(false); await reload(); }}
        >
          Skip the quiz
        </button>
      </div>
      {busy && <p className="faint">Local models can take a minute or two to write the questions.</p>}
      {error && <div className="error-note" role="alert">{error}</div>}
    </>
  );
}

function QuizReview({ result }: { result: PlacementResult }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <div className="row">
        <button className="btn btn-quiet" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "Hide answers" : "Review answers"}
        </button>
      </div>
      {open && (
        <div className="quiz">
          {result.review.map((r, i) => (
            <div key={i} style={{ marginBottom: 22 }}>
              <div className="quiz-prompt" style={{ fontSize: "1.1rem", margin: "6px 0 10px" }}><Markdown remarkPlugins={mdPlugins} components={mdComponents}>{r.prompt}</Markdown></div>
              <div className="options">
                {r.options.map((o, j) => (
                  <div
                    key={j}
                    className="option"
                    data-verdict={j === r.answer ? "right" : j === r.chosen ? "wrong" : undefined}
                  >
                    <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{o}</Markdown>
                  </div>
                ))}
              </div>
              <p className="faint">{r.explanation}</p>
            </div>
          ))}
        </div>
      )}
    </>
  );
}

// ---------- 3. Course ----------

function Course(props: {
  trackId: string;
  detail: TrackDetail;
  ready: boolean;
  reload: () => Promise<void>;
  onOpenLesson: (id: number) => void;
}) {
  const { detail } = props;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const c = detail.curriculum;

  const generate = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.generateCurriculum(props.trackId);
      await props.reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const grouped = useMemo(() => {
    if (!c) return [];
    return c.phases.map((p, pi) => ({
      ...p,
      modules: p.modules.map((m, mi) => ({
        ...m,
        lessons: c.lessons.filter((l) => l.phase_idx === pi && l.module_idx === mi),
      })),
    }));
  }, [c]);

  const nextLesson: Lesson | undefined = c?.lessons.find((l) => l.status === "in_progress") ?? c?.lessons.find((l) => l.status === "todo");

  if (!c) {
    return (
      <>
        <p className="muted">
          CPrep designs a course from the track syllabus, your quiz results and the files you connected. You can rebuild it any time.
        </p>
        <div className="row">
          <button className="btn btn-primary" disabled={busy || !props.ready} onClick={generate}>
            {busy && <span className="spinner" />} {busy ? "Designing your course…" : "Design my course"}
          </button>
        </div>
        {!props.ready && <p className="faint">Finish step 2 first, or skip the quiz.</p>}
        {busy && <p className="faint">This is the biggest job. A local model can take several minutes.</p>}
        {error && <div className="error-note" role="alert">{error}</div>}
      </>
    );
  }

  const done = c.lessons.filter((l) => l.status === "done" || l.status === "skipped").length;

  return (
    <>
      <p className="muted">{c.title}: {done} of {c.lessons.length} lessons done or skipped. Open any lesson to go back over it.</p>
      <div className="row">
        {nextLesson && (
          <button className="btn btn-primary" onClick={() => props.onOpenLesson(nextLesson.id)}>
            {nextLesson.status === "in_progress" ? "Continue" : "Start"}: {nextLesson.title}
          </button>
        )}
        <button className="btn btn-quiet" disabled={busy} onClick={generate}>
          {busy ? <span className="spinner" /> : null} Rebuild course
        </button>
      </div>
      {error && <div className="error-note" role="alert">{error}</div>}
      {grouped.map((p, pi) => (
        <section className="phase" key={pi}>
          <h3>{p.title}</h3>
          {p.goal && <p className="phase-goal">{p.goal}</p>}
          {p.modules.map((m, mi) => (
            <div key={mi}>
              <div className="module-title">
                {m.title}
                {m.tested_out && <span className="tag">tested out</span>}
              </div>
              {m.lessons.map((l) => (
                <button key={l.id} className="lesson-row" data-status={l.status} onClick={() => props.onOpenLesson(l.id)}>
                  <span className="status-mark" data-status={l.status} aria-label={statusLabel(l.status)} />
                  <span>{l.title}</span>
                  <span className="hint">{statusLabel(l.status)}</span>
                </button>
              ))}
            </div>
          ))}
        </section>
      ))}
    </>
  );
}

export function statusLabel(s: Lesson["status"]) {
  return { todo: "", in_progress: "In progress", done: "Done", skipped: "Skipped" }[s];
}
