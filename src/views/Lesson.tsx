import { KeyboardEvent, Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, LessonView as LessonViewT, Message, Source, TrackSummary } from "../api";
import { Icon, MdIcon, remarkEmojiIcons } from "../icons";
import { LessonProgress, topicStates } from "./CourseNav";
import { CanvasBridge, Diagram } from "./Diagram";

const Lab = lazy(() => import("./Lab"));
const Canvas = lazy(() => import("./Canvas"));
const PANEL_KEY = "cprep.panel";
type Panel = "practice" | "canvas" | null;

type Props = {
  track: TrackSummary;
  lessonId: number;
  live: boolean;
  onBack: () => void;
  onOpenLesson: (id: number) => void;
  onChanged: () => void;
  onProgress: (p: LessonProgress) => void;
};

type Pending = { user: string; text: string; sources: Source[] };

const INDENT = "    ";
const MARKER_RE = /\[\[\s*topic\s*:\s*\d+\s*\]\]\s*/g;

/** Hide topic markers, including one that is still arriving mid-stream. */
function stripMarkers(text: string) {
  return text.replace(MARKER_RE, "").replace(/^\s*\[\[[^\]]*\]?$/, "");
}

export default function LessonView({ track, lessonId, live, onBack, onOpenLesson, onChanged, onProgress }: Props) {
  const [view, setView] = useState<LessonViewT | null>(null);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [escalate, setEscalate] = useState(false);
  const [lastMeta, setLastMeta] = useState<string | null>(null);
  const [celebrating, setCelebrating] = useState(false);
  const [panel, setPanelState] = useState<Panel>(() => {
    try {
      const v = localStorage.getItem(PANEL_KEY);
      return v === "practice" || v === "canvas" ? v : null;
    } catch {
      return null;
    }
  });
  const [pendingMermaid, setPendingMermaid] = useState<string | null>(null);
  const [notesPath, setNotesPath] = useState<string | null>(null);
  const [notesBusy, setNotesBusy] = useState(false);
  const [notesMsg, setNotesMsg] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const clearPendingMermaid = useCallback(() => setPendingMermaid(null), []);
  const started = useRef(false);
  const leaveOnTab = useRef(false);

  const load = useCallback(async () => {
    const v = await api.getLesson(lessonId);
    setView(v);
    const taught = [...new Set(v.messages.filter((m) => m.role === "assistant" && m.topic_idx != null).map((m) => m.topic_idx!))];
    onProgress({ lessonId, topicIdx: v.lesson.topic_idx, status: v.lesson.status, taught });
    return v;
  }, [lessonId, onProgress]);

  const send = useCallback(
    async (text: string) => {
      setError(null);
      setPending({ user: text, text: "", sources: [] });
      try {
        await api.sendMessage(lessonId, text, escalate, (e) => {
          if (e.type === "sources") setPending((p) => (p ? { ...p, sources: e.sources } : p));
          if (e.type === "delta") setPending((p) => (p ? { ...p, text: p.text + e.text } : p));
          if (e.type === "done") setLastMeta(`${e.provider}${e.costUsd > 0 ? ` · $${e.costUsd.toFixed(4)}` : " · free"}`);
          if (e.type === "error") setError(e.message);
        });
        await load();
        onChanged();
      } catch (e) {
        setError(String(e));
        if (text) setDraft(text);
      } finally {
        setPending(null);
      }
    },
    [lessonId, escalate, load, onChanged],
  );

  useEffect(() => {
    started.current = false;
    setView(null);
    setLastMeta(null);
    setNotesMsg(null);
    api.notesPath(lessonId).then(setNotesPath).catch(() => setNotesPath(null));
    load().then((v) => {
      if (v.messages.length === 0 && !started.current) {
        started.current = true;
        send("");
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lessonId]);

  // Keep the newest text in view while streaming.
  useEffect(() => {
    const el = scroller.current;
    if (el && pending) el.scrollTop = el.scrollHeight;
  }, [pending?.text, pending]);

  useEffect(() => {
    const el = scroller.current;
    if (el && view) el.scrollTop = el.scrollHeight;
  }, [view?.messages.length, view]);

  if (!view) return <div className="page"><span className="spinner" aria-label="Loading lesson" /></div>;
  const { lesson } = view;
  const states = topicStates(lesson, null);
  const currentTopic = Math.min(lesson.topic_idx, Math.max(0, lesson.objectives.length - 1));

  const submit = () => {
    const t = draft.trim();
    if (!t || pending) return;
    setDraft("");
    send(t);
  };

  // Tab indents like a code editor; Esc then Tab moves focus on as usual.
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const ta = e.currentTarget;
    if (e.key === "Escape") {
      leaveOnTab.current = true;
      return;
    }
    if (e.key === "Tab" && !leaveOnTab.current) {
      e.preventDefault();
      const { selectionStart: start, selectionEnd: end, value } = ta;
      const lineStart = value.lastIndexOf("\n", start - 1) + 1;
      let next: string;
      let cursor: number;
      if (e.shiftKey) {
        const lead = value.slice(lineStart).match(/^ {1,4}/)?.[0].length ?? 0;
        next = value.slice(0, lineStart) + value.slice(lineStart + lead);
        cursor = Math.max(lineStart, start - lead);
      } else {
        next = value.slice(0, start) + INDENT + value.slice(end);
        cursor = start + INDENT.length;
      }
      setDraft(next);
      requestAnimationFrame(() => ta.setSelectionRange(cursor, cursor));
      return;
    }
    leaveOnTab.current = false;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  const setPanel = (next: Panel) => {
    setPanelState(next);
    try {
      localStorage.setItem(PANEL_KEY, next ?? "none");
    } catch {
      /* storage unavailable */
    }
  };
  const openOnCanvas = (code: string) => {
    setPendingMermaid(code);
    setPanel("canvas");
  };

  const writeNotes = async () => {
    setNotesBusy(true);
    setNotesMsg(null);
    try {
      const path = await api.writeNotes(lesson.id);
      setNotesPath(path);
      setNotesMsg(`Notes saved to ${path}`);
    } catch (e) {
      setNotesMsg(String(e));
    } finally {
      setNotesBusy(false);
    }
  };

  const markDone = async () => {
    await api.setLessonStatus(lesson.id, "done");
    onChanged();
    // Notes are written in the background; they'll be ready in the notes folder.
    api.writeNotes(lesson.id).catch(() => {});
    setCelebrating(true);
  };

  const afterCelebration = () => {
    setCelebrating(false);
    if (view.next_id) onOpenLesson(view.next_id);
    else onBack();
  };

  const retake = async () => {
    const ok = await api.confirm("Retake this lesson? The conversation so far will be cleared and the tutor starts again.", "Retake");
    if (!ok) return;
    await api.retakeLesson(lesson.id);
    onChanged();
    started.current = true;
    await load();
    send("");
  };

  // Mark where each topic starts in the conversation, so the sidebar can jump there.
  const seen = new Set<number>();

  return (
    <div className="lesson">
      <header className="lesson-bar">
        <button className="btn btn-quiet" onClick={onBack} aria-label={`Back to ${track.name}`}>
          <Icon name="back" size={16} />
          Course
        </button>
        <div className="grow">
          <div className="lesson-crumbs">
            {lesson.phase} / {lesson.module}
            {lesson.objectives.length > 0 && lesson.status !== "done" && (
              <> · Topic {currentTopic + 1} of {lesson.objectives.length}</>
            )}
          </div>
          <div className="lesson-title-sm">{lesson.title}</div>
        </div>
        <div className="lesson-actions">
          <button className="btn btn-quiet" disabled={!view.prev_id} onClick={() => view.prev_id && onOpenLesson(view.prev_id)}>
            Previous
          </button>
          <button className="btn btn-quiet" disabled={!!pending} onClick={retake}>Retake</button>
          <button
            className="btn btn-quiet"
            disabled={notesBusy || !!pending}
            onClick={() => (notesPath ? api.openNotes(notesPath).catch((e) => setNotesMsg(String(e))) : writeNotes())}
          >
            {notesBusy ? <span className="spinner" /> : <Icon name="notes" size={15} />}
            {notesBusy ? "Writing notes…" : notesPath ? "Open notes" : "Write notes"}
          </button>
          <button className="btn btn-quiet" aria-pressed={panel === "practice"} onClick={() => setPanel(panel === "practice" ? null : "practice")}>
            <Icon name="code" size={15} />
            Practice
          </button>
          <button className="btn btn-quiet" aria-pressed={panel === "canvas"} onClick={() => setPanel(panel === "canvas" ? null : "canvas")}>
            <Icon name="canvas" size={15} />
            Canvas
          </button>
          <button className="btn" disabled={!!pending || lesson.status === "done"} onClick={markDone}>
            {lesson.status === "done" ? "Completed" : "Mark complete"}
          </button>
        </div>
      </header>

      {notesMsg && (
        <div className="notes-strip" role="status">
          <span>{notesMsg}</span>
          {notesPath && (
            <>
              <button className="btn btn-quiet" onClick={() => api.openNotes(notesPath)}>Open</button>
              <button className="btn btn-quiet" onClick={() => api.openNotes(null)}>Show folder</button>
              <button className="btn btn-quiet" disabled={notesBusy} onClick={writeNotes}>
                <Icon name="refresh" size={14} /> Rewrite
              </button>
            </>
          )}
          <button className="icon-btn" aria-label="Dismiss" onClick={() => setNotesMsg(null)}>
            <Icon name="close" size={14} />
          </button>
        </div>
      )}

      <CanvasBridge.Provider value={openOnCanvas}>
      <div className="lesson-body" data-lab={panel !== null} data-panel={panel ?? "none"}>
      <div className="lesson-chat">
      <div className="lesson-scroll" ref={scroller}>
        <section className="lesson-intro">
          <div>
            <h1>{lesson.title}</h1>
            <ol className="intro-topics">
              {lesson.objectives.map((o, i) => (
                <li key={i} data-state={states[i]}>{o}</li>
              ))}
            </ol>
          </div>
        </section>

        {view.messages.map((m) => {
          let anchor: number | null = null;
          if (m.role === "assistant" && m.topic_idx != null && !seen.has(m.topic_idx)) {
            seen.add(m.topic_idx);
            anchor = m.topic_idx;
          }
          return (
            <div key={m.id}>
              {anchor != null && (
                <div className="topic-divider" id={`topic-${anchor}`}>
                  <span>Topic {anchor + 1}</span> {lesson.objectives[anchor]}
                </div>
              )}
              <Turn message={m} />
            </div>
          );
        })}
        {pending && (
          <>
            {pending.user && <Turn message={{ id: -1, role: "user", content: pending.user, sources: [], created_at: "", topic_idx: null }} />}
            <Turn
              message={{ id: -2, role: "assistant", content: stripMarkers(pending.text), sources: pending.sources, created_at: "", topic_idx: null }}
              streaming
            />
          </>
        )}
        {error && (
          <div className="empty-lesson">
            <div className="error-note" role="alert">{error}</div>
            <div className="row">
              <button className="btn" onClick={() => send(view.messages.length ? "Please continue." : "")}>Try again</button>
            </div>
          </div>
        )}
      </div>

      <footer className="composer">
        <div className="composer-inner">
          <label htmlFor="reply" className="faint composer-label">
            Answer the tutor, or ask anything about this lesson
            <span className="keys">Tab indents · Esc then Tab leaves the box</span>
          </label>
          <textarea
            id="reply"
            name="reply"
            autoComplete="off"
            spellCheck={false}
            value={draft}
            rows={3}
            placeholder="Type your answer… Enter sends, Shift+Enter adds a line"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            onBlur={() => (leaveOnTab.current = false)}
          />
          <div className="composer-row">
            <span className="faint">{lastMeta ?? (live ? "Live: Claude teaches" : "Test mode: free model")}</span>
            <div className="row" style={{ marginTop: 0 }}>
              {live && (
                <label className="toggle">
                  <input type="checkbox" checked={escalate} onChange={(e) => setEscalate(e.target.checked)} />
                  Ask the expert model
                </label>
              )}
              <button className="btn btn-primary" disabled={!draft.trim() || !!pending} onClick={submit}>
                {pending ? <span className="spinner" aria-label="Tutor is replying" /> : "Send"}
              </button>
            </div>
          </div>
        </div>
      </footer>
      </div>
      {panel === "practice" && (
        <Suspense fallback={<aside className="lab"><span className="spinner" aria-label="Loading practice" /></aside>}>
          <Lab lessonId={lesson.id} topic={lesson.objectives[currentTopic] ?? lesson.title} onClose={() => setPanel(null)} />
        </Suspense>
      )}
      {panel === "canvas" && (
        <Suspense fallback={<aside className="lab"><span className="spinner" aria-label="Loading canvas" /></aside>}>
          <Canvas
            lessonId={lesson.id}
            pendingMermaid={pendingMermaid}
            onMermaidPlaced={clearPendingMermaid}
            onClose={() => setPanel(null)}
          />
        </Suspense>
      )}
      </div>
      </CanvasBridge.Provider>

      <AnimatePresence>
        {celebrating && <Celebration title={lesson.title} hasNext={!!view.next_id} onDone={afterCelebration} />}
      </AnimatePresence>
    </div>
  );
}

/** Turn [S2] into clickable citation marks, but leave code blocks alone. */
function linkCitations(md: string) {
  return md
    .split(/(```[\s\S]*?```)/g)
    .map((part, i) => (i % 2 === 1 ? part : part.replace(/\[S(\d+)\]/g, "[S$1](#cite-$1)")))
    .join("");
}

function Turn({ message, streaming }: { message: Message; streaming?: boolean }) {
  const [active, setActive] = useState<number | null>(null);
  if (message.role === "user") {
    if (!message.content) return null;
    return (
      <div className="turn turn-user">
        <div className="said">{message.content}</div>
      </div>
    );
  }
  const sources = message.sources;
  return (
    <div className="turn">
      <div className="prose">
        <Markdown
          remarkPlugins={[remarkGfm, remarkEmojiIcons]}
          components={{
            i: MdIcon as never,
            pre: ({ node, children }) => {
              const codeEl = node?.children?.[0] as { tagName?: string; properties?: { className?: unknown }; children?: { value?: string }[] } | undefined;
              const cls = codeEl?.properties?.className;
              if (codeEl?.tagName === "code" && Array.isArray(cls) && cls.includes("language-mermaid")) {
                const src = (codeEl.children ?? []).map((c) => c.value ?? "").join("");
                return <Diagram code={src} streaming={streaming} />;
              }
              return <pre>{children}</pre>;
            },
            a: ({ href, children }) => {
              const m = href?.match(/^#cite-(\d+)$/);
              if (m) {
                const n = Number(m[1]);
                return (
                  <button className="cite" title={sources[n - 1]?.title ?? "Source"} onClick={() => setActive(n)}>
                    {children}
                  </button>
                );
              }
              return <a href={href} target="_blank" rel="noreferrer">{children}</a>;
            },
          }}
        >
          {linkCitations(message.content)}
        </Markdown>
        {streaming && <span className="caret" aria-hidden="true" />}
      </div>
      {sources.length > 0 && (
        <aside className="margin" aria-label="Sources from your files">
          <ol>
            {sources.map((s, i) => (
              <li key={s.chunk_id} data-active={active === i + 1}>
                <span className="src-n">S{i + 1}</span>
                <span className="src-title">{s.title.split("/").pop()}</span>
                {s.page != null && <span className="faint"> · p. {s.page}</span>}
                <div className="src-snip">{s.text.replace(/^\[[^\]]*\]\n/, "")}</div>
              </li>
            ))}
          </ol>
        </aside>
      )}
    </div>
  );
}

/** One short moment when a lesson is finished: an SVG burst, then on to the next lesson. */
function Celebration({ title, hasNext, onDone }: { title: string; hasNext: boolean; onDone: () => void }) {
  const reduce = useReducedMotion();
  const done = useRef(onDone);
  done.current = onDone;
  useEffect(() => {
    const t = setTimeout(() => done.current(), reduce ? 1100 : 1700);
    return () => clearTimeout(t);
  }, [reduce]);

  const rays = Array.from({ length: 12 }, (_, i) => i);
  const colors = ["#F0C35A", "#9CC58A", "#E9E5D9", "#E58FA0", "#5B9BD5"];

  return (
    <motion.div
      className="celebrate"
      role="status"
      aria-live="polite"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      onClick={onDone}
    >
      <div className="celebrate-card">
        <svg width="120" height="120" viewBox="-60 -60 120 120" aria-hidden="true">
          {rays.map((i) => {
            const a = (i / rays.length) * Math.PI * 2;
            return (
              <motion.rect
                key={i}
                x={-2}
                y={-6}
                width={4}
                height={12}
                rx={1.5}
                fill={colors[i % colors.length]}
                initial={reduce ? { x: Math.cos(a) * 38 - 2, y: Math.sin(a) * 38 - 6, rotate: (a * 180) / Math.PI } : { x: -2, y: -6, opacity: 0, rotate: 0 }}
                animate={{ x: Math.cos(a) * 44 - 2, y: Math.sin(a) * 44 - 6, opacity: reduce ? 1 : [0, 1, 0.9], rotate: (a * 180) / Math.PI + 90 }}
                transition={{ duration: reduce ? 0 : 0.7, ease: [0.22, 1, 0.36, 1], delay: reduce ? 0 : (i % 3) * 0.04 }}
              />
            );
          })}
          <motion.g
            initial={reduce ? false : { scale: 0.6, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1] }}
          >
            <circle r="22" fill="#F0C35A" />
            <path d="M-9 1l6 6 12-14" stroke="#1B1608" strokeWidth="3.2" fill="none" strokeLinecap="round" strokeLinejoin="round" />
          </motion.g>
        </svg>
        <div className="celebrate-title">Lesson complete</div>
        <div className="faint">{title}{hasNext ? " · opening the next lesson" : ""}</div>
      </div>
    </motion.div>
  );
}
