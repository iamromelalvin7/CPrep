// Typed bridge to the Rust core. Outside Tauri (plain browser, Playwright) a mock backend
// with sample data is used so the UI can be developed and screenshotted on its own.
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

export type TrackSummary = {
  id: string;
  name: string;
  blurb: string;
  accent: string;
  status: "active" | "paused";
  folders: number;
  documents: number;
  lessons_total: number;
  lessons_done: number;
};

export type Settings = {
  live: boolean;
  has_anthropic: boolean;
  has_gemini: boolean;
  has_groq: boolean;
  env_path: string | null;
  test_provider: string;
  tutor_model: string;
  curriculum_model: string;
  escalate_model: string;
  ollama_chat_model: string;
  ollama_embed_model: string;
  ollama_up: boolean;
  notes_dir: string;
  vision_model: string;
  ollama_models: string[];
  spend_usd: number;
};

export type Folder = { id: number; path: string; last_ingested: string | null; documents: number; chunks: number };
export type TopicScore = { topic: string; correct: number; total: number };
export type Review = { prompt: string; chosen: number | null; answer: number; options: string[]; explanation: string };
export type PlacementResult = {
  correct: number;
  total: number;
  topics: TopicScore[];
  strong: string[];
  weak: string[];
  review: Review[];
};
export type Question = { topic: string; prompt: string; options: string[] };
export type LessonStatus = "todo" | "in_progress" | "done" | "skipped";
export type Lesson = {
  id: number;
  curriculum_id: number;
  ord: number;
  phase_idx: number;
  module_idx: number;
  phase: string;
  module: string;
  title: string;
  /** The lesson's topics, in teaching order. */
  objectives: string[];
  status: LessonStatus;
  /** 0-based index of the topic the tutor has reached. */
  topic_idx: number;
};
export type Phase = { title: string; goal: string; modules: { title: string; tested_out: boolean }[] };
export type Curriculum = { id: number; title: string; summary: string; phases: Phase[]; lessons: Lesson[] };
export type TrackDetail = {
  folders: Folder[];
  known: string[];
  placement: PlacementResult | null;
  curriculum: Curriculum | null;
};
export type Source = { chunk_id: number; title: string; path: string; page: number | null; text: string; score: number };
export type Message = {
  id: number;
  role: "user" | "assistant";
  content: string;
  sources: Source[];
  created_at: string;
  topic_idx: number | null;
};
export type LessonView = { lesson: Lesson; messages: Message[]; prev_id: number | null; next_id: number | null };
export type IngestProgress = { folder_id: number; stage: string; done: number; total: number; current: string };
export type IngestReport = { scanned: number; updated: number; removed: number; chunks_embedded: number; failed: string[] };
export type ChatEvent =
  | { type: "sources"; sources: Source[] }
  | { type: "delta"; text: string }
  | { type: "done"; messageId: number; provider: string; costUsd: number; topicIdx: number }
  | { type: "error"; message: string };

export type LabLanguage = "python" | "javascript" | "sql" | "java";
export type LabKind = "function" | "output" | "query";
export type TestCase = { args: unknown[]; expected: unknown; stdin: string; expected_output: string; hidden: boolean };
export type Exercise = {
  id: number;
  lesson_id: number;
  topic_idx: number;
  title: string;
  prompt: string;
  language: LabLanguage;
  kind: LabKind;
  function_name: string;
  starter_code: string;
  setup_sql: string;
  draft: string | null;
  examples: TestCase[];
  hidden_count: number;
  best: [number, number] | null;
  attempts: number;
};
export type RunOutput = { stdout: string; stderr: string; exit_code: number | null; timed_out: boolean; millis: number };
export type TestResult = {
  passed: boolean;
  hidden: boolean;
  input: string | null;
  expected: string | null;
  got: string | null;
  error: string | null;
};
export type Grade = { passed: number; total: number; results: TestResult[]; error: string | null; stdout: string };
export type LabEvent =
  | { type: "graded"; grade: Grade }
  | { type: "delta"; text: string }
  | { type: "done"; attemptId: number; provider: string; costUsd: number }
  | { type: "error"; message: string };

export type CanvasState = { scene: string | null; task: string | null; critique: string | null };

