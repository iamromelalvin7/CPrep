import { useCallback, useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, LessonView as LessonViewT, Message, Source, TrackSummary } from "../api";

type Props = {
  track: TrackSummary;
  lessonId: number;
  live: boolean;
  onBack: () => void;
  onOpenLesson: (id: number) => void;
  onChanged: () => void;
};

type Pending = { user: string; text: string; sources: Source[] };

export default function LessonView({ track, lessonId, live, onBack, onOpenLesson, onChanged }: Props) {
  const [view, setView] = useState<LessonViewT | null>(null);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [escalate, setEscalate] = useState(false);
  const [lastMeta, setLastMeta] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const started = useRef(false);

  const load = useCallback(async () => {
    const v = await api.getLesson(lessonId);
    setView(v);
    return v;
  }, [lessonId]);

  const send = useCallback(
    async (text: string) => {
      setError(null);
      setPending({ user: text, text: "", sources: [] });
      try {
        await api.sendMessage(lessonId, text, escalate, (e) => {
          if (e.type === "sources") setPending((p) => (p ? { ...p, sources: e.sources } : p));
          if (e.type === "delta") setPending((p) => (p ? { ...p, text: p.text + e.text } : p));
          if (e.type === "done")
            setLastMeta(`${e.provider}${e.costUsd > 0 ? ` · $${e.costUsd.toFixed(4)}` : " · free"}`);
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

  const submit = () => {
    const t = draft.trim();
    if (!t || pending) return;
    setDraft("");
    send(t);
  };

  const markDone = async () => {
    await api.setLessonStatus(lesson.id, "done");
    onChanged();
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

  return (
    <div className="lesson">
      <header className="lesson-bar">
        <button className="btn btn-quiet" onClick={onBack} aria-label={`Back to ${track.name}`}>
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M10 3L5 8l5 5" stroke="currentColor" strokeWidth="1.6" fill="none" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          Course
        </button>
        <div className="grow">
          <div className="lesson-crumbs">{lesson.phase} / {lesson.module}</div>
          <div className="lesson-title-sm">{lesson.title}</div>
        </div>
        <div className="lesson-actions">
          <button className="btn btn-quiet" disabled={!view.prev_id} onClick={() => view.prev_id && onOpenLesson(view.prev_id)}>
            Previous
          </button>
          <button className="btn btn-quiet" disabled={!!pending} onClick={retake}>Retake</button>
          <button className="btn" disabled={!!pending || lesson.status === "done"} onClick={markDone}>
            {lesson.status === "done" ? "Completed" : "Mark complete"}
          </button>
        </div>
      </header>

      <div className="lesson-scroll" ref={scroller}>
        <section className="lesson-intro">
          <div>
            <h1>{lesson.title}</h1>
            <ul className="objectives">
              {lesson.objectives.map((o, i) => <li key={i}>{o}</li>)}
            </ul>
          </div>
        </section>

        {view.messages.map((m) => <Turn key={m.id} message={m} />)}
        {pending && (
          <>
            {pending.user && <Turn message={{ id: -1, role: "user", content: pending.user, sources: [], created_at: "" }} />}
            <Turn
              message={{ id: -2, role: "assistant", content: pending.text, sources: pending.sources, created_at: "" }}
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
          <label htmlFor="reply" className="faint" style={{ display: "block", marginBottom: 6 }}>
            Answer the tutor, or ask anything about this lesson
          </label>
          <textarea
            id="reply"
            value={draft}
            rows={2}
            name="reply"
            autoComplete="off"
            placeholder="Type your answer… Enter sends, Shift+Enter adds a line"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                submit();
              }
            }}
          />
          <div className="composer-row">
            <span className="faint">{lastMeta ?? (live ? "Live: Claude teaches" : "Test mode: free local model")}</span>
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
          remarkPlugins={[remarkGfm]}
          components={{
            a: ({ href, children }) => {
              const m = href?.match(/^#cite-(\d+)$/);
              if (m) {
                const n = Number(m[1]);
                return (
                  <button
                    className="cite"
                    title={sources[n - 1]?.title ?? "Source"}
                    onClick={() => setActive(n)}
                  >
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
