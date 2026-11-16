import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { api, Lesson, Occurrence, ReminderSettings, Repeat, TrackSummary } from "../api";
import { Icon } from "../icons";

type Props = { tracks: TrackSummary[]; onChanged: () => void };

const pad = (n: number) => String(n).padStart(2, "0");
const dateInput = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const rangeFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

function startOfWeek(d: Date) {
  const x = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const wd = (x.getDay() + 6) % 7; // Monday = 0
  x.setDate(x.getDate() - wd);
  return x;
}

const REPEATS: { value: Repeat; label: string }[] = [
  { value: "none", label: "Once" },
  { value: "daily", label: "Every day" },
  { value: "weekdays", label: "Weekdays" },
  { value: "weekly", label: "Every week" },
];

export default function Schedule({ tracks, onChanged }: Props) {
  const [weekStart, setWeekStart] = useState(() => startOfWeek(new Date()));
  const [agenda, setAgenda] = useState<Occurrence[]>([]);
  const [selected, setSelected] = useState<Occurrence | null>(null);
  const [error, setError] = useState<string | null>(null);

  const days = useMemo(
    () => Array.from({ length: 7 }, (_, i) => new Date(weekStart.getFullYear(), weekStart.getMonth(), weekStart.getDate() + i)),
    [weekStart],
  );

  const load = useCallback(async () => {
    const end = new Date(weekStart);
    end.setDate(end.getDate() + 7);
    setAgenda(await api.getAgenda(weekStart.toISOString(), end.toISOString()));
  }, [weekStart]);

  useEffect(() => {
    load().catch((e) => setError(String(e)));
  }, [load]);

  const shiftWeek = (n: number) => {
    const d = new Date(weekStart);
    d.setDate(d.getDate() + 7 * n);
    setWeekStart(d);
    setSelected(null);
  };

  const trackOf = (id: string) => tracks.find((t) => t.id === id);
  const now = new Date();
  const weekLabel = `${rangeFmt.format(days[0])} – ${rangeFmt.format(days[6])}`;

  return (
    <div className="page schedule">
      <h1 className="page-title">Schedule</h1>
      <p className="page-lede">
        Plan when you'll study. CPrep sends a Windows notification before each session, even when its window is closed.
      </p>
      {error && <div className="error-note" role="alert">{error}</div>}

      <div className="week-bar">
        <button className="icon-btn" aria-label="Previous week" onClick={() => shiftWeek(-1)}>
          <Icon name="back" size={16} />
        </button>
        <span className="week-label">{weekLabel}</span>
        <button className="icon-btn" aria-label="Next week" onClick={() => shiftWeek(1)}>
          <Icon name="chevron" size={16} />
        </button>
        <button className="btn btn-quiet" onClick={() => setWeekStart(startOfWeek(new Date()))}>This week</button>
      </div>

      <div className="week" role="list" aria-label={`Week of ${weekLabel}`}>
        {days.map((d) => {
          const items = agenda.filter((o) => sameDay(new Date(o.start), d));
          const isToday = sameDay(d, now);
          return (
            <section key={d.toISOString()} className="day" data-today={isToday} role="listitem">
              <header className="day-head">
                <span className="day-name">{dayFmt.format(d)}</span>
                <span className="day-num">{d.getDate()}</span>
              </header>
              {items.length === 0 && <div className="day-empty" aria-hidden="true" />}
              {items.map((o) => {
                const t = trackOf(o.session.track_id);
                const start = new Date(o.start);
                const past = new Date(o.end) < now;
                const active = selected?.start === o.start && selected.session.id === o.session.id;
                return (
                  <button
                    key={o.session.id + o.start}
                    className="slot"
                    data-past={past}
                    aria-pressed={active}
                    style={{ ["--track" as string]: t?.accent ?? "#97a39c" }}
                    onClick={() => setSelected(active ? null : o)}
                  >
                    <span className="slot-time">{timeFmt.format(start)}</span>
                    <span className="slot-title">{o.session.lesson_title ?? o.session.title}</span>
                    <span className="slot-meta">{o.session.duration_min} min</span>
                  </button>
                );
              })}
            </section>
          );
        })}
      </div>

      {selected && (
        <SessionDetail
          occ={selected}
          trackName={trackOf(selected.session.track_id)?.name ?? ""}
          onDelete={async () => {
            const ok = await api.confirm(
              selected.session.repeat === "none"
                ? "Delete this session?"
                : "Delete this repeating session? All of its future times are removed.",
              "Delete",
            );
            if (!ok) return;
            await api.deleteSession(selected.session.id);
            setSelected(null);
            await load();
            onChanged();
          }}
        />
      )}

      <div className="schedule-cols">
        <PlanForm tracks={tracks.filter((t) => t.status === "active")} onAdded={async () => { await load(); onChanged(); }} />
        <Reminders />
      </div>
    </div>
  );
}