export type Repeat = "none" | "daily" | "weekdays" | "weekly";
export type Session = {
  id: number;
  track_id: string;
  lesson_id: number | null;
  lesson_title: string | null;
  title: string;
  starts_at: string;
  duration_min: number;
  repeat: Repeat;
};
export type NewSession = Omit<Session, "id" | "lesson_title">;
export type Occurrence = { session: Session; start: string; end: string };
export type ReminderSettings = { offsets: number[]; review_time: string | null; keep_in_tray: boolean; autostart: boolean };
export type Memory = { stability: number; difficulty: number; reps: number; lapses: number };
export type Card = {
  id: number;
  track_id: string;
  lesson_id: number | null;
  lesson_title: string | null;
  front: string;
  back: string;
  due: string;
  memory: Memory;
  last_review: string | null;
  preview: string[];
};
export type ReviewSummary = {
  due_now: number;
  total: number;
  new_cards: number;
  reviewed_today: number;
  next_due: string | null;
  by_track: [string, number][];
};
export type LessonCards = { lesson_id: number; title: string; track_id: string; cards: number; status: string };

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return mock<T>(cmd, args ?? {});
  return tauriInvoke<T>(cmd, args);
}

export const api = {
  listTracks: () => call<TrackSummary[]>("list_tracks"),
  getSettings: () => call<Settings>("get_settings"),
  setLive: (live: boolean) => call<void>("set_live", { live }),
  getTrack: (trackId: string) => call<TrackDetail>("get_track", { trackId }),
  addFolder: (trackId: string, path: string) => call<number>("add_folder", { trackId, path }),
  removeFolder: (folderId: number) => call<void>("remove_folder", { folderId }),
  ingestFolder: (folderId: number) => call<IngestReport>("ingest_folder", { folderId }),
  startPlacement: (trackId: string, known: string[], note: string) =>
    call<Question[]>("start_placement", { trackId, known, note }),
  submitPlacement: (trackId: string, answers: (number | null)[]) =>
    call<PlacementResult>("submit_placement", { trackId, answers }),
  skipPlacement: (trackId: string, known: string[]) => call<void>("skip_placement", { trackId, known }),
  generateCurriculum: (trackId: string) => call<number>("generate_curriculum", { trackId }),
  getLesson: (lessonId: number) => call<LessonView>("get_lesson", { lessonId }),
  setLessonStatus: (lessonId: number, status: LessonStatus) => call<void>("set_lesson_status", { lessonId, status }),
  retakeLesson: (lessonId: number) => call<void>("retake_lesson", { lessonId }),
  sendMessage: async (lessonId: number, text: string, escalate: boolean, onEvent: (e: ChatEvent) => void) => {
    if (!inTauri) return mockChat(text, onEvent);
    const ch = new Channel<ChatEvent>();
    ch.onmessage = onEvent;
    await tauriInvoke("send_message", { lessonId, text, escalate, onEvent: ch });
  },
  listExercises: (lessonId: number) => call<Exercise[]>("list_exercises", { lessonId }),
  newExercise: (lessonId: number) => call<Exercise>("new_exercise", { lessonId }),
  runCode: (exerciseId: number, code: string, stdin: string) => call<RunOutput>("run_code", { exerciseId, code, stdin }),
  saveDraft: (exerciseId: number, code: string) => call<void>("save_draft", { exerciseId, code }),
  submitCode: async (exerciseId: number, code: string, onEvent: (e: LabEvent) => void) => {
    if (!inTauri) return mockSubmit(code, onEvent);
    const ch = new Channel<LabEvent>();
    ch.onmessage = onEvent;
    await tauriInvoke("submit_code", { exerciseId, code, onEvent: ch });
  },
  writeNotes: (lessonId: number) => call<string>("write_notes", { lessonId }),
  notesPath: (lessonId: number) => call<string | null>("notes_path", { lessonId }),
  openNotes: (path: string | null) => call<void>("open_notes", { path }),
  getCanvas: (lessonId: number) => call<CanvasState>("get_canvas", { lessonId }),
  saveCanvas: (lessonId: number, scene: string) => call<void>("save_canvas", { lessonId, scene }),
  canvasTask: (lessonId: number) => call<string>("canvas_task", { lessonId }),
  critiqueCanvas: async (
    lessonId: number,
    description: string,
    imagePng: string | null,
    onEvent: (e: LabEvent) => void,
  ) => {
    if (!inTauri) return mockCritique(description, onEvent);
    const ch = new Channel<LabEvent>();
    ch.onmessage = onEvent;
    await tauriInvoke("critique_canvas", { lessonId, description, imagePng, onEvent: ch });
  },
  listSessions: () => call<Session[]>("list_sessions"),
  getAgenda: (from: string, to: string) => call<Occurrence[]>("get_agenda", { from, to }),
  addSession: (session: NewSession) => call<number>("add_session", { session }),
  deleteSession: (id: number) => call<void>("delete_session", { id }),
  getReminders: () => call<ReminderSettings>("get_reminders"),
  saveReminders: (settings: ReminderSettings) => call<void>("save_reminders", { settings }),
  testNotification: () => call<void>("test_notification"),
  reviewSummary: () => call<ReviewSummary>("review_summary"),
  dueCards: (trackId: string | null) => call<Card[]>("due_cards", { trackId }),
  rateCard: (cardId: number, rating: 1 | 2 | 3 | 4) => call<string>("rate_card", { cardId, rating }),
  makeCards: (lessonId: number) => call<number>("make_cards", { lessonId }),
  lessonsForCards: () => call<LessonCards[]>("lessons_for_cards"),
  pickFolder: async (): Promise<string | null> => {
    if (!inTauri) return "C:\\Users\\you\\Desktop\\programming\\BACKEND JOURNEY";
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ directory: true, multiple: false, title: "Choose a study folder" });
    return typeof r === "string" ? r : null;
  },
  confirm: async (message: string, okLabel: string): Promise<boolean> => {
    if (!inTauri) return window.confirm(message);
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return ask(message, { title: "CPrep", kind: "warning", okLabel, cancelLabel: "Cancel" });
  },
  onIngestProgress: async (cb: (p: IngestProgress) => void): Promise<() => void> => {
    if (!inTauri) return () => {};
    return tauriListen<IngestProgress>("ingest-progress", (e) => cb(e.payload));
  },
};

