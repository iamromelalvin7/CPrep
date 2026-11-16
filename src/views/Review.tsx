import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, Card, LessonCards, ReviewSummary, TrackSummary } from "../api";
import { MdIcon, remarkEmojiIcons } from "../icons";

type Props = { tracks: TrackSummary[]; onChanged: () => void };

const mdPlugins = [remarkGfm, remarkEmojiIcons];
const mdComponents = { i: MdIcon as never };
const RATINGS = [
  { n: 1 as const, label: "Again", hint: "Forgot it" },
  { n: 2 as const, label: "Hard", hint: "Got it, with effort" },
  { n: 3 as const, label: "Good", hint: "Got it" },
  { n: 4 as const, label: "Easy", hint: "Instantly" },
];

function relative(iso: string) {
  const mins = Math.round((new Date(iso).getTime() - Date.now()) / 60000);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (Math.abs(mins) < 60) return rtf.format(mins, "minute");
  if (Math.abs(mins) < 60 * 24) return rtf.format(Math.round(mins / 60), "hour");
  return rtf.format(Math.round(mins / 1440), "day");
}

export default function Review({ tracks, onChanged }: Props) {
  const [summary, setSummary] = useState<ReviewSummary | null>(null);
  const [trackFilter, setTrackFilter] = useState<string | null>(null);
  const [queue, setQueue] = useState<Card[] | null>(null);
  const [done, setDone] = useState(0);
  const [revealed, setRevealed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reduce = useReducedMotion();

  const load = useCallback(async () => {
    const [s, q] = await Promise.all([api.reviewSummary(), api.dueCards(trackFilter)]);
    setSummary(s);
    setQueue(q);
    setDone(0);
    setRevealed(false);
  }, [trackFilter]);

  useEffect(() => {
    load().catch((e) => setError(String(e)));
  }, [load]);

  const card = queue?.[0] ?? null;

  const rate = useCallback(
    async (n: 1 | 2 | 3 | 4) => {
      if (!card) return;
      try {
        await api.rateCard(card.id, n);
        setQueue((q) => {
          if (!q) return q;
          const rest = q.slice(1);
          // "Again" comes back in 10 minutes: keep it at the end of this sitting.
          return n === 1 ? [...rest, card] : rest;
        });
        if (n !== 1) setDone((d) => d + 1);
        setRevealed(false);
        onChanged();
      } catch (e) {
        setError(String(e));
      }
    },
    [card, onChanged],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target instanceof Element ? e.target : null;
      if (!card || target?.closest("input, textarea, select")) return;
      // Space/Enter on a focused button already clicks it; don't handle it twice.
      const onButton = !!target?.closest("button");
      if (!revealed && (e.key === " " || e.key === "Enter") && !onButton) {
        e.preventDefault();
        setRevealed(true);
      } else if (revealed && ["1", "2", "3", "4"].includes(e.key)) {
        rate(Number(e.key) as 1 | 2 | 3 | 4);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [card, revealed, rate]);

  const trackName = (id: string) => tracks.find((t) => t.id === id)?.name ?? id;
  const accent = (id: string) => tracks.find((t) => t.id === id)?.accent ?? "#97a39c";
  const total = (queue?.length ?? 0) + done;

  return (
    <div className="page review">
      <h1 className="page-title">Review</h1>
      <p className="page-lede">
        Short questions from lessons you've done, shown just before you'd forget them. Be honest with the rating: it decides when you
        see each card again.
      </p>
      {error && <div className="error-note" role="alert">{error}</div>}

      {summary && summary.by_track.length > 1 && (
        <div className="chips" role="group" aria-label="Filter by track">
          <button className="chip" aria-pressed={trackFilter === null} onClick={() => setTrackFilter(null)}>All tracks</button>
          {summary.by_track.map(([id, n]) => (
            <button key={id} className="chip" aria-pressed={trackFilter === id} onClick={() => setTrackFilter(id)}>
              {trackName(id)} · {n}
            </button>
          ))}
        </div>
      )}

      <section className="review-stage" aria-live="polite">
        {queue === null ? (
          <span className="spinner" aria-label="Loading cards" />
        ) : card ? (
          <>
            <div className="review-progress">
              <span>{done} of {total} reviewed</span>
              <span className="review-progress-bar" aria-hidden="true">
                <span style={{ transform: `scaleX(${total ? done / total : 0})` }} />
              </span>
            </div>
            <AnimatePresence mode="wait" initial={false}>
              <motion.article
                key={card.id + (revealed ? "-b" : "-f") + done}
                className="flashcard"
                style={{ ["--track" as string]: accent(card.track_id) }}
                initial={reduce ? false : { opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={reduce ? undefined : { opacity: 0 }}
                transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
              >
                <div className="flashcard-meta">
                  {trackName(card.track_id)}
                  {card.lesson_title ? ` · ${card.lesson_title}` : ""}
                  {card.memory.reps === 0 ? " · new" : ""}
                </div>
                <div className="flashcard-front prose">
                  <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{card.front}</Markdown>
                </div>
                {revealed && (
                  <div className="flashcard-back prose">
                    <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{card.back}</Markdown>
                  </div>
                )}
              </motion.article>
            </AnimatePresence>
            {!revealed ? (
              <div className="row review-actions">
                <button className="btn btn-primary" onClick={() => setRevealed(true)}>
                  Show answer <kbd>Space</kbd>
                </button>
              </div>
            ) : (
              <div className="ratings" role="group" aria-label="How well did you remember?">
                {RATINGS.map((r, i) => (
                  <button key={r.n} className="rating" data-rating={r.n} onClick={() => rate(r.n)} title={r.hint}>
                    <span className="rating-label">{r.label}</span>
                    <span className="rating-next">{card.preview[i]}</span>
                    <kbd>{r.n}</kbd>
                  </button>
                ))}
              </div>
            )}
          </>
        ) : (
          <div className="review-empty">
            <p className="review-empty-title">{done > 0 ? `Done. ${done} card${done === 1 ? "" : "s"} reviewed.` : "Nothing to review right now."}</p>
            <p className="muted">
              {summary?.next_due ? `The next card is due ${relative(summary.next_due)}.` : "Finish a lesson, or make cards from one below."}
              {summary && summary.total > 0 ? ` You have ${summary.total} cards in total.` : ""}
            </p>
          </div>
        )}
      </section>

      <MakeCards tracks={tracks} onMade={load} />
    </div>
  );
}

function MakeCards({ tracks, onMade }: { tracks: TrackSummary[]; onMade: () => Promise<void> }) {
  const [lessons, setLessons] = useState<LessonCards[] | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  const load = useCallback(() => api.lessonsForCards().then(setLessons), []);
  useEffect(() => {
    load();
  }, [load]);

  if (!lessons || lessons.length === 0) return null;
  const trackName = (id: string) => tracks.find((t) => t.id === id)?.name ?? id;

  return (
    <section className="make-cards" aria-labelledby="make-cards-h">
      <h2 id="make-cards-h">Make cards from a lesson</h2>
      <p className="muted">Cards are made automatically when you mark a lesson complete. Make more any time.</p>
      <ul>
        {lessons.map((l) => (
          <li key={l.lesson_id}>
            <span>
              {l.title}
              <span className="faint"> · {trackName(l.track_id)} · {l.cards} card{l.cards === 1 ? "" : "s"}</span>
            </span>
            <button
              className="btn btn-quiet"
              disabled={busy !== null}
              onClick={async () => {
                setBusy(l.lesson_id);
                setMsg(null);
                try {
                  const n = await api.makeCards(l.lesson_id);
                  setMsg(n ? `Added ${n} card${n === 1 ? "" : "s"} from "${l.title}".` : `No new cards: "${l.title}" is already covered.`);
                  await Promise.all([load(), onMade()]);
                } catch (e) {
                  setMsg(String(e));
                } finally {
                  setBusy(null);
                }
              }}
            >
              {busy === l.lesson_id ? <span className="spinner" /> : null} {l.cards ? "Make more" : "Make cards"}
            </button>
          </li>
        ))}
      </ul>
      {msg && <p className="faint" role="status">{msg}</p>}
    </section>
  );
}
