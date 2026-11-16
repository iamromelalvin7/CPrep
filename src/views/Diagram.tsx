import { createContext, useContext, useEffect, useId, useState } from "react";
import { Icon } from "../icons";

/** Lets a diagram in the chat hand its Mermaid source to the canvas panel. */
export const CanvasBridge = createContext<((mermaid: string) => void) | null>(null);

type MermaidApi = typeof import("mermaid").default;
let mermaidReady: Promise<MermaidApi> | null = null;

function loadMermaid() {
  mermaidReady ??= import("mermaid").then(({ default: mermaid }) => {
    mermaid.initialize({
      startOnLoad: false,
      // Diagram text comes from a model: keep Mermaid's sanitising strict mode.
      securityLevel: "strict",
      theme: "base",
      fontFamily: '"IBM Plex Sans", "Segoe UI", sans-serif',
      themeVariables: {
        background: "#17201e",
        primaryColor: "#1e2926",
        primaryTextColor: "#e9e5d9",
        primaryBorderColor: "#f0c35a",
        secondaryColor: "#121916",
        tertiaryColor: "#1e2926",
        lineColor: "#97a39c",
        textColor: "#e9e5d9",
        edgeLabelBackground: "#17201e",
        clusterBkg: "#121916",
        clusterBorder: "#2e3a36",
        actorBkg: "#1e2926",
        actorBorder: "#f0c35a",
        actorTextColor: "#e9e5d9",
        signalColor: "#97a39c",
        signalTextColor: "#e9e5d9",
        noteBkgColor: "#2a2f22",
        noteTextColor: "#e9e5d9",
        fontSize: "14px",
      },
    });
    return mermaid;
  });
  return mermaidReady;
}

/** Renders a ```mermaid block from a tutor reply. While a reply is still streaming, shows a placeholder. */
export function Diagram({ code, streaming }: { code: string; streaming?: boolean }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const id = "mmd-" + useId().replace(/[^a-zA-Z0-9]/g, "");
  const openOnCanvas = useContext(CanvasBridge);

  useEffect(() => {
    if (streaming) return;
    let alive = true;
    setFailed(false);
    loadMermaid()
      .then((m) => m.render(id, code.trim()))
      .then(({ svg }) => alive && setSvg(svg))
      .catch(() => {
        if (alive) setFailed(true);
        // Mermaid leaves an error element behind on failure.
        document.getElementById("d" + id)?.remove();
      });
    return () => {
      alive = false;
    };
  }, [code, id, streaming]);

  if (streaming) return <div className="diagram diagram-pending">Drawing a diagram…</div>;
  if (failed)
    return (
      <figure className="diagram">
        <pre><code>{code}</code></pre>
        <figcaption className="faint">The tutor's diagram had a syntax error, so here is its source.</figcaption>
      </figure>
    );
  return (
    <figure className="diagram">
      {svg ? (
        <div className="diagram-svg" role="img" aria-label="Diagram" dangerouslySetInnerHTML={{ __html: svg }} />
      ) : (
        <div className="diagram-pending">Drawing…</div>
      )}
      {svg && openOnCanvas && (
        <figcaption>
          <button className="btn btn-quiet" onClick={() => openOnCanvas(code)}>
            <Icon name="canvas" size={14} /> Open on canvas
          </button>
        </figcaption>
      )}
    </figure>
  );
}