// ---------------- Mock backend (browser only) ----------------

const sampleTracks: TrackSummary[] = [
  ["backend", "Backend Engineering", "APIs, frameworks, auth, data and deployment.", "#E8A33D", 1, 112, 24, 5],
  ["ai-eng", "AI Engineering", "LLM apps, RAG, agents, evals and serving.", "#8FB573", 0, 0, 0, 0],
  ["databases", "Databases", "SQL, Postgres, modelling and performance.", "#5B9BD5", 0, 0, 0, 0],
  ["dsa", "Data Structures & Algorithms", "Patterns that win interviews.", "#D9B44A", 0, 0, 0, 0],
  ["system-design", "System Design", "Scaling, trade-offs and architecture.", "#B07CC6", 0, 0, 0, 0],
  ["networking", "Networking", "From packets to protocols.", "#4FB3A9", 0, 0, 0, 0],
  ["automation", "AI Automation", "Workflows that help businesses scale.", "#E58FA0", 0, 0, 0, 0],
  ["interview", "Interview Prep", "DSA drills, design mocks, behavioural.", "#C9C2B2", 0, 0, 0, 0],
  ["hardware", "Embedded & Edge AI", "Paused for now.", "#6E6A62", 0, 0, 0, 0],
].map(([id, name, blurb, accent, folders, documents, total, done]) => ({
  id: id as string,
  name: name as string,
  blurb: blurb as string,
  accent: accent as string,
  status: id === "hardware" ? "paused" : "active",
  folders: folders as number,
  documents: documents as number,
  lessons_total: total as number,
  lessons_done: done as number,
}));

