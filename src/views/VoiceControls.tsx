import { Icon } from "../icons";
import { speaker, useRecorder, useSpeaker } from "../voice";

/** Click to record, click again to stop; the transcript is handed to `onText`. */
export function MicButton({ onText, disabled, big }: { onText: (t: string) => void; disabled?: boolean; big?: boolean }) {
  const r = useRecorder(onText);
  const recording = r.status === "recording";
  const busy = r.status === "transcribing";
  const mins = Math.floor(r.seconds / 60);
  const secs = String(r.seconds % 60).padStart(2, "0");
  return (
    <span className="mic-wrap">
      <button
        type="button"
        className={big ? "mic mic-big" : "mic"}
        data-state={r.status}
        disabled={disabled || busy}
        aria-pressed={recording}
        aria-label={recording ? "Stop recording" : busy ? "Transcribing" : "Answer by voice"}
        title={recording ? "Stop and transcribe" : "Answer by voice"}
        onClick={() => (recording ? r.stop() : r.start())}
        style={{ ["--level" as string]: r.level.toFixed(2) }}
      >
        {busy ? <span className="spinner" /> : <Icon name={recording ? "stop" : "mic"} size={big ? 22 : 16} />}
      </button>
      {recording && (
        <span className="mic-status" role="status">
          <span className="rec-dot" aria-hidden="true" /> {mins}:{secs}
        </span>
      )}
      {busy && <span className="mic-status" role="status">Transcribing…</span>}
      {r.error && (
        <span className="mic-error" role="alert">
          {r.error}{" "}
          <button type="button" className="link-btn" onClick={r.clearError}>Dismiss</button>
        </span>
      )}
    </span>
  );
}

/** Read a piece of text aloud with Kokoro; shows Stop while it's playing. */
export function ListenButton({ text, id }: { text: string; id: string }) {
  const s = useSpeaker();
  const mine = s.key === id;
  const active = mine && s.status !== "idle";
  return (
    <button
      type="button"
      className="btn btn-quiet listen-btn"
      aria-pressed={active}
      onClick={() => (active ? speaker.stop() : speaker.speak(text, id))}
    >
      {mine && s.status === "loading" ? <span className="spinner" /> : <Icon name={active ? "stop" : "speaker"} size={14} />}
      {active ? (s.status === "loading" ? "Preparing voice…" : "Stop") : "Listen"}
    </button>
  );
}

export function SpeakerError() {
  const s = useSpeaker();
  if (!s.error) return null;
  return <div className="error-note" role="alert">Voice: {s.error}</div>;
}
