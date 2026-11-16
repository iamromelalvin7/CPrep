import { useState } from "react";
import { api, Settings } from "../api";

export default function SettingsView({ settings, onChange }: { settings: Settings | null; onChange: () => void }) {
  const [error, setError] = useState<string | null>(null);
  if (!settings) return <div className="page"><span className="spinner" /></div>;
  const s = settings;

  const setLive = async (live: boolean) => {
    setError(null);
    try {
      await api.setLive(live);
      onChange();
    } catch (e) {
      setError(String(e));
    }
  };

  const key = (has: boolean, name: string) =>
    has ? <span className="ok">Found</span> : <span className="faint">Not set. Add <span className="code-inline">{name}</span> to .env</span>;

  return (
    <div className="page">
      <h1 className="page-title">Settings</h1>
      <p className="page-lede">
        Test mode runs on free local models and costs nothing. Switch to live when you want Claude to teach.
      </p>
      {error && <div className="error-note" role="alert">{error}</div>}

      <dl className="settings-list">
        <div className="setting">
          <dt>Who teaches</dt>
          <dd>
            <div className="switch" role="group" aria-label="Teaching mode">
              <button aria-pressed={!s.live} onClick={() => setLive(false)}>Test mode</button>
              <button aria-pressed={s.live} onClick={() => setLive(true)}>Live (Claude)</button>
            </div>
            <p className="faint" style={{ marginBottom: 0 }}>
              Live uses <span className="code-inline">{s.tutor_model}</span> for lessons, <span className="code-inline">{s.curriculum_model}</span> to
              design courses, and <span className="code-inline">{s.escalate_model}</span> when you ask the expert.
            </p>
          </dd>
        </div>

        <div className="setting">
          <dt>Spent on Claude</dt>
          <dd>${s.spend_usd.toFixed(2)} <span className="faint">(estimate from token counts; check your Anthropic console for the exact bill)</span></dd>
        </div>

        <div className="setting">
          <dt>Local models</dt>
          <dd>
            {s.ollama_up ? (
              <>
                <span className="ok">Ollama is running.</span>{" "}
                <span className="faint">Installed: {s.ollama_models.join(", ") || "none"}</span>
                {!s.ollama_models.some((m) => m.startsWith(s.ollama_chat_model)) && (
                  <p className="faint" style={{ marginBottom: 0 }}>
                    <span className="code-inline">{s.ollama_chat_model}</span> isn't installed yet, so test mode uses the first chat model above.
                  </p>
                )}
              </>
            ) : (
              <span className="bad">Ollama isn't running. Start the Ollama app, then reopen this page.</span>
            )}
          </dd>
        </div>

        <div className="setting">
          <dt>Canvas feedback</dt>
          <dd>
            <span className="code-inline">{s.vision_model}</span>{" "}
            <span className="faint">
              (a model that can see your drawing; change it with GROQ_VISION_MODEL in .env. If it fails, feedback falls back to
              reading your shapes and arrows as text.)
            </span>
          </dd>
        </div>

        <div className="setting">
          <dt>Test-mode provider</dt>
          <dd>
            <span className="code-inline">{s.test_provider}</span>{" "}
            <span className="faint">(set TEST_PROVIDER to local, gemini or groq in .env)</span>
          </dd>
        </div>

        <div className="setting">
          <dt>API keys</dt>
          <dd>
            <div>Claude: {key(s.has_anthropic, "ANTHROPIC_API_KEY")}</div>
            <div>Gemini: {key(s.has_gemini, "GEMINI_API_KEY")}</div>
            <div>Groq: {key(s.has_groq, "GROQ_API_KEY")}</div>
            <p className="faint" style={{ marginBottom: 0 }}>
              Keys are read from <span className="code-inline">{s.env_path ?? ".env (not found)"}</span> at startup. Restart CPrep after editing it.
            </p>
          </dd>
        </div>
      </dl>
    </div>
  );
}