const lessonTitles: [string, string, string[]][] = [
  ["Foundations", "Python for the web", ["Modules & packages", "Virtual environments and pip", "Working with JSON"]],
  ["Foundations", "HTTP from first principles", ["Requests, responses, status codes", "Designing REST resources"]],
  ["Building APIs", "Flask", ["Routes and views", "Request data and validation", "Blueprints and app factories"]],
  ["Building APIs", "FastAPI", ["Path operations and Pydantic", "Dependency injection"]],
  ["Data", "SQL and Postgres", ["Joins and aggregation", "Indexes and query plans", "Transactions"]],
  ["Data", "ORMs", ["SQLAlchemy models", "Migrations with Alembic"]],
  ["Shipping", "Auth", ["Sessions vs JWT", "OAuth in practice"]],
  ["Shipping", "Deploying", ["Docker basics", "Deploying a Flask API"]],
];

function mockCurriculum(): Curriculum {
  const lessons: Lesson[] = [];
  const phases: Phase[] = [];
  let ord = 0;
  lessonTitles.forEach(([phase, module, titles]) => {
    let p = phases.find((x) => x.title === phase);
    if (!p) {
      p = { title: phase, goal: "", modules: [] };
      phases.push(p);
    }
    p.modules.push({ title: module, tested_out: module === "Python for the web" });
    const pi = phases.indexOf(p);
    titles.forEach((title) => {
      lessons.push({
        id: ord + 1,
        curriculum_id: 1,
        ord,
        phase_idx: pi,
        module_idx: p!.modules.length - 1,
        phase,
        module,
        title,
        objectives: ["Routes and URL rules", "HTTP methods on a route", "Returning JSON", "Status codes", "Blueprints"],
        status: module === "Python for the web" ? "skipped" : ord < 5 ? "done" : ord === 5 ? "in_progress" : "todo",
        topic_idx: ord === 5 ? 1 : ord < 5 ? 4 : 0,
      });
      ord++;
    });
  });
  phases[0].goal = "Close the gaps the placement found, fast.";
  phases[1].goal = "Build real APIs with the two frameworks in your folder.";
  phases[2].goal = "Store data properly and query it well.";
  phases[3].goal = "Secure it and put it on the internet.";
  return {
    id: 1,
    title: "Backend from your own notes",
    summary: "Builds on your Flask projects and fills in HTTP, SQL and deployment.",
    phases,
    lessons,
  };
}

const mockSources: Source[] = [
  { chunk_id: 1, title: "Quickstart — Flask Documentation (3.1.x).pdf", path: "", page: 2, text: "A minimal Flask application looks something like this…", score: 0.82 },
  { chunk_id: 2, title: "flask-rest-api-tech-with-tim/app.py", path: "", page: null, text: "@app.route('/api/items', methods=['GET'])", score: 0.77 },
  { chunk_id: 3, title: "How to Design a REST API - Step by Step Guide.pdf", path: "", page: 5, text: "Use nouns for resources…", score: 0.7 },
];

const mockReply = `A **route** connects a URL to a Python function. Flask reads the decorator and registers the function as the handler for that path [S1].

\`\`\`python
from flask import Flask
app = Flask(__name__)

@app.route("/api/items", methods=["GET"])
def list_items():
    return {"items": []}
\`\`\`

Here is the path a request takes:

\`\`\`mermaid
flowchart LR
  C["Client"] -->|"GET /api/items"| F["Flask app"]
  F --> R["list_items()"]
  R -->|"dict to JSON"| F
  F -->|"200 OK"| C
\`\`\`

You already did this in your Tech With Tim project [S2]. Notice that returning a \`dict\` makes Flask send JSON automatically.

**Check:** what status code does Flask send if a client calls \`POST /api/items\` on the route above?`;

const mockReply2 = `✅ Right: Flask answers **405 Method Not Allowed**, because the route only accepts \`GET\`.

To accept both, list them and branch on \`request.method\`:

\`\`\`python
@app.route("/api/items", methods=["GET", "POST"])
def items():
    if request.method == "POST":
        return {"created": True}, 201
    return {"items": []}
\`\`\`

**Check:** which status code did we return for a successful \`POST\`, and why that one?`;

let mockMessages: Message[] = [
  { id: 1, role: "user", content: "", sources: [], created_at: "", topic_idx: null },
  { id: 2, role: "assistant", content: mockReply, sources: mockSources, created_at: "", topic_idx: 0 },
  { id: 3, role: "user", content: "A 405 Method Not Allowed?", sources: [], created_at: "", topic_idx: null },
  { id: 4, role: "assistant", content: mockReply2, sources: mockSources.slice(0, 2), created_at: "", topic_idx: 1 },
];

