import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, NoteEntry, TrackSummary } from "../api";
import { Icon, MdIcon, remarkEmojiIcons } from "../icons";
import { ListenButton } from "./VoiceControls";

type Props = { tracks: TrackSummary[] };

const mdPlugins = [remarkGfm, remarkEmojiIcons];
const mdComponents = { i: MdIcon as never };
const dateFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });

/** Notes may contain <details> answer blocks. Raw HTML isn't rendered (model output), so turn them into Markdown. */
export function cleanNotes(md: string, hideAnswers = false) {
  return md.replace(/<details>\s*<summary>([\s\S]*?)<\/summary>([\s\S]*?)<\/details>/g, (_, summary: string, body: string) =>
    hideAnswers ? "" : `\n**${summary.trim()}**\n\n${body.trim()}\n`,
  );
}

export type Slide = { title: string; body: string; kind: "title" | "content" };

/** One slide per `##` section; long sections are split so no slide carries more than 6 bullets. */
export function toSlides(md: string): Slide[] {
  const text = cleanNotes(md, true);
  const title = text.match(/^#\s+(.+)$/m)?.[1]?.trim() ?? "Notes";
  const meta = text.match(/^_(.+)_\s*$/m)?.[1] ?? "";
  const sections = text.split(/^##\s+/m);
  const intro = sections[0].replace(/^#\s+.+$/m, "").replace(/^_.+_\s*$/m, "").trim();
  const slides: Slide[] = [{ kind: "title", title, body: [meta, intro].filter(Boolean).join("\n\n") }];
  for (const sec of sections.slice(1)) {
    const [head, ...rest] = sec.split("\n");
    const body = rest.join("\n").trim();
    // Split into blocks (code fences kept whole), then pack them into slides.
    const blocks = body.split(/\n(?=```)|(?<=```)\n/).flatMap((b) => (b.startsWith("```") ? [b] : b.split(/\n(?=- |\d+\. )/)));
    let cur: string[] = [];
    let bullets = 0;
    const flush = (cont: boolean) => {
      if (cur.length) slides.push({ kind: "content", title: head.trim() + (cont ? " (cont.)" : ""), body: cur.join("\n").trim() });
      cur = [];
      bullets = 0;
    };
    let part = 0;
    for (const b of blocks) {
      const isBullet = /^(- |\d+\. )/.test(b.trim());
      const isCode = b.trim().startsWith("```");
      if ((isBullet && bullets >= 6) || (isCode && cur.some((c) => c.trim().startsWith("```")))) {
        flush(part++ > 0);
      }
      cur.push(b);
      if (isBullet) bullets++;
    }
    flush(part > 0);
  }
  return slides;
}

export default function Notes({ tracks }: Props) {
  const [notes, setNotes] = useState<NoteEntry[] | null>(null);
  const [open, setOpen] = useState<NoteEntry | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [presenting, setPresenting] = useState(false);
  const [printing, setPrinting] = useState<"notes" | "slides" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.listNotes().then(setNotes).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (!open) return;
    setText(null);
    api.readNotes(open.lesson_id).then(setText).catch((e) => setError(String(e)));
  }, [open]);

  // Print through the system dialog ("Microsoft Print to PDF" saves a PDF).
  useEffect(() => {
    if (!printing) return;
    document.body.dataset.print = printing;
    const done = () => {
      delete document.body.dataset.print;
      setPrinting(null);
    };
    window.addEventListener("afterprint", done, { once: true });
    requestAnimationFrame(() => requestAnimationFrame(() => window.print()));
    return () => window.removeEventListener("afterprint", done);
  }, [printing]);

  const trackName = (id: string) => tracks.find((t) => t.id === id)?.name ?? id;
  const accent = (id: string) => tracks.find((t) => t.id === id)?.accent ?? "#97a39c";
  const slides = useMemo(() => (text ? toSlides(text) : []), [text]);

  if (open) {
    return (
      <div className="page notes-reader">
        <div className="notes-bar">
          <button className="btn btn-quiet" onClick={() => { setOpen(null); setText(null); }}>
            <Icon name="back" size={16} /> All notes
          </button>
          <div className="grow" />
          <button className="btn" disabled={!text} onClick={() => setPresenting(true)}>
            <Icon name="play" size={14} /> Present as slides
          </button>
          <button className="btn btn-quiet" disabled={!text} onClick={() => setPrinting("notes")}>Save notes as PDF</button>
          <button className="btn btn-quiet" disabled={!text} onClick={() => setPrinting("slides")}>Save slides as PDF</button>
          <button className="btn btn-quiet" onClick={() => api.openNotes(open.path).catch((e) => setError(String(e)))}>Open in editor</button>
        </div>
        {error && <div className="error-note" role="alert">{error}</div>}
        {text === null ? (
          <span className="spinner" aria-label="Loading notes" />
        ) : (
          <article className="notes-article prose print-notes">
            <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{cleanNotes(text)}</Markdown>
            <ListenButton text={cleanNotes(text, true)} id={`notes-${open.lesson_id}`} />
          </article>
        )}
        {presenting && slides.length > 0 && <Presenter slides={slides} accent={accent(open.track_id)} onClose={() => setPresenting(false)} />}
        {printing === "slides" &&
          createPortal(
            <div className="print-slides" aria-hidden="true">
              {slides.map((s, i) => <SlideView key={i} slide={s} index={i} total={slides.length} accent={accent(open.track_id)} />)}
            </div>,
            document.body,
          )}
      </div>
    );
  }

  const groups = (notes ?? []).reduce<Record<string, NoteEntry[]>>((g, n) => ((g[n.track_id] ??= []).push(n), g), {});
  return (
    <div className="page notes-library">
      <h1 className="page-title">Notes</h1>
      <p className="page-lede">
        Study notes CPrep wrote from your lessons. Read them here, present them as slides, or save them as PDF. The files live in
        your Documents folder, so they're yours outside the app too.
      </p>
      {error && <div className="error-note" role="alert">{error}</div>}
      {notes === null ? (
        <span className="spinner" aria-label="Loading" />
      ) : notes.length === 0 ? (
        <div className="review-empty">
          <p className="review-empty-title">No notes yet.</p>
          <p className="muted">Open a lesson and choose Write notes, or mark a lesson complete and notes are written for you.</p>
        </div>
      ) : (
        Object.entries(groups).map(([trackId, list]) => (
          <section key={trackId} className="notes-group" aria-labelledby={`ng-${trackId}`}>
            <h2 id={`ng-${trackId}`}>
              <span className="track-dot" style={{ background: accent(trackId) }} aria-hidden="true" /> {trackName(trackId)}
            </h2>
            <ul>
              {list.map((n) => (
                <li key={n.lesson_id}>
                  <button className="history-row" onClick={() => setOpen(n)}>
                    <span>{n.title}</span>
                    <span className="faint">{n.phase} · {n.module}</span>
                    <span className="history-verdict">{dateFmt.format(new Date(n.created_at))}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))
      )}
      <div className="row">
        <button className="btn btn-quiet" onClick={() => api.openNotes(null).catch((e) => setError(String(e)))}>Open the notes folder</button>
      </div>
    </div>
  );
}

function SlideView({ slide, index, total, accent }: { slide: Slide; index: number; total: number; accent: string }) {
  return (
    <section className="slide" data-kind={slide.kind} style={{ ["--track" as string]: accent }}>
      <h2 className="slide-title">{slide.title}</h2>
      <div className="slide-body prose">
        <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{slide.body}</Markdown>
      </div>
      <div className="slide-foot">
        <span>CPrep</span>
        <span>{index + 1} / {total}</span>
      </div>
    </section>
  );
}

function Presenter({ slides, accent, onClose }: { slides: Slide[]; accent: string; onClose: () => void }) {
  const [i, setI] = useState(0);
  const root = useRef<HTMLDivElement>(null);

  // Modal focus: move focus in, keep Tab inside, and give it back to the opener on close.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    root.current?.querySelector<HTMLElement>(".presenter-bar button:not([disabled])")?.focus();
    const trap = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || !root.current) return;
      const items = [...root.current.querySelectorAll<HTMLElement>("button:not([disabled])")];
      if (!items.length) return;
      const first = items[0], last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    };
    document.addEventListener("keydown", trap);
    return () => {
      document.removeEventListener("keydown", trap);
      opener?.focus?.();
    };
  }, []);
  const go = useCallback((n: number) => setI((cur) => Math.max(0, Math.min(slides.length - 1, cur + n))), [slides.length]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (["ArrowRight", "PageDown", " "].includes(e.key)) { e.preventDefault(); go(1); }
      else if (["ArrowLeft", "PageUp"].includes(e.key)) { e.preventDefault(); go(-1); }
      else if (e.key === "Home") setI(0);
      else if (e.key === "End") setI(slides.length - 1);
      else if (e.key === "Escape") onClose();
      else if (e.key.toLowerCase() === "f") {
        if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
        else document.documentElement.requestFullscreen().catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    };
  }, [go, onClose, slides.length]);

  return createPortal(
    <div className="presenter" role="dialog" aria-modal="true" aria-label="Slides" ref={root}>
      <div className="presenter-stage" onClick={(e) => (e.clientX > window.innerWidth / 2 ? go(1) : go(-1))}>
        <SlideView slide={slides[i]} index={i} total={slides.length} accent={accent} />
      </div>
      <div className="presenter-bar">
        <button className="icon-btn" aria-label="Previous slide" disabled={i === 0} onClick={() => go(-1)}><Icon name="back" size={16} /></button>
        <span className="faint">{i + 1} / {slides.length} · ←/→ to move · F for full screen · Esc to close</span>
        <button className="icon-btn" aria-label="Next slide" disabled={i === slides.length - 1} onClick={() => go(1)}><Icon name="chevron" size={16} /></button>
        <button className="btn btn-quiet" onClick={onClose}>Close</button>
      </div>
    </div>,
    document.body,
  );
}
