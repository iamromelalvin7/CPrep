import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { api, Settings as SettingsT, TrackSummary } from "./api";
import { Icon } from "./icons";
import TrackHome from "./views/TrackHome";
import LessonView from "./views/Lesson";
import SettingsView from "./views/Settings";
import CourseNav, { LessonProgress } from "./views/CourseNav";

export type Route =
  | { kind: "track"; trackId: string }
  | { kind: "lesson"; trackId: string; lessonId: number }
  | { kind: "settings" };

const SIDEBAR_KEY = "cprep.sidebar";

function readCollapsed() {
  try {
    return localStorage.getItem(SIDEBAR_KEY) === "collapsed";
  } catch {
    return false;
  }
}

export default function App() {
  const [tracks, setTracks] = useState<TrackSummary[]>([]);
  const [settings, setSettings] = useState<SettingsT | null>(null);
  const [route, setRoute] = useState<Route>({ kind: "track", trackId: "backend" });
  const [menuOpen, setMenuOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const [progress, setProgress] = useState<LessonProgress | null>(null);
  const [navKey, setNavKey] = useState(0);
  const reduce = useReducedMotion();

  const refresh = useCallback(async () => {
    const [t, s] = await Promise.all([api.listTracks(), api.getSettings()]);
    setTracks(t);
    setSettings(s);
    setNavKey((k) => k + 1);
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const toggleCollapsed = (value: boolean) => {
    setCollapsed(value);
    try {
      localStorage.setItem(SIDEBAR_KEY, value ? "collapsed" : "open");
    } catch {
      /* storage unavailable: keep it for this session only */
    }
  };

  const go = (r: Route) => {
    setRoute(r);
    setMenuOpen(false);
  };

  const current = route.kind === "settings" ? null : tracks.find((t) => t.id === route.trackId) ?? null;
  const key = route.kind === "lesson" ? `l${route.lessonId}` : route.kind === "track" ? `t${route.trackId}` : "s";

  return (
    <div className="shell" data-collapsed={collapsed}>
      <header className="topbar">
        <button className="btn btn-quiet menu-btn" aria-label="Open navigation" onClick={() => setMenuOpen(true)}>
          <Icon name="sidebar" size={18} />
        </button>
        <div className="brand">CPrep</div>
      </header>

      {collapsed && (
        <div className="rail">
          <button className="icon-btn" aria-label="Show sidebar" title="Show sidebar" onClick={() => toggleCollapsed(false)}>
            <Icon name="sidebar" size={18} />
          </button>
        </div>
      )}

      <nav
        className="sidebar"
        data-open={menuOpen}
        aria-label="Navigation"
        onClick={(e) => e.target === e.currentTarget && setMenuOpen(false)}
      >
        <div className="sidebar-head">
          <div className="brand">
            CPrep
            <small>Learn from your own notes</small>
          </div>
          <button className="icon-btn collapse-btn" aria-label="Hide sidebar" title="Hide sidebar" onClick={() => toggleCollapsed(true)}>
            <Icon name="sidebar" size={18} />
          </button>
        </div>

        {route.kind === "lesson" && current ? (
          <CourseNav
            track={current}
            lessonId={route.lessonId}
            progress={progress}
            refreshKey={navKey}
            onOpenLesson={(id) => go({ kind: "lesson", trackId: current.id, lessonId: id })}
            onBack={() => go({ kind: "track", trackId: current.id })}
          />
        ) : (
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
        )}

        <div className="sidebar-foot">
          <div className="mode-pill" data-live={settings?.live ?? false}>
            <span className="lamp" />
            {settings?.live ? `Live · ${settings.tutor_model}` : `Test mode · ${settings?.test_provider === "local" ? "local model" : settings?.test_provider ?? "free"}`}
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
                onProgress={setProgress}
              />
            )}
          </motion.div>
        </AnimatePresence>
      </main>
    </div>
  );
}
