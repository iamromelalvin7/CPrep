import "../monaco";
import { useCallback, useEffect, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api, Debrief, Interview, InterviewKind, InterviewTurn, TrackSummary } from "../api";
import { Icon, MdIcon, remarkEmojiIcons } from "../icons";
import { loadPrefs, savePrefs, speaker } from "../voice";
import { ListenButton, MicButton, SpeakerError } from "./VoiceControls";

type Props = { tracks: TrackSummary[] };

const mdPlugins = [remarkGfm, remarkEmojiIcons];
const mdComponents = { i: MdIcon as never };
const START_MARK = "(The candidate has joined";

const KINDS: { kind: InterviewKind; title: string; about: string }[] = [
  { kind: "behavioral", title: "Behavioral", about: "Projects, teamwork, conflict and failure. Practise answering with the STAR structure." },
  { kind: "technical", title: "Technical questions", about: "Concepts and scenarios from one track, getting harder as you answer well." },
  { kind: "dsa", title: "Coding (DSA)", about: "One problem: clarify, explain your approach, code it, then complexity and edge cases." },
  { kind: "system_design", title: "System design", about: "Design a small system end to end: requirements, API, data, scaling and trade-offs." },
];
const kindTitle = (k: InterviewKind) => KINDS.find((x) => x.kind === k)?.title ?? k;
const dateFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

export default function InterviewRoom({ tracks }: Props) {
  const [activeId, setActiveId] = useState<number | null>(null);
  const [history, setHistory] = useState<Interview[]>([]);

  const loadHistory = useCallback(() => api.listInterviews().then(setHistory), []);
  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  if (activeId !== null)
    return (
      <Session
        id={activeId}
        tracks={tracks}
        onExit={() => {
          speaker.stop();
          setActiveId(null);
          loadHistory();
        }}
      />
    );
  return <Setup tracks={tracks} history={history} onStart={setActiveId} onOpen={setActiveId} />;
}

// ---------- Setup ----------

