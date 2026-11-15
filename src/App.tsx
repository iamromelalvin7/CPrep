import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { api, Settings as SettingsT, TrackSummary } from "./api";
import TrackHome from "./views/TrackHome";
import LessonView from "./views/Lesson";
import SettingsView from "./views/Settings";

export type Route =
  | { kind: "track"; trackId: string }
  | { kind: "lesson"; trackId: string; lessonId: number }
  | { kind: "settings" };

export default function App() {
  const [tracks, setTracks] = useState<TrackSummary[]>([]);
  const [settings, setSettings] = useState<SettingsT | null>(null);
  const [route, setRoute] = useState<Route>({ kind: "track", trackId: "backend" });
  const [menuOpen, setMenuOpen] = useState(false);
  const reduce = useReducedMotion();

  const refresh = useCallback(async () => {
    const [t, s] = await Promise.all([api.listTracks(), api.getSettings()]);
    setTracks(t);
    setSettings(s);
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const go = (r: Route) => {
    setRoute(r);
    setMenuOpen(false);
  };

  const current = route.kind === "settings" ? null : tracks.find((t) => t.id === route.trackId) ?? null;
  const key = route.kind === "lesson" ? `l${route.lessonId}` : route.kind === "track" ? `t${route.trackId}` : "s";

  return (
    <div className="shell">
      <header className="topbar">
        <button className="btn btn-quiet menu-btn" aria-label="Open tracks" onClick={() => setMenuOpen(true)}>
          <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
            <path d="M2 4h14M2 9h14M2 14h14" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
          </svg>
        </button>
        <div className="brand">CPrep</div>
      </header>

      <nav className="sidebar" data-open={menuOpen} aria-label="Tracks" onClick={(e) => e.target === e.currentTarget && setMenuOpen(false)}>
        <div className="brand">
          CPrep
          <small>Career prep, from your own notes</small>
        </div>
        <ul className="track-list">
          {tracks.map((t) => {
            const active = route.kind !== "settings" && route.trackId === t.id;
            return (
              <li key={t.id}>
                <button
                  className="track-link"
                  aria-current={active ? "page" : undefined}
                  data-paused={t.status === "paused"}
                  onClick={() => go({ kind: "track", trackId: t.id })}
                >
                  <span className="track-dot" style={{ background: t.accent }} />
                  <span>{t.name}</span>
                  <span className="track-count">
                    {t.status === "paused" ? "paused" : t.lessons_total ? `${t.lessons_done}/${t.lessons_total}` : ""}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
        <div className="sidebar-foot">
          <div className="mode-pill" data-live={settings?.live ?? false}>
            <span className="lamp" />
            {settings?.live ? `Live · ${settings.tutor_model}` : "Test mode · free models"}
          </div>
          <button
            className="track-link"
            aria-current={route.kind === "settings" ? "page" : undefined}
            onClick={() => go({ kind: "settings" })}
          >
            <span />
            <span>Settings</span>
            <span className="track-count">{settings && settings.spend_usd > 0 ? `$${settings.spend_usd.toFixed(2)}` : ""}</span>
          </button>
        </div>
      </nav>

      <main className="main">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={key}
            style={{ height: "100%" }}
            initial={reduce ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? undefined : { opacity: 0 }}
            transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
          >
            {route.kind === "settings" && <SettingsView settings={settings} onChange={refresh} />}
            {route.kind === "track" && current && (
              <TrackHome track={current} onOpenLesson={(id) => go({ kind: "lesson", trackId: current.id, lessonId: id })} onChanged={refresh} />
            )}
            {route.kind === "lesson" && current && (
              <LessonView
                track={current}
                lessonId={route.lessonId}
                live={settings?.live ?? false}
                onBack={() => go({ kind: "track", trackId: current.id })}
                onOpenLesson={(id) => go({ kind: "lesson", trackId: current.id, lessonId: id })}
                onChanged={refresh}
              />
            )}
          </motion.div>
        </AnimatePresence>
      </main>
    </div>
  );
}
