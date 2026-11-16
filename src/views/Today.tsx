import { useEffect, useMemo, useRef, useState } from "react";
import { api, Dashboard, TrackSummary } from "../api";

type Props = {
  tracks: TrackSummary[];
  onOpenLesson: (trackId: string, lessonId: number) => void;
  onGo: (kind: "review" | "schedule" | "interview") => void;
};

// Sequential ramp: one hue (the lamp accent) mixed into the dark surface, darkest = least activity.
const HEAT = ["#26302d", "#534f33", "#7d6e3d", "#b1954b", "#f0c35a"];
const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });
const timeFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit" });
const money = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", minimumFractionDigits: 2 });
const pct = new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 0 });
const KIND_LABEL: Record<string, string> = { behavioral: "Behavioral", technical: "Technical", dsa: "Coding", system_design: "System design" };

function greeting() {
  const h = new Date().getHours();
  return h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : "Good evening";
}

type Tip = { x: number; y: number; text: string } | null;

function useTip() {
  const [tip, setTip] = useState<Tip>(null);
  const host = useRef<HTMLDivElement>(null);
  const show = (e: { clientX: number; clientY: number }, text: string) => {
    const r = host.current?.getBoundingClientRect();
    if (r) setTip({ x: e.clientX - r.left, y: e.clientY - r.top, text });
  };
  const showAt = (el: Element, text: string) => {
    const r = host.current?.getBoundingClientRect();
    const b = el.getBoundingClientRect();
    if (r) setTip({ x: b.left - r.left + b.width / 2, y: b.top - r.top, text });
  };
  return { tip, host, show, showAt, hide: () => setTip(null) };
}