async function mock<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  await new Promise((r) => setTimeout(r, 120));
  if (cmd === "add_session") {
    const n = args.session as NewSession;
    mockSessions = [...mockSessions, { ...n, id: mockSessions.length + 10, lesson_title: null }];
    return mockSessions.length as T;
  }
  if (cmd === "delete_session") {
    mockSessions = mockSessions.filter((x) => x.id !== args.id);
    return null as T;
  }
  if (cmd === "get_agenda") return mockAgenda(String(args.from), String(args.to)) as T;
  if (cmd === "rate_card") {
    mockCards = mockCards.filter((c) => c.id !== args.cardId);
    return new Date().toISOString() as T;
  }
  if (cmd === "new_exercise") {
    const ex = { ...mockExercise, id: mockExercises.length + 1, title: "Count words" };
    mockExercises = [...mockExercises, ex];
    return ex as T;
  }
  const curriculum = mockCurriculum();
  const out: Record<string, unknown> = {
    list_tracks: sampleTracks,
    get_settings: {
      live: false,
      has_anthropic: false,
      has_gemini: false,
      has_groq: false,
      env_path: "C:\\Users\\you\\Desktop\\programming\\CPrep\\.env",
      test_provider: "local",
      tutor_model: "claude-haiku-5-5",
      curriculum_model: "claude-sonnet-5-5",
      escalate_model: "claude-opus-5-5",
      ollama_chat_model: "qwen3:4b",
      ollama_embed_model: "nomic-embed-text",
      ollama_up: true,
      notes_dir: "C:\\Users\\you\\Documents\\CPrep Notes",
      vision_model: "groq · qwen/qwen3.8-27b",
      ollama_models: ["gemma2:9b", "nomic-embed-text:latest"],
      spend_usd: 0,
    } satisfies Settings,
    get_track:
      args.trackId === "backend"
        ? ({
            folders: [{ id: 1, path: "C:\\Users\\you\\Desktop\\programming\\BACKEND JOURNEY", last_ingested: new Date().toISOString(), documents: 76, chunks: 1240 }],
            known: ["Python", "Flask"],
            placement: {
              correct: 6,
              total: 8,
              topics: [
                { topic: "Python basics", correct: 3, total: 3 },
                { topic: "Flask routing", correct: 2, total: 2 },
                { topic: "HTTP methods", correct: 1, total: 2 },
                { topic: "SQL joins", correct: 0, total: 1 },
              ],
              strong: ["Python basics", "Flask routing"],
              weak: ["SQL joins"],
              review: [],
            },
            curriculum,
          } satisfies TrackDetail)
        : ({ folders: [], known: [], placement: null, curriculum: null } satisfies TrackDetail),
    get_lesson: (() => {
      const l = curriculum.lessons.find((x) => x.id === args.lessonId) ?? curriculum.lessons[5];
      return { lesson: l, messages: mockMessages, prev_id: l.id > 1 ? l.id - 1 : null, next_id: l.id + 1 } satisfies LessonView;
    })(),
    start_placement: [
      { topic: "Flask routing", prompt: "What does `@app.route('/users/<int:id>')` pass to the view function?", options: ["The string \"id\"", "An integer named id", "The whole request", "Nothing; you read it from request.args"] },
      { topic: "HTTP methods", prompt: "Which method should create a new resource in a REST API?", options: ["GET", "POST", "PUT", "DELETE"] },
    ] satisfies Question[],
    list_exercises: mockExercises,
    run_code: {
      stdout: "[4, 16, 36]\n",
      stderr: "",
      exit_code: 0,
      timed_out: false,
      millis: 41,
    } satisfies RunOutput,
    write_notes: "C:\\Users\\you\\Documents\\CPrep Notes\\Backend Engineering\\06 Routes and views.md",
    notes_path: null,
    list_sessions: mockSessions,
    get_reminders: { offsets: [30, 15, 0], review_time: "19:00", keep_in_tray: true, autostart: false } satisfies ReminderSettings,
    save_reminders: null,
    test_notification: null,
    review_summary: {
      due_now: mockCards.length,
      total: 23,
      new_cards: 5,
      reviewed_today: 4,
      next_due: new Date(Date.now() + 3 * 3600_000).toISOString(),
      by_track: [["backend", mockCards.length]],
    } satisfies ReviewSummary,
    due_cards: mockCards,
    make_cards: 6,
    lessons_for_cards: [
      { lesson_id: 1, title: "Routes and URL rules", track_id: "backend", cards: 6, status: "done" },
      { lesson_id: 6, title: "Routes and views", track_id: "backend", cards: 0, status: "in_progress" },
    ] satisfies LessonCards[],
    get_canvas: { scene: null, task: null, critique: null } satisfies CanvasState,
    save_canvas: null,
    canvas_task:
      "**Draw the path of one request** through a Flask API that reads from a database.\n\n- The client, the Flask app and the database as separate boxes\n- Arrows for the request and the response, labelled with the HTTP method and status code\n- Where the route function runs\n- What happens when the item doesn't exist (404)",
    submit_placement: { correct: 1, total: 2, topics: [], strong: [], weak: [], review: [] },
  };
  if (cmd === "retake_lesson") mockMessages = [];
  return (out[cmd] ?? null) as T;
}

