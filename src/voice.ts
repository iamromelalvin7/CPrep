// Speaking (Kokoro, sentence by sentence) and listening (record → Groq Whisper).
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { api, inTauri } from "./api";

// ---------- Preferences (per device) ----------

export type VoicePrefs = { voice: string; speed: number; autoRead: boolean };
const PREFS_KEY = "cprep.voice";
const DEFAULT_PREFS: VoicePrefs = { voice: "af_heart", speed: 1, autoRead: false };

export function loadPrefs(): VoicePrefs {
  try {
    return { ...DEFAULT_PREFS, ...JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") };
  } catch {
    return DEFAULT_PREFS;
  }
}

export function savePrefs(p: VoicePrefs) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable */
  }
}

// ---------- Text preparation ----------

/** Markdown → something pleasant to hear. Code is mentioned, not read out. */
export function toSpeech(md: string): string {
  return md
    .replace(/```mermaid[\s\S]*?```/g, " (There's a diagram on screen.) ")
    .replace(/```[\s\S]*?```/g, " (There's a code example on screen.) ")
    .replace(/\[\[\s*topic\s*:\s*\d+\s*\]\]/g, "")
    .replace(/\[S\d+\]/g, "")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^\s*#{1,6}\s*/gm, "")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/^\s*\d+\.\s+/gm, "")
    .replace(/[*_~]{1,3}([^*_~]+)[*_~]{1,3}/g, "$1")
    .replace(/<\/?[^>]+>/g, " ")
    .replace(/\p{Extended_Pictographic}/gu, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** Split into sentence-sized pieces: short ones are merged, very long ones split at commas. */
export function splitSentences(text: string): string[] {
  const raw = text.match(/[^.!?]+(?:[.!?]+["')\]]*|$)/g)?.map((s) => s.trim()).filter(Boolean) ?? [];
  const out: string[] = [];
  for (const s of raw) {
    if (out.length && (out[out.length - 1].length < 40 || s.length < 12)) out[out.length - 1] += " " + s;
    else out.push(s);
  }
  return out.flatMap((s) => {
    if (s.length <= 320) return [s];
    const parts: string[] = [];
    let cur = "";
    for (const piece of s.split(/(?<=,)\s+/)) {
      if (cur && cur.length + piece.length > 280) {
        parts.push(cur);
        cur = piece;
      } else cur = cur ? `${cur} ${piece}` : piece;
    }
    if (cur) parts.push(cur);
    return parts;
  });
}

// ---------- Speaker (one at a time, app-wide) ----------

type SpeakState = { status: "idle" | "loading" | "speaking"; key: string | null; error: string | null };
let state: SpeakState = { status: "idle", key: null, error: null };
const listeners = new Set<() => void>();
const setState = (s: Partial<SpeakState>) => {
  state = { ...state, ...s };
  listeners.forEach((l) => l());
};
let generation = 0;
let audio: HTMLAudioElement | null = null;

function b64ToUrl(b64: string) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
}

function play(url: string, gen: number) {
  return new Promise<void>((resolve) => {
    if (gen !== generation) return resolve();
    audio = new Audio(url);
    audio.onended = audio.onerror = () => {
      URL.revokeObjectURL(url);
      resolve();
    };
    audio.play().catch(() => resolve());
  });
}

export const speaker = {
  /** Read text aloud. `key` identifies what's playing (e.g. a message id) so its button can show Stop. */
  async speak(markdown: string, key: string) {
    const gen = ++generation;
    audio?.pause();
    const prefs = loadPrefs();
    const sentences = splitSentences(toSpeech(markdown));
    if (!sentences.length) return;
    setState({ status: "loading", key, error: null });
    if (!inTauri) {
      // Browser preview: no Kokoro, so pretend briefly and finish.
      setState({ status: "speaking" });
      await new Promise((r) => setTimeout(r, 1200));
      if (gen === generation) setState({ status: "idle", key: null });
      return;
    }
    const fetchOne = (s: string) => api.speak(s, prefs.voice, prefs.speed).then((r) => b64ToUrl(r.wav_b64));
    try {
      // Synthesise the next sentence while the current one plays.
      let next = fetchOne(sentences[0]);
      for (let i = 0; i < sentences.length; i++) {
        const url = await next;
        if (gen !== generation) return URL.revokeObjectURL(url);
        if (i + 1 < sentences.length) next = fetchOne(sentences[i + 1]);
        setState({ status: "speaking" });
        await play(url, gen);
      }
      if (gen === generation) setState({ status: "idle", key: null });
    } catch (e) {
      if (gen === generation) setState({ status: "idle", key: null, error: String(e) });
    }
  },
  stop() {
    generation++;
    audio?.pause();
    setState({ status: "idle", key: null });
  },
  subscribe(l: () => void) {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  get: () => state,
};

export function useSpeaker() {
  return useSyncExternalStore(speaker.subscribe, speaker.get);
}

// ---------- Recorder ----------

async function blobToB64(b: Blob) {
  const buf = new Uint8Array(await b.arrayBuffer());
  let bin = "";
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return btoa(bin);
}

/** Record from the microphone and transcribe with Groq Whisper. */
export function useRecorder(onText: (text: string) => void) {
  const [status, setStatus] = useState<"idle" | "recording" | "transcribing">("idle");
  const [seconds, setSeconds] = useState(0);
  const [level, setLevel] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const rec = useRef<MediaRecorder | null>(null);
  const chunks = useRef<Blob[]>([]);
  const timer = useRef<ReturnType<typeof setInterval> | undefined>(undefined);
  const raf = useRef<number>(0);
  const onTextRef = useRef(onText);
  onTextRef.current = onText;

  const cleanup = useCallback(() => {
    clearInterval(timer.current);
    cancelAnimationFrame(raf.current);
    rec.current?.stream.getTracks().forEach((t) => t.stop());
    setLevel(0);
  }, []);

  useEffect(() => cleanup, [cleanup]);

  const start = useCallback(async () => {
    setError(null);
    speaker.stop();
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
      const mime = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";
      const r = new MediaRecorder(stream, { mimeType: mime });
      chunks.current = [];
      r.ondataavailable = (e) => e.data.size && chunks.current.push(e.data);
      r.onstop = async () => {
        cleanup();
        const blob = new Blob(chunks.current, { type: mime });
        if (blob.size < 2000) {
          setStatus("idle");
          setError("Didn't catch anything. Check your microphone isn't muted.");
          return;
        }
        setStatus("transcribing");
        try {
          const text = await api.transcribe(await blobToB64(blob), mime);
          if (text) onTextRef.current(text);
          else setError("Didn't catch any words. Try again a little closer to the mic.");
        } catch (e) {
          setError(String(e));
        } finally {
          setStatus("idle");
        }
      };
      // Simple input level meter so you can see the mic is hearing you.
      const ctx = new AudioContext();
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      ctx.createMediaStreamSource(stream).connect(analyser);
      const data = new Uint8Array(analyser.fftSize);
      const tick = () => {
        analyser.getByteTimeDomainData(data);
        let peak = 0;
        for (const v of data) peak = Math.max(peak, Math.abs(v - 128));
        setLevel(Math.min(1, peak / 64));
        raf.current = requestAnimationFrame(tick);
      };
      tick();
      rec.current = r;
      r.start(500);
      setSeconds(0);
      timer.current = setInterval(() => setSeconds((s) => s + 1), 1000);
      setStatus("recording");
    } catch (e) {
      cleanup();
      setStatus("idle");
      setError(
        String(e).includes("NotAllowed")
          ? "Microphone access was blocked. Allow it when Windows or CPrep asks, or in Windows Settings → Privacy → Microphone."
          : `Couldn't start the microphone: ${e}`,
      );
    }
  }, [cleanup]);

  const stop = useCallback(() => {
    if (rec.current && rec.current.state !== "inactive") rec.current.stop();
  }, []);

  return { status, seconds, level, error, start, stop, clearError: () => setError(null) };
}