function SessionDetail({ occ, trackName, onDelete }: { occ: Occurrence; trackName: string; onDelete: () => void }) {
  const repeat = REPEATS.find((r) => r.value === occ.session.repeat)?.label ?? "";
  const start = new Date(occ.start);
  return (
    <div className="session-detail" role="region" aria-label="Session details">
      <div>
        <div className="faint">{trackName}</div>
        <div className="session-detail-title">{occ.session.lesson_title ?? occ.session.title}</div>
        <div className="muted">
          {new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" }).format(start)}, {timeFmt.format(start)} ·{" "}
          {occ.session.duration_min} min · {repeat}
        </div>
      </div>
      <button className="btn btn-quiet" onClick={onDelete}>Delete session</button>
    </div>
  );
}

function PlanForm({ tracks, onAdded }: { tracks: TrackSummary[]; onAdded: () => Promise<void> }) {
  const soon = new Date(Date.now() + 60 * 60_000);
  const [trackId, setTrackId] = useState(tracks[0]?.id ?? "backend");
  const [lessons, setLessons] = useState<Lesson[]>([]);
  const [lessonId, setLessonId] = useState<number | null>(null);
  const [title, setTitle] = useState("");
  const [date, setDate] = useState(dateInput(soon));
  const [time, setTime] = useState(`${pad(soon.getHours())}:00`);
  const [duration, setDuration] = useState(45);
  const [repeat, setRepeat] = useState<Repeat>("none");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLessonId(null);
    api.getTrack(trackId).then((d) => {
      const ls = d.curriculum?.lessons ?? [];
      setLessons(ls);
      const next = ls.find((l) => l.status === "in_progress") ?? ls.find((l) => l.status === "todo");
      setLessonId(next?.id ?? null);
    });
  }, [trackId]);

  const track = tracks.find((t) => t.id === trackId);
  const lesson = lessons.find((l) => l.id === lessonId);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    setMsg(null);
    try {
      await api.addSession({
        track_id: trackId,
        lesson_id: lessonId,
        title: title.trim() || lesson?.title || track?.name || "Study session",
        starts_at: `${date}T${time}`,
        duration_min: duration,
        repeat,
      });
      setMsg("Session added. You'll get reminders before it starts.");
      setTitle("");
      await onAdded();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="plan" onSubmit={submit}>
      <h2>Plan a session</h2>
      <label className="field">
        <span>Track</span>
        <select value={trackId} onChange={(e) => setTrackId(e.target.value)}>
          {tracks.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
        </select>
      </label>
      <label className="field">
        <span>Lesson</span>
        <select value={lessonId ?? ""} onChange={(e) => setLessonId(e.target.value ? Number(e.target.value) : null)}>
          <option value="">{lessons.length ? "No specific lesson" : "No course yet for this track"}</option>
          {lessons.map((l) => (
            <option key={l.id} value={l.id}>
              {l.ord + 1}. {l.title}{l.status === "done" ? " (done)" : ""}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Name <em className="faint">(optional)</em></span>
        <input
          className="text-input"
          name="session-name"
          autoComplete="off"
          value={title}
          placeholder={lesson?.title ?? track?.name ?? "e.g. Subnetting practice…"}
          onChange={(e) => setTitle(e.target.value)}
        />
      </label>
      <div className="field-row">
        <label className="field">
          <span>Date</span>
          <input className="text-input" type="date" required value={date} onChange={(e) => setDate(e.target.value)} />
        </label>
        <label className="field">
          <span>Start</span>
          <input className="text-input" type="time" required value={time} onChange={(e) => setTime(e.target.value)} />
        </label>
        <label className="field">
          <span>Minutes</span>
          <input
            className="text-input"
            type="number"
            inputMode="numeric"
            min={5}
            max={480}
            step={5}
            required
            value={duration}
            onChange={(e) => setDuration(Number(e.target.value))}
          />
        </label>
      </div>
      <fieldset className="field">
        <legend>Repeat</legend>
        <div className="chips">
          {REPEATS.map((r) => (
            <button type="button" key={r.value} className="chip" aria-pressed={repeat === r.value} onClick={() => setRepeat(r.value)}>
              {r.label}
            </button>
          ))}
        </div>
      </fieldset>
      <div className="row">
        <button className="btn btn-primary" type="submit" disabled={busy}>
          {busy && <span className="spinner" />} Add session
        </button>
      </div>
      {msg && <p className="faint" role="status">{msg}</p>}
      {error && <div className="error-note" role="alert">{error}</div>}
    </form>
  );
}

const OFFSETS = [
  { value: 60, label: "1 hour before" },
  { value: 30, label: "30 min before" },
  { value: 15, label: "15 min before" },
  { value: 5, label: "5 min before" },
  { value: 0, label: "When it starts" },
];

function Reminders() {
  const [s, setS] = useState<ReminderSettings | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getReminders().then(setS).catch((e) => setError(String(e)));
  }, []);

  const save = async (next: ReminderSettings) => {
    setS(next);
    setError(null);
    try {
      await api.saveReminders(next);
      setStatus("Saved");
    } catch (e) {
      setError(String(e));
    }
  };

  if (!s) return <section className="reminders"><span className="spinner" /></section>;
  const toggleOffset = (v: number) =>
    save({ ...s, offsets: s.offsets.includes(v) ? s.offsets.filter((x) => x !== v) : [...s.offsets, v] });

  return (
    <section className="reminders" aria-labelledby="reminders-h">
      <h2 id="reminders-h">Reminders</h2>
      <div className="check-list" role="group" aria-label="Remind me">
        {OFFSETS.map((o) => (
          <label key={o.value} className="check">
            <input type="checkbox" checked={s.offsets.includes(o.value)} onChange={() => toggleOffset(o.value)} />
            {o.label}
          </label>
        ))}
      </div>
      <label className="check">
        <input
          type="checkbox"
          checked={s.review_time !== null}
          onChange={(e) => save({ ...s, review_time: e.target.checked ? "19:00" : null })}
        />
        Daily nudge when review cards are due, at
        <input
          className="text-input time-inline"
          type="time"
          aria-label="Review reminder time"
          disabled={s.review_time === null}
          value={s.review_time ?? "19:00"}
          onChange={(e) => save({ ...s, review_time: e.target.value })}
        />
      </label>
      <label className="check">
        <input type="checkbox" checked={s.keep_in_tray} onChange={(e) => save({ ...s, keep_in_tray: e.target.checked })} />
        Keep running in the tray when I close the window
      </label>
      <label className="check">
        <input type="checkbox" checked={s.autostart} onChange={(e) => save({ ...s, autostart: e.target.checked })} />
        Start CPrep when I sign in to Windows (opens quietly in the tray)
      </label>
      {!s.keep_in_tray && (
        <p className="faint">Reminders only arrive while CPrep is running. With the tray off, closing the window stops them.</p>
      )}
      <div className="row">
        <button className="btn" onClick={() => api.testNotification().then(() => setStatus("Test notification sent")).catch((e) => setError(String(e)))}>
          Send a test notification
        </button>
        {status && <span className="faint" role="status">{status}</span>}
      </div>
      {error && <div className="error-note" role="alert">{error}</div>}
    </section>
  );
}