const mockExercise: Exercise = {
  id: 1,
  lesson_id: 6,
  topic_idx: 1,
  title: "Even squares",
  prompt:
    "Write `even_squares(nums)` that returns the squares of the **even** numbers in `nums`, in their original order. Use a list comprehension.\n\nExample: `even_squares([1, 2, 3, 4])` returns `[4, 16]`.",
  language: "python",
  kind: "function",
  function_name: "even_squares",
  starter_code: "def even_squares(nums):\n    # TODO: use a list comprehension\n    pass\n",
  setup_sql: "",
  draft: null,
  examples: [
    { args: [[1, 2, 3, 4]], expected: [4, 16], stdin: "", expected_output: "", hidden: false },
    { args: [[]], expected: [], stdin: "", expected_output: "", hidden: false },
    { args: [[-2, 0, 3]], expected: [4, 0], stdin: "", expected_output: "", hidden: false },
  ],
  hidden_count: 2,
  best: null,
  attempts: 0,
};
let mockExercises: Exercise[] = [mockExercise];

async function mockSubmit(code: string, onEvent: (e: LabEvent) => void) {
  const right = /%\s*2\s*==\s*0/.test(code);
  onEvent({
    type: "graded",
    grade: {
      passed: right ? 5 : 1,
      total: 5,
      error: null,
      stdout: "",
      results: [
        { passed: right, hidden: false, input: "[1, 2, 3, 4]", expected: "[4,16]", got: right ? "[4,16]" : "[1,4,9,16]", error: null },
        { passed: true, hidden: false, input: "[]", expected: "[]", got: "[]", error: null },
        { passed: right, hidden: false, input: "[-2, 0, 3]", expected: "[4,0]", got: right ? "[4,0]" : "[4,0,9]", error: null },
        { passed: right, hidden: true, input: null, expected: null, got: null, error: null },
        { passed: right, hidden: true, input: null, expected: null, got: null, error: null },
      ],
    },
  });
  const review = right
    ? "**All tests pass.** Clean, single comprehension. One improvement: name the loop variable `n` to match `nums`."
    : "**Not yet:** odd numbers are getting squared too.\n\nYour comprehension has no filter, so every `x` in `nums` is kept. A comprehension can end with an `if` clause that decides which items stay. How do you test whether a number is even?";
  for (const w of review.split(/(\s+)/)) {
    await new Promise((r) => setTimeout(r, 6));
    onEvent({ type: "delta", text: w });
  }
  onEvent({ type: "done", attemptId: Date.now(), provider: "Mock · browser", costUsd: 0 });
}