function Setup({ tracks, history, onStart, onOpen }: { tracks: TrackSummary[]; history: Interview[]; onStart: (id: number) => void; onOpen: (id: number) => void }) {
  const active = tracks.filter((t) => t.status === "active" && t.id !== "interview");
  const [kind, setKind] = useState<InterviewKind>("technical");
  const [trackId, setTrackId] = useState(active[0]?.id ?? "backend");
  const [minutes, setMinutes] = useState(20);
  const [voiceOn, setVoiceOn] = useState(() => loadPrefs().autoRead);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const start = async () => {
    setBusy(true);
    setError(null);
    savePrefs({ ...loadPrefs(), autoRead: voiceOn });
    if (voiceOn) api.voiceWarmUp().catch(() => {});
    try {
      onStart(await api.startInterview(kind, kind === "technical" ? trackId : kind === "dsa" ? "dsa" : kind === "system_design" ? "system-design" : null, minutes));
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="page interview-setup">
      <h1 className="page-title">Mock interview</h1>
      <p className="page-lede">
        An interviewer asks one question at a time and follows up. Answer out loud or by typing. At the end you get scores, what
        to fix, and a stronger version of your weakest answer.
      </p>

      <fieldset className="kind-list">
        <legend className="sr-only">Interview type</legend>
        {KINDS.map((k) => (
          <label key={k.kind} className="kind-option" data-selected={kind === k.kind}>
            <input type="radio" name="kind" value={k.kind} checked={kind === k.kind} onChange={() => setKind(k.kind)} />
            <span className="kind-title">{k.title}</span>
            <span className="kind-about">{k.about}</span>
          </label>
        ))}
      </fieldset>

      <div className="interview-options">
        {kind === "technical" && (
          <label className="field">
            <span>Track</span>
            <select value={trackId} onChange={(e) => setTrackId(e.target.value)}>
              {active.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
            </select>
          </label>
        )}
        <fieldset className="field">
          <legend>Length</legend>
          <div className="chips">
            {[10, 20, 30, 45].map((m) => (
              <button key={m} type="button" className="chip" aria-pressed={minutes === m} onClick={() => setMinutes(m)}>
                {m} min
              </button>
            ))}
          </div>
        </fieldset>
        <label className="check">
          <input type="checkbox" checked={voiceOn} onChange={(e) => setVoiceOn(e.target.checked)} />
          The interviewer speaks (Kokoro voice; the first start takes about 30 seconds)
        </label>
      </div>
      <div className="row">
        <button className="btn btn-primary" disabled={busy} onClick={start}>
          {busy && <span className="spinner" />} Start interview
        </button>
      </div>
      {error && <div className="error-note" role="alert">{error}</div>}

      {history.length > 0 && (
        <section className="interview-history" aria-labelledby="hist-h">
          <h2 id="hist-h">Past interviews</h2>
          <ul>
            {history.map((iv) => (
              <li key={iv.id}>
                <button className="history-row" onClick={() => onOpen(iv.id)}>
                  <span>{kindTitle(iv.kind)}{iv.track_id && iv.kind === "technical" ? ` · ${tracks.find((t) => t.id === iv.track_id)?.name ?? ""}` : ""}</span>
                  <span className="faint">{dateFmt.format(new Date(iv.started_at))} · {iv.minutes} min</span>
                  <span className="history-verdict">{iv.debrief ? `${iv.debrief.verdict} · ${iv.debrief.overall}/5` : iv.ended_at ? "Ended" : "Not finished"}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

// ---------- Session ----------

function useCountdown(startedAt: string | null, minutes: number) {
  const [left, setLeft] = useState(minutes * 60);
  useEffect(() => {
    if (!startedAt) return;
    const tick = () => setLeft(Math.max(0, Math.round(minutes * 60 - (Date.now() - new Date(startedAt).getTime()) / 1000)));
    tick();
    const t = setInterval(tick, 1000);
    return () => clearInterval(t);
  }, [startedAt, minutes]);
  return left;
}

function Session({ id, tracks, onExit }: { id: number; tracks: TrackSummary[]; onExit: () => void }) {
  const [iv, setIv] = useState<Interview | null>(null);
  const [turns, setTurns] = useState<InterviewTurn[]>([]);
  const [pending, setPending] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [code, setCode] = useState("");
  const [sentCode, setSentCode] = useState("");
  const [ending, setEnding] = useState(false);
  const [debrief, setDebrief] = useState<Debrief | null>(null);
  const [error, setError] = useState<string | null>(null);
  const started = useRef(false);
  const scroller = useRef<HTMLDivElement>(null);
  const left = useCountdown(iv && !iv.ended_at ? iv.started_at : null, iv?.minutes ?? 0);

  const load = useCallback(async () => {
    const v = await api.getInterview(id);
    setIv(v.interview);
    setTurns(v.turns);
    setDebrief(v.interview.debrief);
    return v;
  }, [id]);

  const send = useCallback(
    async (text: string, withCode: string | null) => {
      setError(null);
      setPending("");
      try {
        let reply = "";
        await api.interviewTurn(id, text, withCode, (e) => {
          if (e.type === "delta") {
            reply += e.text;
            setPending(reply);
          }
          if (e.type === "error") setError(e.message);
        });
        await load();
        if (reply && loadPrefs().autoRead) speaker.speak(reply, `iv-${id}-${Date.now()}`);
      } catch (e) {
        setError(String(e));
        if (text) setDraft(text);
      } finally {
        setPending(null);
      }
    },
    [id, load],
  );

  useEffect(() => {
    load().then((v) => {
      if (v.turns.length === 0 && !v.interview.ended_at && !started.current) {
        started.current = true;
        send("", null);
      }
    });
  }, [load, send]);

  useEffect(() => {
    if (scroller.current) scroller.current.scrollTop = scroller.current.scrollHeight;
  }, [turns.length, pending]);

  const submit = () => {
    const t = draft.trim();
    const c = iv?.kind === "dsa" && code.trim() && code !== sentCode ? code : null;
    if ((!t && !c) || pending !== null) return;
    setDraft("");
    if (c) setSentCode(c);
    send(t, c);
  };

  const end = async () => {
    setEnding(true);
    setError(null);
    speaker.stop();
    try {
      setDebrief(await api.endInterview(id));
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setEnding(false);
    }
  };

  if (!iv) return <div className="page"><span className="spinner" /></div>;
  const shown = turns.filter((t) => !t.content.startsWith(START_MARK));
  const mm = Math.floor(left / 60);
  const ss = String(left % 60).padStart(2, "0");
  const trackName = iv.track_id ? tracks.find((t) => t.id === iv.track_id)?.name : null;

  return (
    <div className="lesson interview" data-kind={iv.kind}>
      <header className="lesson-bar">
        <button className="btn btn-quiet" onClick={onExit}>
          <Icon name="back" size={16} /> Interviews
        </button>
        <div className="grow">
          <div className="lesson-crumbs">Mock interview{trackName && iv.kind === "technical" ? ` · ${trackName}` : ""}</div>
          <div className="lesson-title-sm">{kindTitle(iv.kind)}</div>
        </div>
        {!iv.ended_at && (
          <>
            <span className="countdown" data-low={left <= 120} aria-live="off" aria-label={`${mm} minutes ${ss} seconds left`}>
              {mm}:{ss}
            </span>
            <button className="btn" disabled={ending || pending !== null} onClick={end}>
              {ending ? <span className="spinner" /> : null} {ending ? "Scoring…" : "End and get feedback"}
            </button>
          </>
        )}
      </header>

      <div className="lesson-body" data-lab={iv.kind === "dsa" && !iv.ended_at} data-panel="practice">
        <div className="lesson-chat">
          <div className="lesson-scroll" ref={scroller}>
            {debrief && <DebriefView d={debrief} />}
            <section className={debrief ? "transcript transcript-done" : "transcript"} aria-label="Interview transcript">
              {debrief && <h2 className="transcript-h">Transcript</h2>}
              {shown.map((t) => (
                <div key={t.id} className={t.role === "assistant" ? "iv-turn iv-interviewer" : "iv-turn iv-you"}>
                  <div className="iv-who">{t.role === "assistant" ? "Interviewer" : "You"}</div>
                  {t.role === "assistant" ? (
                    <div className="prose"><Markdown remarkPlugins={mdPlugins} components={mdComponents}>{t.content}</Markdown></div>
                  ) : (
                    <div className="said">{t.content}</div>
                  )}
                  {t.code && <pre className="iv-code"><code>{t.code}</code></pre>}
                  {t.role === "assistant" && !debrief && <ListenButton text={t.content} id={`ivt-${t.id}`} />}
                </div>
              ))}
              {pending !== null && (
                <div className="iv-turn iv-interviewer">
                  <div className="iv-who">Interviewer</div>
                  <div className="prose">
                    <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{pending}</Markdown>
                    <span className="caret" aria-hidden="true" />
                  </div>
                </div>
              )}
            </section>
            {error && <div className="empty-lesson"><div className="error-note" role="alert">{error}</div></div>}
            <div className="empty-lesson"><SpeakerError /></div>
          </div>

          {!iv.ended_at && (
            <footer className="composer">
              <div className="composer-inner iv-composer">
                <MicButton big disabled={pending !== null} onText={(t) => setDraft((d) => (d ? `${d} ${t}` : t))} />
                <div className="iv-answer">
                  <textarea
                    aria-label="Your answer"
                    name="answer"
                    autoComplete="off"
                    value={draft}
                    rows={3}
                    placeholder="Speak with the mic, or type your answer… Enter sends, Shift+Enter adds a line"
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        submit();
                      }
                    }}
                  />
                  <div className="composer-row">
                    <span className="faint">
                      {iv.kind === "dsa" ? "Your code is sent with your answer whenever it has changed." : "Your recording is transcribed by Groq Whisper."}
                    </span>
                    <button className="btn btn-primary" disabled={pending !== null || (!draft.trim() && !(iv.kind === "dsa" && code.trim() && code !== sentCode))} onClick={submit}>
                      {pending !== null ? <span className="spinner" aria-label="Interviewer is replying" /> : "Send answer"}
                    </button>
                  </div>
                </div>
              </div>
            </footer>
          )}
        </div>

        {iv.kind === "dsa" && !iv.ended_at && (
          <aside className="lab" aria-label="Code editor">
            <header className="lab-head">
              <div className="lab-title"><Icon name="code" size={16} /> Your code</div>
              <span className="faint">Python</span>
            </header>
            <div className="lab-body">
              <div className="lab-editor iv-editor">
                <Editor
                  height="100%"
                  language="python"
                  theme="cprep-ink"
                  value={code}
                  onChange={(v) => setCode(v ?? "")}
                  options={{
                    minimap: { enabled: false },
                    fontSize: 13,
                    fontFamily: '"JetBrains Mono Variable", Consolas, monospace',
                    fontLigatures: false,
                    scrollBeyondLastLine: false,
                    tabSize: 4,
                    automaticLayout: true,
                    padding: { top: 12 },
                    lineNumbersMinChars: 3,
                    ariaLabel: "Code for the interviewer",
                  }}
                />
              </div>
            </div>
          </aside>
        )}
      </div>
    </div>
  );
}

function DebriefView({ d }: { d: Debrief }) {
  return (
    <section className="debrief" aria-label="Interview feedback">
      <div className="debrief-head">
        <div className="debrief-verdict" data-overall={d.overall}>{d.verdict}</div>
        <div className="faint">Overall {d.overall} of 5 · judged against an internship bar</div>
      </div>
      <p className="debrief-summary">{d.summary}</p>
      <ol className="skill-scores">
        {d.scores.map((s) => (
          <li key={s.skill}>
            <div className="skill-row">
              <span className="skill-name">{s.skill}</span>
              <span className="skill-meter" aria-label={`${s.score} out of 5`}>
                {[1, 2, 3, 4, 5].map((n) => <span key={n} data-on={n <= s.score} />)}
              </span>
            </div>
            <p className="faint">{s.note}</p>
          </li>
        ))}
      </ol>
      <div className="debrief-cols">
        <div>
          <h3>Went well</h3>
          <ul>{d.strengths.map((x, i) => <li key={i}>{x}</li>)}</ul>
        </div>
        <div>
          <h3>Work on next</h3>
          <ul>{d.improvements.map((x, i) => <li key={i}>{x}</li>)}</ul>
        </div>
      </div>
      <div className="better-answer">
        <h3>A stronger answer</h3>
        <p className="faint">{d.better_answer.question}</p>
        <div className="prose"><Markdown remarkPlugins={mdPlugins} components={mdComponents}>{d.better_answer.answer}</Markdown></div>
        <ListenButton text={d.better_answer.answer} id="better-answer" />
      </div>
    </section>
  );
}
