// Whiteboard panel: Excalidraw, saved per lesson. Fonts are bundled in /excalidraw-assets so it works offline.
(window as unknown as { EXCALIDRAW_ASSET_PATH: string }).EXCALIDRAW_ASSET_PATH = "/excalidraw-assets/";

import "@excalidraw/excalidraw/index.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { Excalidraw, MainMenu, convertToExcalidrawElements, exportToBlob } from "@excalidraw/excalidraw";
import type { ExcalidrawImperativeAPI } from "@excalidraw/excalidraw/types";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../api";
import { Icon, MdIcon, remarkEmojiIcons } from "../icons";
import { describeScene } from "../describeScene";

type Props = {
  lessonId: number;
  /** Mermaid source waiting to be placed on the canvas (from "Open on canvas"). */
  pendingMermaid: string | null;
  onMermaidPlaced: () => void;
  onClose: () => void;
};

const mdPlugins = [remarkGfm, remarkEmojiIcons];
const mdComponents = { i: MdIcon as never };
type AnyEl = Parameters<typeof describeScene>[0][number];

async function blobToBase64(b: Blob): Promise<string> {
  const buf = new Uint8Array(await b.arrayBuffer());
  let bin = "";
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return btoa(bin);
}

export default function Canvas({ lessonId, pendingMermaid, onMermaidPlaced, onClose }: Props) {
  const [excal, setExcal] = useState<ExcalidrawImperativeAPI | null>(null);
  const [initial, setInitial] = useState<{ elements: unknown[] } | null | undefined>(undefined);
  const [task, setTask] = useState<string | null>(null);
  const [taskOpen, setTaskOpen] = useState(true);
  const [critique, setCritique] = useState<string | null>(null);
  const [critiqueMeta, setCritiqueMeta] = useState<string | null>(null);
  const [busy, setBusy] = useState<"task" | "critique" | "mermaid" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const lastSaved = useRef("");

  useEffect(() => {
    setInitial(undefined);
    api.getCanvas(lessonId).then((c) => {
      let elements: unknown[] = [];
      try {
        elements = c.scene ? (JSON.parse(c.scene).elements ?? []) : [];
      } catch {
        elements = [];
      }
      lastSaved.current = c.scene ?? "";
      setInitial({ elements });
      setTask(c.task);
      setCritique(c.critique);
    });
  }, [lessonId]);

  const onChange = useCallback(
    (elements: readonly { isDeleted?: boolean }[]) => {
      clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        const scene = JSON.stringify({ elements: elements.filter((e) => !e.isDeleted) });
        if (scene === lastSaved.current) return;
        lastSaved.current = scene;
        api.saveCanvas(lessonId, scene).catch(() => {});
      }, 700);
    },
    [lessonId],
  );

  // Place a tutor diagram on the canvas, to the right of whatever is already there.
  useEffect(() => {
    if (!excal || !pendingMermaid) return;
    let cancelled = false;
    (async () => {
      setBusy("mermaid");
      setError(null);
      try {
        const { parseMermaidToExcalidraw } = await import("@excalidraw/mermaid-to-excalidraw");
        const { elements: skeleton } = await parseMermaidToExcalidraw(pendingMermaid, { themeVariables: { fontSize: "16px" } });
        const fresh = convertToExcalidrawElements(skeleton as never, { regenerateIds: true });
        const existing = excal.getSceneElements();
        const maxX = existing.reduce((m, e) => Math.max(m, e.x + e.width), -Infinity);
        const minY = existing.reduce((m, e) => Math.min(m, e.y), Infinity);
        const dx = existing.length ? maxX + 120 : 0;
        const dy = existing.length ? minY : 0;
        const placed = fresh.map((e) => ({ ...e, x: e.x + dx, y: e.y + dy }));
        if (cancelled) return;
        excal.updateScene({ elements: [...existing, ...placed] });
        excal.scrollToContent(placed, { fitToContent: true, animate: true });
      } catch (e) {
        setError(`Couldn't turn that diagram into shapes: ${String(e).slice(0, 160)}`);
      } finally {
        setBusy(null);
        onMermaidPlaced();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [excal, pendingMermaid, onMermaidPlaced]);

  const newTask = async () => {
    setBusy("task");
    setError(null);
    try {
      setTask(await api.canvasTask(lessonId));
      setTaskOpen(true);
      setCritique(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const getFeedback = async () => {
    if (!excal) return;
    setBusy("critique");
    setError(null);
    setCritique("");
    setCritiqueMeta(null);
    try {
      const elements = excal.getSceneElements();
      const description = describeScene(elements as unknown as AnyEl[]);
      let image: string | null = null;
      if (elements.length) {
        const blob = await exportToBlob({
          elements,
          files: excal.getFiles(),
          appState: { exportBackground: true, viewBackgroundColor: "#ffffff", exportWithDarkMode: false },
          mimeType: "image/png",
          maxWidthOrHeight: 1400,
        });
        image = await blobToBase64(blob);
      }
      await api.critiqueCanvas(lessonId, description, image, (e) => {
        if (e.type === "delta") setCritique((c) => (c ?? "") + e.text);
        if (e.type === "done") setCritiqueMeta(`Reviewed by ${e.provider}`);
        if (e.type === "error") setError(e.message);
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <aside className="lab canvas-panel" aria-label="Canvas">
      <header className="lab-head">
        <div className="lab-title">
          <Icon name="canvas" size={16} />
          Canvas
        </div>
        <div className="grow" />
        <button className="btn btn-quiet" disabled={busy !== null} onClick={newTask}>
          {busy === "task" ? <span className="spinner" /> : null}
          {task ? "New task" : "Drawing task"}
        </button>
        <button className="btn btn-primary" disabled={busy !== null || !excal} onClick={getFeedback}>
          {busy === "critique" ? <span className="spinner" /> : null} Get feedback
        </button>
        <button className="icon-btn" aria-label="Close canvas" title="Close canvas" onClick={onClose}>
          <Icon name="close" size={16} />
        </button>
      </header>

      {task && (
        <section className="canvas-task" data-open={taskOpen}>
          <button className="canvas-task-toggle" aria-expanded={taskOpen} onClick={() => setTaskOpen(!taskOpen)}>
            <Icon name="chevron" size={12} className="nav-chevron" />
            Your task
          </button>
          {taskOpen && (
            <div className="lab-prompt">
              <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{task}</Markdown>
            </div>
          )}
        </section>
      )}
      {error && <div className="error-note canvas-error" role="alert">{error}</div>}
      {busy === "mermaid" && <p className="faint canvas-status" role="status">Placing the diagram…</p>}

      <div className="canvas-stage">
        {initial === undefined ? (
          <span className="spinner" aria-label="Loading canvas" />
        ) : (
          <Excalidraw
            key={lessonId}
            theme="dark"
            initialData={{ elements: initial?.elements as never, appState: { viewBackgroundColor: "#ffffff" }, scrollToContent: true }}
            excalidrawAPI={(api) => {
              setExcal(api);
              // Dev-only handle for automated UI checks.
              if (import.meta.env.DEV) (window as unknown as { __excal: unknown }).__excal = api;
            }}
            onChange={onChange as never}
            UIOptions={{ canvasActions: { loadScene: false, saveToActiveFile: false, export: false, saveAsImage: true, toggleTheme: false } }}
          >
            <MainMenu>
              <MainMenu.DefaultItems.SaveAsImage />
              <MainMenu.DefaultItems.ClearCanvas />
              <MainMenu.DefaultItems.Help />
            </MainMenu>
          </Excalidraw>
        )}
      </div>

      {critique !== null && (critique || busy === "critique") && (
        <section className="canvas-critique" aria-live="polite">
          <div className="canvas-critique-head">
            <span>Feedback on your drawing</span>
            <button className="icon-btn" aria-label="Hide feedback" onClick={() => setCritique(null)}>
              <Icon name="close" size={14} />
            </button>
          </div>
          <div className="prose">
            <Markdown remarkPlugins={mdPlugins} components={mdComponents}>{critique}</Markdown>
            {busy === "critique" && <span className="caret" aria-hidden="true" />}
          </div>
          {critiqueMeta && <div className="turn-meta">{critiqueMeta}</div>}
        </section>
      )}
    </aside>
  );
}
