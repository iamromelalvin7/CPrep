// Turns an Excalidraw scene into a short text structure (boxes, labels, arrows between them) so a
// text-only model can critique a diagram.

type El = {
  id: string;
  type: string;
  x: number;
  y: number;
  width: number;
  height: number;
  isDeleted?: boolean;
  text?: string;
  containerId?: string | null;
  points?: [number, number][];
  startBinding?: { elementId: string } | null;
  endBinding?: { elementId: string } | null;
  boundElements?: { id: string; type: string }[] | null;
  strokeStyle?: string;
  frameId?: string | null;
  name?: string | null;
};

const SHAPES = new Set(["rectangle", "ellipse", "diamond", "frame", "image", "embeddable", "iframe"]);
const LINES = new Set(["arrow", "line"]);

const clean = (s: string) => s.replace(/\s+/g, " ").trim();
const center = (e: El) => ({ x: e.x + e.width / 2, y: e.y + e.height / 2 });

export function describeScene(all: readonly El[]): string {
  const els = all.filter((e) => !e.isDeleted);
  if (els.length === 0) return "";
  const byId = new Map(els.map((e) => [e.id, e]));
  const labelOf = new Map<string, string>();
  for (const t of els) {
    if (t.type === "text" && t.containerId && t.text) {
      labelOf.set(t.containerId, clean((labelOf.get(t.containerId) ?? "") + " " + t.text));
    }
  }
  const shapes = els.filter((e) => SHAPES.has(e.type));
  // Text placed on top of a shape (not bound to it) still labels that shape.
  for (const t of els) {
    if (t.type !== "text" || t.containerId || !t.text) continue;
    const c = center(t);
    const host = shapes.find((s) => c.x >= s.x && c.x <= s.x + s.width && c.y >= s.y && c.y <= s.y + s.height);
    if (host && !labelOf.has(host.id)) labelOf.set(host.id, clean(t.text));
  }

  const names = new Map<string, string>();
  shapes
    .slice()
    .sort((a, b) => a.y - b.y || a.x - b.x)
    .forEach((s, i) => {
      const label = s.type === "frame" ? s.name || labelOf.get(s.id) : labelOf.get(s.id);
      names.set(s.id, label ? `"${label}"` : `unlabelled ${s.type} #${i + 1}`);
    });

  const nearestShape = (p: { x: number; y: number }) => {
    let best: El | null = null;
    let bestD = Infinity;
    for (const s of shapes) {
      const dx = Math.max(s.x - p.x, 0, p.x - (s.x + s.width));
      const dy = Math.max(s.y - p.y, 0, p.y - (s.y + s.height));
      const d = Math.hypot(dx, dy);
      if (d < bestD) {
        bestD = d;
        best = s;
      }
    }
    return best && bestD < 60 ? best : null;
  };

  const lines: string[] = [];
  const shapeLines = shapes
    .slice()
    .sort((a, b) => a.y - b.y || a.x - b.x)
    .map((s) => {
      const inside = s.frameId && byId.get(s.frameId) ? ` (inside ${names.get(s.frameId)})` : "";
      return `- ${s.type} ${names.get(s.id)}${inside}`;
    });
  if (shapeLines.length) lines.push("Shapes (top to bottom, left to right):", ...shapeLines);

  const conns: string[] = [];
  for (const a of els.filter((e) => LINES.has(e.type))) {
    const pts = a.points ?? [[0, 0], [a.width, a.height]];
    const start = { x: a.x + pts[0][0], y: a.y + pts[0][1] };
    const end = { x: a.x + pts[pts.length - 1][0], y: a.y + pts[pts.length - 1][1] };
    const from = (a.startBinding && byId.get(a.startBinding.elementId)) || nearestShape(start);
    const to = (a.endBinding && byId.get(a.endBinding.elementId)) || nearestShape(end);
    const fromName = from ? names.get(from.id) ?? `"${labelOf.get(from.id) ?? from.type}"` : "(nothing)";
    const toName = to ? names.get(to.id) ?? `"${labelOf.get(to.id) ?? to.type}"` : "(nothing)";
    const label = labelOf.get(a.id);
    const style = a.strokeStyle && a.strokeStyle !== "solid" ? `, ${a.strokeStyle}` : "";
    const sym = a.type === "arrow" ? "→" : "—";
    conns.push(`- ${fromName} ${sym} ${toName}${label ? ` [label "${label}"]` : ""}${style}`);
  }
  if (conns.length) lines.push("Connections:", ...conns);

  const used = new Set([...shapes.map((s) => s.id)]);
  const free = els
    .filter((e) => e.type === "text" && e.text && !e.containerId)
    .filter((t) => {
      const c = center(t);
      return !shapes.some((s) => used.has(s.id) && c.x >= s.x && c.x <= s.x + s.width && c.y >= s.y && c.y <= s.y + s.height);
    })
    .map((t) => `- "${clean(t.text!)}"`);
  if (free.length) lines.push("Free text notes:", ...free);

  const sketches = els.filter((e) => e.type === "freedraw").length;
  if (sketches) lines.push(`(${sketches} freehand stroke${sketches === 1 ? "" : "s"} not described)`);
  return lines.join("\n");
}