async function mockCritique(description: string, onEvent: (e: LabEvent) => void) {
  const text = description.includes("→")
    ? "**What works**\n- Client, app and database are separate, and the request flows left to right.\n\n**Missing or wrong**\n- The response arrow back to the client has no status code.\n- Nothing shows the 404 path when the item doesn't exist.\n\n**Next step:** add a dashed arrow from the app back to the client labelled `404 Not Found`."
    : "**The canvas is nearly empty.** Start with three boxes: the client, your Flask app and the database, then connect them with arrows for the request.";
  for (const w of text.split(/(\s+)/)) {
    await new Promise((r) => setTimeout(r, 6));
    onEvent({ type: "delta", text: w });
  }
  onEvent({ type: "done", attemptId: 0, provider: "Mock · browser", costUsd: 0 });
}

const pad = (n: number) => String(n).padStart(2, "0");
const localStamp = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
const today = new Date();
let mockSessions: Session[] = [
  { id: 1, track_id: "backend", lesson_id: 6, lesson_title: "Routes and views", title: "Backend", starts_at: localStamp(new Date(today.getFullYear(), today.getMonth(), today.getDate(), 19, 0)), duration_min: 60, repeat: "weekdays" },
  { id: 2, track_id: "networking", lesson_id: null, lesson_title: null, title: "Subnetting practice", starts_at: localStamp(new Date(today.getFullYear(), today.getMonth(), today.getDate() + 2, 7, 30)), duration_min: 45, repeat: "weekly" },
  { id: 3, track_id: "dsa", lesson_id: null, lesson_title: null, title: "Two pointers drills", starts_at: localStamp(new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1, 21, 0)), duration_min: 30, repeat: "none" },
];

function mockAgenda(from: string, to: string): Occurrence[] {
  const out: Occurrence[] = [];
  const a = new Date(from), b = new Date(to);
  for (const s of mockSessions) {
    const first = new Date(s.starts_at);
    for (let d = new Date(a); d < b; d.setDate(d.getDate() + 1)) {
      const at = new Date(d.getFullYear(), d.getMonth(), d.getDate(), first.getHours(), first.getMinutes());
      if (at < first || at < a || at >= b) continue;
      const wd = at.getDay();
      const ok = s.repeat === "daily" || (s.repeat === "weekdays" && wd > 0 && wd < 6) || (s.repeat === "weekly" && wd === first.getDay()) || (s.repeat === "none" && at.getTime() === first.getTime());
      if (ok) out.push({ session: s, start: at.toISOString(), end: new Date(at.getTime() + s.duration_min * 60000).toISOString() });
    }
  }
  return out.sort((x, y) => x.start.localeCompare(y.start));
}

let mockCards: Card[] = [
  { id: 1, track_id: "backend", lesson_id: 1, lesson_title: "Routes and URL rules", front: "What does Flask return when a route only allows `GET` and the client sends `POST`?", back: "`405 Method Not Allowed`. List both methods in `methods=[...]` and branch on `request.method` to accept both.", due: today.toISOString(), memory: { stability: 3.7, difficulty: 5, reps: 1, lapses: 0 }, last_review: null, preview: ["10m", "2d", "4d", "14d"] },
  { id: 2, track_id: "backend", lesson_id: 1, lesson_title: "Routes and URL rules", front: "Why does returning a `dict` from a Flask view produce JSON?", back: "Flask runs `jsonify` on dict and list return values, which serialises them and sets `Content-Type: application/json`.", due: today.toISOString(), memory: { stability: 0, difficulty: 0, reps: 0, lapses: 0 }, last_review: null, preview: ["10m", "1d", "4d", "14d"] },
];

async function mockChat(text: string, onEvent: (e: ChatEvent) => void) {
  onEvent({ type: "sources", sources: mockSources });
  const words = ("[[topic:2]]\n" + mockReply2).split(/(\s+)/);
  for (const w of words) {
    await new Promise((r) => setTimeout(r, 8));
    onEvent({ type: "delta", text: w });
  }
  mockMessages = [
    ...mockMessages,
    { id: Date.now(), role: "user", content: text, sources: [], created_at: "", topic_idx: null },
    { id: Date.now() + 1, role: "assistant", content: mockReply2, sources: mockSources, created_at: "", topic_idx: 1 },
  ];
  onEvent({ type: "done", messageId: Date.now(), provider: "Mock · browser", costUsd: 0, topicIdx: 1 });
}