export default function Today({ tracks, onOpenLesson, onGo }: Props) {
  const [d, setD] = useState<Dashboard | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getDashboard().then(setD).catch((e) => setError(String(e)));
  }, []);

  if (error) return <div className="page"><div className="error-note" role="alert">{error}</div></div>;
  if (!d) return <div className="page"><span className="spinner" aria-label="Loading" /></div>;
  const nextTrack = d.next_lesson ? tracks.find((t) => t.id === d.next_lesson!.track_id) : null;
  const todayCount = d.activity[d.activity.length - 1]?.count ?? 0;

  return (
    <div className="page today">
      <h1 className="page-title">{greeting()}.</h1>
      <p className="page-lede">
        {d.streak > 0
          ? `You've studied ${d.streak} day${d.streak === 1 ? "" : "s"} in a row${todayCount ? ", including today" : ". Study today to keep it going"}.`
          : "Pick up where you left off. A short session today starts a streak."}
      </p>

      <div className="today-actions">
        {d.next_lesson && (
          <button className="btn btn-primary" onClick={() => onOpenLesson(d.next_lesson!.track_id, d.next_lesson!.lesson_id)}>
            {d.next_lesson.status === "in_progress" ? "Continue" : "Start"}: {d.next_lesson.title}
          </button>
        )}
        {d.review.due_now > 0 && (
          <button className="btn" onClick={() => onGo("review")}>
            Review {d.review.due_now} card{d.review.due_now === 1 ? "" : "s"}
          </button>
        )}
        {nextTrack && <span className="faint">{nextTrack.name}</span>}
      </div>

      <section className="today-section" aria-labelledby="act-h">
        <div className="section-head">
          <h2 id="act-h">Activity</h2>
          <span className="faint">{d.active_days_30} active day{d.active_days_30 === 1 ? "" : "s"} in the last 30</span>
        </div>
        <Heatmap activity={d.activity} />
      </section>

      <div className="today-grid">
        <section className="today-section" aria-labelledby="prog-h">
          <div className="section-head">
            <h2 id="prog-h">Courses</h2>
          </div>
          {d.tracks.length === 0 ? (
            <p className="muted">No courses yet. Open a track, take the quiz and design your course.</p>
          ) : (
            <ul className="progress-list">
              {d.tracks.map((t) => (
                <li key={t.id}>
                  <div className="progress-row">
                    <span className="progress-name">
                      <span className="track-dot" style={{ background: t.accent }} aria-hidden="true" /> {t.name}
                    </span>
                    <span className="progress-num">{t.done} of {t.total} lessons</span>
                  </div>
                  <div className="progress-track" role="progressbar" aria-label={`${t.name} progress`} aria-valuemin={0} aria-valuemax={t.total} aria-valuenow={t.done}>
                    <span style={{ transform: `scaleX(${t.total ? t.done / t.total : 0})`, background: t.accent }} />
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="today-section" aria-labelledby="up-h">
          <div className="section-head">
            <h2 id="up-h">Coming up</h2>
            <button className="btn btn-quiet" onClick={() => onGo("schedule")}>Schedule</button>
          </div>
          {d.upcoming.length === 0 ? (
            <p className="muted">Nothing planned this week. Plan a session and CPrep will remind you.</p>
          ) : (
            <ul className="upcoming">
              {d.upcoming.map((o) => {
                const t = tracks.find((x) => x.id === o.session.track_id);
                return (
                  <li key={o.session.id + o.start} style={{ ["--track" as string]: t?.accent ?? "#97a39c" }}>
                    <span className="up-time">{timeFmt.format(new Date(o.start))}</span>
                    <span className="up-title">{o.session.lesson_title ?? o.session.title}</span>
                    <span className="faint">{o.session.duration_min} min</span>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>

      <section className="today-section" aria-labelledby="stats-h">
        <div className="section-head">
          <h2 id="stats-h">Practice</h2>
        </div>
        <dl className="stat-row">
          <div>
            <dt>Review cards</dt>
            <dd>{d.review.total}</dd>
            <span className="faint">{d.review.reviewed_today} reviewed today</span>
          </div>
          <div>
            <dt>Remembered, last 30 days</dt>
            <dd>{d.retention_30d === null ? "—" : pct.format(d.retention_30d)}</dd>
            <span className="faint">aim for about 90%</span>
          </div>
          <div>
            <dt>Exercises solved</dt>
            <dd>{d.exercises_solved}<span className="stat-of"> / {d.exercises_tried}</span></dd>
            <span className="faint">all tests passing</span>
          </div>
          <div>
            <dt>Mock interviews</dt>
            <dd>{d.interviews.length}</dd>
            <button className="link-btn" onClick={() => onGo("interview")}>Start one</button>
          </div>
        </dl>
        {d.interviews.length >= 2 && <InterviewTrend points={d.interviews} />}
      </section>

      <section className="today-section" aria-labelledby="spend-h">
        <div className="section-head">
          <h2 id="spend-h">AI spend</h2>
          <span className="faint">estimated from token counts</span>
        </div>
        <Spend s={d.spend} />
      </section>
    </div>
  );
}

function Heatmap({ activity }: { activity: Dashboard["activity"] }) {
  const { tip, host, showAt, hide } = useTip();
  const max = Math.max(1, ...activity.map((a) => a.count));
  const level = (n: number) => (n === 0 ? 0 : Math.min(4, 1 + Math.floor((n / max) * 3.999)));
  // Columns are weeks (Monday first); pad the start so the first column begins on a Monday.
  const first = new Date(activity[0].date + "T00:00");
  const pad = (first.getDay() + 6) % 7;
  const cells = [...Array(pad).fill(null), ...activity];
  const weeks: (typeof activity[number] | null)[][] = [];
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
  const total = activity.reduce((s, a) => s + a.count, 0);

  return (
    <div className="heatmap-wrap" ref={host} onMouseLeave={hide}>
      <div className="heatmap" role="img" aria-label={`Activity over the last 12 weeks: ${total} actions on ${activity.filter((a) => a.count).length} days`}>
        {weeks.map((w, wi) => (
          <div className="heat-col" key={wi}>
            {w.map((c, ci) =>
              c ? (
                <span
                  key={ci}
                  className="heat-cell"
                  style={{ background: HEAT[level(c.count)] }}
                  onMouseEnter={(e) => showAt(e.currentTarget, `${dayFmt.format(new Date(c.date + "T00:00"))}: ${c.count} action${c.count === 1 ? "" : "s"}`)}
                />
              ) : (
                <span key={ci} className="heat-cell heat-empty" aria-hidden="true" />
              ),
            )}
          </div>
        ))}
      </div>
      <div className="heat-legend" aria-hidden="true">
        <span>Less</span>
        {HEAT.map((c) => <span key={c} className="heat-cell" style={{ background: c }} />)}
        <span>More</span>
      </div>
      <p className="faint heat-note">Counts lesson replies, card reviews, code submissions and interview answers.</p>
      {tip && <div className="chart-tip" style={{ left: tip.x, top: tip.y }} role="status">{tip.text}</div>}
    </div>
  );
}

function InterviewTrend({ points }: { points: Dashboard["interviews"] }) {
  const { tip, host, showAt, hide } = useTip();
  const W = 560, H = 140, L = 28, R = 12, T = 12, B = 26;
  const x = (i: number) => L + (points.length === 1 ? 0 : (i / (points.length - 1)) * (W - L - R));
  const y = (v: number) => T + ((5 - v) / 4) * (H - T - B);
  const path = useMemo(() => points.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(p.overall).toFixed(1)}`).join(" "), [points]);
  const short = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
  return (
    <figure className="trend" ref={host} onMouseLeave={hide}>
      <figcaption className="faint">Interview score (out of 5), oldest to newest</figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label={`Interview scores: ${points.map((p) => p.overall).join(", ")}`}>
        {[1, 2, 3, 4, 5].map((v) => (
          <g key={v}>
            <line x1={L} x2={W - R} y1={y(v)} y2={y(v)} className="grid-line" />
            <text x={L - 8} y={y(v) + 4} className="axis-label" textAnchor="end">{v}</text>
          </g>
        ))}
        <path d={path} className="trend-line" />
        {points.map((p, i) => (
          <g key={p.id}>
            <circle cx={x(i)} cy={y(p.overall)} r={5} className="trend-dot" />
            <circle
              cx={x(i)}
              cy={y(p.overall)}
              r={14}
              className="hit"
              onMouseEnter={(e) => showAt(e.currentTarget, `${short.format(new Date(p.date + "T00:00"))} · ${KIND_LABEL[p.kind] ?? p.kind}: ${p.overall} of 5`)}
            />
            {(i === 0 || i === points.length - 1) && (
              <text x={x(i)} y={H - 6} className="axis-label" textAnchor={i === 0 ? "start" : "end"}>
                {short.format(new Date(p.date + "T00:00"))}
              </text>
            )}
          </g>
        ))}
      </svg>
      {tip && <div className="chart-tip" style={{ left: tip.x, top: tip.y }} role="status">{tip.text}</div>}
    </figure>
  );
}

function Spend({ s }: { s: Dashboard["spend"] }) {
  const used = Math.min(1, s.budget_usd ? s.claude_usd / s.budget_usd : 0);
  const left = Math.max(0, s.budget_usd - s.claude_usd);
  return (
    <div className="spend">
      <div className="spend-head">
        <span>
          <b>{money.format(s.claude_usd)}</b> of your {money.format(s.budget_usd)} Claude credit used
        </span>
        <span className="faint">{money.format(left)} left · {money.format(s.month_usd)} this month</span>
      </div>
      <div className="progress-track spend-track" role="progressbar" aria-label="Claude credit used" aria-valuemin={0} aria-valuemax={s.budget_usd} aria-valuenow={s.claude_usd}>
        <span style={{ transform: `scaleX(${used})` }} data-warn={used > 0.8} />
      </div>
      {s.by_model.length > 0 && (
        <table className="spend-table">
          <thead>
            <tr><th scope="col">Model</th><th scope="col">Requests</th><th scope="col">Cost</th></tr>
          </thead>
          <tbody>
            {s.by_model.map(([model, cost, n]) => (
              <tr key={model}>
                <td><code>{model}</code></td>
                <td>{n}</td>
                <td>{cost > 0 ? money.format(cost) : <span className="faint">free</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="faint">Change the budget with CLAUDE_BUDGET_USD in .env. Check your Anthropic console for the exact bill.</p>
    </div>
  );
}

