import { useEffect, useMemo, useState } from "react";
import { api, Curriculum, Lesson, TrackSummary } from "../api";
import { Icon } from "../icons";

export type LessonProgress = {
  lessonId: number;
  topicIdx: number;
  status: Lesson["status"];
  /** Topics that have a place in the conversation to jump back to. */
  taught: number[];
};

type Props = {
  track: TrackSummary;
  lessonId: number;
  progress: LessonProgress | null;
  refreshKey: number;
  onOpenLesson: (id: number) => void;
  onBack: () => void;
};

export type TopicState = "done" | "current" | "locked";

export function topicStates(lesson: Lesson, progress: LessonProgress | null): TopicState[] {
  const idx = progress && progress.lessonId === lesson.id ? progress.topicIdx : lesson.topic_idx;
  const status = progress && progress.lessonId === lesson.id ? progress.status : lesson.status;
  return lesson.objectives.map((_, i) =>
    status === "done" ? "done" : i < idx ? "done" : i === idx ? "current" : "locked",
  );
}

export function jumpToTopic(i: number) {
  const el = document.getElementById(`topic-${i}`);
  if (!el) return false;
  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  el.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
  return true;
}

/** Sidebar while inside a course: phases → modules → lessons, with the open lesson's topics. */
export default function CourseNav({ track, lessonId, progress, refreshKey, onOpenLesson, onBack }: Props) {
  const [course, setCourse] = useState<Curriculum | null>(null);
  const [open, setOpen] = useState<Record<number, boolean>>({});

  useEffect(() => {
    api.getTrack(track.id).then((d) => setCourse(d.curriculum));
  }, [track.id, refreshKey]);

  const current = course?.lessons.find((l) => l.id === lessonId);

  // Keep the phase holding the open lesson expanded.
  useEffect(() => {
    if (current) setOpen((o) => ({ ...o, [current.phase_idx]: true }));
  }, [current?.phase_idx, current]);

  const grouped = useMemo(() => {
    if (!course) return [];
    return course.phases.map((p, pi) => ({
      title: p.title,
      pi,
      modules: p.modules.map((m, mi) => ({
        title: m.title,
        lessons: course.lessons.filter((l) => l.phase_idx === pi && l.module_idx === mi),
      })),
    }));
  }, [course]);

  return (
    <div className="course-nav">
      <button className="nav-back" onClick={onBack}>
        <Icon name="back" size={14} />
        {track.name}
      </button>
      {course && <div className="course-name">{course.title}</div>}
      <div className="course-tree">
        {grouped.map((p) => {
          const expanded = open[p.pi] ?? false;
          const lessons = p.modules.flatMap((m) => m.lessons);
          const done = lessons.filter((l) => l.status === "done" || l.status === "skipped").length;
          return (
            <section key={p.pi} className="nav-phase">
              <button
                className="nav-phase-head"
                aria-expanded={expanded}
                onClick={() => setOpen((o) => ({ ...o, [p.pi]: !expanded }))}
              >
                <Icon name="chevron" size={12} className="nav-chevron" />
                <span className="nav-phase-title">{p.title}</span>
                <span className="track-count">{done}/{lessons.length}</span>
              </button>
              {expanded && (
                <div className="nav-phase-body">
                  {p.modules.map((m, mi) => (
                    <div key={mi}>
                      <div className="nav-module">{m.title}</div>
                      {m.lessons.map((l) => {
                        const isCurrent = l.id === lessonId;
                        const status = isCurrent && progress?.lessonId === l.id ? progress.status : l.status;
                        return (
                          <div key={l.id}>
                            <button
                              className="nav-lesson"
                              aria-current={isCurrent ? "page" : undefined}
                              onClick={() => onOpenLesson(l.id)}
                            >
                              <span className="status-mark" data-status={status} aria-hidden="true" />
                              <span>{l.title}</span>
                            </button>
                            {isCurrent && <TopicList lesson={l} progress={progress} />}
                          </div>
                        );
                      })}
                    </div>
                  ))}
                </div>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}

function TopicList({ lesson, progress }: { lesson: Lesson; progress: LessonProgress | null }) {
  const states = topicStates(lesson, progress);
  const taught = new Set(progress?.lessonId === lesson.id ? progress.taught : []);
  return (
    <ol className="nav-topics" aria-label={`Topics in ${lesson.title}`}>
      {lesson.objectives.map((t, i) => {
        const state = states[i];
        const canJump = state !== "locked" && taught.has(i);
        return (
          <li key={i}>
            <button
              className="nav-topic"
              data-state={state}
              aria-current={state === "current" ? "step" : undefined}
              aria-disabled={!canJump}
              title={state === "locked" ? "Unlocks when you finish the topics before it" : canJump ? "Jump to this topic" : undefined}
              onClick={() => canJump && jumpToTopic(i)}
            >
              <span className="topic-mark" aria-hidden="true">
                {state === "done" ? <Icon name="check" size={12} /> : state === "locked" ? <Icon name="lock" size={11} /> : <span className="topic-dot" />}
              </span>
              <span>{t}</span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}
