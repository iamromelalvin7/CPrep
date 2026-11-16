// Small inline SVG icon set. Also used to swap emoji in tutor replies for icons that match the UI.
import type { ReactElement, SVGProps } from "react";

type IconName =
  | "check"
  | "cross"
  | "warning"
  | "bulb"
  | "celebrate"
  | "pin"
  | "book"
  | "key"
  | "arrow"
  | "star"
  | "lock"
  | "chevron"
  | "sidebar"
  | "back";

const paths: Record<IconName, ReactElement> = {
  check: <path d="M3.5 8.5l3 3 6-7" />,
  cross: <path d="M4 4l8 8M12 4l-8 8" />,
  warning: (
    <>
      <path d="M8 2.2L14.2 13H1.8z" />
      <path d="M8 6.5v3" />
      <circle cx="8" cy="11.4" r="0.4" fill="currentColor" />
    </>
  ),
  bulb: (
    <>
      <path d="M5.6 10.4A4.2 4.2 0 1 1 10.4 10.4c-.6.5-.9 1-.9 1.6H6.5c0-.6-.3-1.1-.9-1.6z" />
      <path d="M6.5 14h3" />
    </>
  ),
  celebrate: (
    <>
      <path d="M2.5 13.5l3.2-8.3 5.1 5.1z" />
      <path d="M9.5 2.5c.3 1.2-.2 2-1.2 2.4M13.5 6.5c-1.2-.3-2 .2-2.4 1.2" />
      <path d="M11.5 2.2v1.2M13.8 3.9h-1.2M12.6 9.8l.9.9" />
    </>
  ),
  pin: (
    <>
      <path d="M8 14.5V9.5" />
      <path d="M5 2.5h6l-1 3.2 2 2.3H4l2-2.3z" />
    </>
  ),
  book: (
    <>
      <path d="M2.5 3.5c2-.8 3.8-.6 5.5.8v9c-1.7-1.4-3.5-1.6-5.5-.8z" />
      <path d="M13.5 3.5c-2-.8-3.8-.6-5.5.8v9c1.7-1.4 3.5-1.6 5.5-.8z" />
    </>
  ),
  key: (
    <>
      <circle cx="5.5" cy="10.5" r="2.8" />
      <path d="M7.5 8.5l5.5-5.5M11 4.5l1.5 1.5" />
    </>
  ),
  arrow: <path d="M3 8h9.5M9 4.5L12.5 8 9 11.5" />,
  star: <path d="M8 2.2l1.8 3.7 4 .6-2.9 2.8.7 4L8 11.4l-3.6 1.9.7-4-2.9-2.8 4-.6z" />,
  lock: (
    <>
      <rect x="3.5" y="7.2" width="9" height="6.3" rx="1.2" />
      <path d="M5.5 7.2V5.4a2.5 2.5 0 0 1 5 0v1.8" />
    </>
  ),
  chevron: <path d="M6 3.5L10.5 8 6 12.5" />,
  sidebar: (
    <>
      <rect x="2" y="2.8" width="12" height="10.4" rx="1.6" />
      <path d="M6.2 2.8v10.4" />
    </>
  ),
  back: <path d="M10 3L5 8l5 5" />,
};

export type { IconName };

export function Icon({ name, size = 16, ...rest }: { name: IconName; size?: number } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      {paths[name]}
    </svg>
  );
}

// ---------- Emoji → icon ----------

const EMOJI_ICONS: [RegExp, IconName][] = [
  [/^(✅|✔|☑|🟢)$/u, "check"],
  [/^(❌|✖|⛔|🚫)$/u, "cross"],
  [/^(⚠|❗|❕|🚨)$/u, "warning"],
  [/^(💡|🧠|🤔)$/u, "bulb"],
  [/^(🎉|🥳|🎊|🏆|🏁)$/u, "celebrate"],
  [/^(📌|📍)$/u, "pin"],
  [/^(📚|📖|📘|📝|🗒)$/u, "book"],
  [/^(🔑|🗝|🔐)$/u, "key"],
  [/^(👉|➡|▶|🔜|🚀)$/u, "arrow"],
  [/^(⭐|🌟|✨)$/u, "star"],
];

export function iconForEmoji(e: string): IconName | null {
  const base = e.replace(/️|‍/g, "");
  for (const [re, name] of EMOJI_ICONS) if (re.test(base)) return name;
  return null;
}

const EMOJI_RE = /(\p{Extended_Pictographic}️?(?:‍\p{Extended_Pictographic}️?)*)/gu;

type MdNode = { type: string; value?: string; children?: MdNode[]; data?: Record<string, unknown> };

/** Remark plugin: replaces emoji in text with icon nodes (rendered by the `i` component) or drops them. */
export function remarkEmojiIcons() {
  const walk = (node: MdNode) => {
    if (!node.children) return;
    const out: MdNode[] = [];
    for (const child of node.children) {
      if (child.type === "text" && child.value && EMOJI_RE.test(child.value)) {
        EMOJI_RE.lastIndex = 0;
        for (const part of child.value.split(EMOJI_RE)) {
          if (!part) continue;
          if (EMOJI_RE.test(part)) {
            EMOJI_RE.lastIndex = 0;
            const name = iconForEmoji(part);
            if (name) out.push({ type: "emojiIcon", data: { hName: "i", hProperties: { "data-icon": name } } });
          } else {
            // Drop the space an emoji leaves at the start of a line.
            out.push({ type: "text", value: out.length === 0 ? part.replace(/^\s+/, "") : part });
          }
        }
        EMOJI_RE.lastIndex = 0;
      } else {
        if (child.type !== "code" && child.type !== "inlineCode") walk(child);
        out.push(child);
      }
    }
    node.children = out;
  };
  return (tree: MdNode) => walk(tree);
}

/** Markdown component for icon nodes produced by remarkEmojiIcons. */
export function MdIcon(props: { "data-icon"?: string; dataIcon?: string; node?: unknown }) {
  const name = (props["data-icon"] ?? props.dataIcon) as IconName | undefined;
  if (!name || !(name in paths)) return null;
  return (
    <span className="md-icon" data-icon={name}>
      <Icon name={name} size={15} />
    </span>
  );
}
